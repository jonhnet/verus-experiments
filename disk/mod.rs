use builtin::*;
use vstd::prelude::*;
use vstd::modes::*;
use vstd::invariant::*;

pub mod frac;

use frac::FractionalResource;

verus! {
    pub const DISK_INV_NS: u64 = 12345;

    pub type DiskView = (u8, u8);

    pub open spec fn view_write(state: DiskView, addr: u8, val: u8) -> DiskView
    {
        if addr == 0 {
            (val, state.1)
        } else {
            (state.0, val)
        }
    }

    pub open spec fn view_read(state: DiskView, addr: u8) -> u8
    {
        if addr == 0 {
            state.0
        } else {
            state.1
        }
    }

    pub struct MemCrashView
    {
        pub mem: DiskView,
        pub crash: DiskView,
    }

    pub trait DiskWritePermission<ResultT> {
        spec fn addr(&self) -> u8;
        spec fn val(&self) -> u8;
        spec fn id(&self) -> int;
        spec fn pre(&self) -> bool;
        spec fn post(&self, r: ResultT) -> bool;
        proof fn apply(tracked self, tracked r: FractionalResource<MemCrashView, 2>, write_crash: bool, tracked credit: OpenInvariantCredit) -> (tracked result: (FractionalResource<MemCrashView, 2>, ResultT))
            where
                Self: Sized
            requires
                self.pre(),
                r.valid(self.id(), 1),
            ensures
                result.0.valid(self.id(), 1),
                result.0.val() == (MemCrashView{
                    mem: view_write(r.val().mem, self.addr(), self.val()),
                    crash: if write_crash { view_write(r.val().crash, self.addr(), self.val()) } else { r.val().crash },
                }),
                self.post(result.1),
            opens_invariants
                [ DISK_INV_NS ];
    }

    pub trait DiskBarrierPermission<ResultT> {
        spec fn id(&self) -> int;
        spec fn pre(&self) -> bool;
        spec fn post(&self, r: ResultT) -> bool;
        proof fn apply(tracked self, tracked r: FractionalResource<MemCrashView, 2>, tracked credit: OpenInvariantCredit) -> (tracked result: (FractionalResource<MemCrashView, 2>, ResultT))
            where
                Self: Sized
            requires
                self.pre(),
                r.valid(self.id(), 1),
                r.val().mem == r.val().crash,
            ensures
                result.0.valid(self.id(), 1),
                result.0.val() == r.val(),
                self.post(result.1),
            opens_invariants
                [ DISK_INV_NS ];
    }

    pub struct Disk
    {
        block0: u8,
        block1: u8,
        ghost durable: DiskView,    // Prophecy-style crash state
        tracked frac: Option<FractionalResource<MemCrashView, 2>>,
    }

    impl Disk
    {
        pub closed spec fn inv(&self) -> bool
        {
            self.frac.is_Some() &&
            self.frac.unwrap().inv() &&
            self.frac.unwrap().val().crash == self.durable &&
            self.frac.unwrap().val().mem == (self.block0, self.block1) &&
            self.frac.unwrap().frac() == 1
        }

        pub closed spec fn id(&self) -> int
        {
            self.frac.unwrap().id()
        }

        pub fn alloc() -> (res: (Disk, Tracked<FractionalResource::<MemCrashView, 2>>))
            requires
                true,
            ensures
                res.0.inv(),
                res.1@.valid(res.0.id(), 1),
                res.1@.val() == (MemCrashView{
                    mem: (0, 0),
                    crash: (0, 0),
                }),
        {
            let tracked r = FractionalResource::<MemCrashView, 2>::alloc(MemCrashView{
                mem: (0, 0),
                crash: (0, 0),
            });
            let tracked (r1, r2) = r.split(1);
            let mut d = Disk{
                block0: 0,
                block1: 0,
                durable: (0, 0),
                // XXX why can't this directly assign Some(r1)?
                frac: None,
            };
            proof {
                d.frac = Some(r1)
            };
            (d, Tracked(r2))
        }

        pub fn read(&self, addr: u8, Tracked(f): Tracked<&mut FractionalResource<MemCrashView, 2>>) -> (result: u8)
            requires
                self.inv(),
                old(f).inv(),
                old(f).id() == self.id(),
            ensures
                result == view_read(f.val().mem, addr),
                f == old(f),
        {
            proof {
                match &self.frac {
                    None => (),
                    Some(ref self_f) => f.agree(self_f),
                }
            };
            if addr == 0 {
                self.block0
            } else {
                self.block1
            }
        }

        pub fn write<ResultT, Perm>(&mut self, addr: u8, val: u8, Tracked(perm): Tracked<Perm>) -> (result: Tracked<ResultT>)
            where
                Perm: DiskWritePermission<ResultT>
            requires
                old(self).inv(),
                perm.pre(),
                perm.addr() == addr,
                perm.val() == val,
                perm.id() == old(self).id(),
            ensures
                self.inv(),
                self.id() == old(self).id(),
                perm.post(result@),
        {
            if addr == 0 {
                self.block0 = val
            } else {
                self.block1 = val
            };
            let credit = create_open_invariant_credit();
            let tracked mut result: Option<ResultT> = None;
            proof {
                let tracked mut opt_frac = None;
                tracked_swap(&mut self.frac, &mut opt_frac);
                let tracked frac = opt_frac.tracked_unwrap();
                let write_crash = vstd::pervasive::arbitrary();
                if write_crash {
                    self.durable = view_write(self.durable, addr, val)
                };
                let tracked (r, res) = perm.apply(frac, write_crash, credit.get());
                self.frac = Some(r);
                result = Some(res)
            };
            Tracked(result.tracked_unwrap())
        }

        // Leftover, should really be implemented in terms of the fupd-style barrier() below
        #[verifier(external_body)]
        pub fn barrier_owned(&self, Tracked(f): Tracked<&FractionalResource::<MemCrashView, 2>>)
            requires
                self.inv(),
                f.valid(self.id(), 1),
            ensures
                f.val().crash == f.val().mem,
        {
            unimplemented!()
        }

        pub fn barrier<ResultT, Perm>(&mut self, Tracked(perm): Tracked<Perm>) -> (result: Tracked<ResultT>)
            where
                Perm: DiskBarrierPermission<ResultT>
            requires
                old(self).inv(),
                perm.pre(),
                perm.id() == old(self).id(),
            ensures
                self.inv(),
                self.id() == old(self).id(),
                perm.post(result@),
        {
            assert(self.durable == (self.block0, self.block1)) by { admit() };

            let credit = create_open_invariant_credit();
            let tracked mut result: Option<ResultT> = None;
            proof {
                let tracked mut opt_frac = None;
                tracked_swap(&mut self.frac, &mut opt_frac);
                let tracked frac = opt_frac.tracked_unwrap();
                let tracked (r, res) = perm.apply(frac, credit.get());
                self.frac = Some(r);
                result = Some(res)
            };
            Tracked(result.tracked_unwrap())
        }
    }

    fn main()
    {
        let (d, Tracked(r)) = Disk::alloc();
        let mut d = d;

        let x0 = d.read(0, Tracked(&mut r));
        let x1 = d.read(1, Tracked(&mut r));
        assert(x0 == 0 && x1 == 0);

        ()
    }
}