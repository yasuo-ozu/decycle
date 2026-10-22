//! With `module = path`, the ranked engine reports an opt-out bound on a cycle (`crate::m::C: Loop`
//! where `C`'s impl leads back) as a decycle error, before rustc overflows on it
//! (`ui/module_absolute_bound*.rs`). It reports only what is certain to overflow: these modules
//! name own types by a path too, and must keep compiling and running.
//!
//! A body calls through an opt-out by its trait's path (`<crate::m::C as crate::m::Loop>::step`):
//! by method syntax, the call does not compile in the engine's ranked copy of the body (`E0283`).

#![allow(dead_code)]

// ---- an opt-out naming an own type that is on no cycle ----

#[decycle::decycle(module = crate::off_cycle)]
pub mod off_cycle {
    #[decycle]
    pub trait Loop {
        fn step(&self, n: u32) -> u32;
    }

    pub struct A;
    pub struct B;
    pub struct Leaf;

    impl Loop for A
    where
        B: Loop,
        crate::off_cycle::Leaf: Loop,
    {
        fn step(&self, n: u32) -> u32 {
            if n == 0 {
                <crate::off_cycle::Leaf as crate::off_cycle::Loop>::step(&Leaf, 0)
            } else {
                B.step(n - 1) + 1
            }
        }
    }

    impl Loop for B
    where
        A: Loop,
    {
        fn step(&self, n: u32) -> u32 {
            A.step(n)
        }
    }

    // `Leaf`'s impl does not lead back to `A`.
    impl Loop for Leaf {
        fn step(&self, n: u32) -> u32 {
            n + 100
        }
    }
}

#[test]
fn an_opt_out_off_every_cycle_compiles() {
    use off_cycle::Loop;
    assert_eq!(off_cycle::A.step(3), 103);
}

// ---- an opt-out whose chain runs through a generic impl, which the check does not judge ----

#[decycle::decycle(module = crate::through_generic)]
pub mod through_generic {
    #[decycle]
    pub trait Loop {
        fn step(&self, n: u32) -> u32;
    }

    pub struct A;
    pub struct B;
    pub struct C;
    pub struct Wrap<T>(pub T);

    impl Loop for A
    where
        B: Loop,
        crate::through_generic::C: Loop,
    {
        fn step(&self, n: u32) -> u32 {
            if n == 0 {
                <crate::through_generic::C as crate::through_generic::Loop>::step(&C, 0)
            } else {
                B.step(n - 1) + 1
            }
        }
    }

    impl Loop for B
    where
        A: Loop,
    {
        fn step(&self, n: u32) -> u32 {
            A.step(n)
        }
    }

    // Whether `Wrap<T>`'s impl leads back depends on `T`: here it does not.
    impl Loop for C
    where
        Wrap<u8>: Loop,
    {
        fn step(&self, n: u32) -> u32 {
            Wrap(1u8).step(n)
        }
    }

    impl<T> Loop for Wrap<T>
    where
        T: Loop,
    {
        fn step(&self, n: u32) -> u32 {
            self.0.step(n) + 1
        }
    }

    impl Loop for u8 {
        fn step(&self, n: u32) -> u32 {
            n + u32::from(*self)
        }
    }
}

// Two impls for one type, of which only the one the chain does not reach leads back: a reading of
// the impls by their type's name would see `C` → `Wrap` → `A` → `C`.
#[decycle::decycle(module = crate::by_instance)]
pub mod by_instance {
    #[decycle]
    pub trait Loop {
        fn step(&self, n: u32) -> u32;
    }

    pub struct A;
    pub struct C;
    pub struct Wrap<T>(pub T);

    impl Loop for A
    where
        crate::by_instance::C: Loop,
    {
        fn step(&self, n: u32) -> u32 {
            <crate::by_instance::C as crate::by_instance::Loop>::step(&C, n) + 1
        }
    }

    impl Loop for C
    where
        Wrap<i32>: Loop,
    {
        fn step(&self, n: u32) -> u32 {
            Wrap(0i32).step(n) + 2
        }
    }

    impl Loop for Wrap<u8>
    where
        A: Loop,
    {
        fn step(&self, n: u32) -> u32 {
            n + 3
        }
    }

    impl Loop for Wrap<i32> {
        fn step(&self, n: u32) -> u32 {
            n + 4
        }
    }
}

#[test]
fn an_opt_out_through_a_generic_impl_is_not_judged() {
    {
        use through_generic::Loop;
        assert_eq!(through_generic::A.step(3), 5);
    }
    {
        use by_instance::Loop;
        assert_eq!(by_instance::A.step(1), 8);
        assert_eq!(by_instance::Wrap(1u8).step(1), 4);
    }
}

// ---- without `module`, nothing is reported ----

#[decycle::decycle]
pub mod no_module {
    #[decycle]
    pub trait Loop {
        fn step(&self, n: u32) -> u32;
    }

    pub struct A;
    pub struct B;
    pub struct Leaf;

    impl Loop for A
    where
        B: Loop,
        crate::no_module::Leaf: Loop,
    {
        fn step(&self, n: u32) -> u32 {
            if n == 0 {
                <crate::no_module::Leaf as crate::no_module::Loop>::step(&Leaf, 0)
            } else {
                B.step(n - 1) + 1
            }
        }
    }

    impl Loop for B
    where
        A: Loop,
    {
        fn step(&self, n: u32) -> u32 {
            A.step(n)
        }
    }

    impl Loop for Leaf {
        fn step(&self, n: u32) -> u32 {
            n + 100
        }
    }
}

#[test]
fn without_module_an_opt_out_is_as_before() {
    use no_module::Loop;
    assert_eq!(no_module::A.step(3), 103);
}

// ---- a path to another module's type of the same name ----

pub mod other {
    pub struct C;

    impl crate::same_name::Loop for C {
        fn step(&self, n: u32) -> u32 {
            n + 1000
        }
    }
}

#[decycle::decycle(module = crate::same_name)]
pub mod same_name {
    #[decycle]
    pub trait Loop {
        fn step(&self, n: u32) -> u32;
    }

    pub struct A;
    pub struct C;

    // `crate::other::C` is not this module's `C`, whose impl does lead back to `A`.
    impl Loop for A
    where
        C: Loop,
        crate::other::C: Loop,
    {
        fn step(&self, n: u32) -> u32 {
            if n == 0 {
                <crate::other::C as crate::same_name::Loop>::step(&crate::other::C, 0)
            } else {
                C.step(n - 1) + 1
            }
        }
    }

    impl Loop for C
    where
        A: Loop,
    {
        fn step(&self, n: u32) -> u32 {
            A.step(n)
        }
    }
}

#[test]
fn another_module_s_type_of_the_same_name_is_not_this_module_s() {
    use same_name::Loop;
    assert_eq!(same_name::A.step(3), 1003);
}

// ---- a chain longer than `recurse_level` reaches the rank floor before it leads back ----

// `C` enters at rank 2, `D` is at 1, and `E` at the floor, which has no cycle edge to `A`: rustc
// never comes back to `A`'s premise, so this compiles, and the check reports nothing. (A call that
// reaches the floor on this chain, here `A.step(3)`, fails closed at run time, "re-entry fn not
// registered": a limit of the engine with an opt-out on the cycle, which the check does not change.)
#[decycle::decycle(module = crate::past_the_floor, recurse_level = 2)]
pub mod past_the_floor {
    #[decycle]
    pub trait Loop {
        fn step(&self, n: u32) -> u32;
    }

    pub struct A;
    pub struct C;
    pub struct D;
    pub struct E;

    impl Loop for A
    where
        crate::past_the_floor::C: Loop,
    {
        fn step(&self, n: u32) -> u32 {
            if n == 0 {
                0
            } else {
                <crate::past_the_floor::C as crate::past_the_floor::Loop>::step(&C, n - 1) + 1
            }
        }
    }

    impl Loop for C
    where
        D: Loop,
    {
        fn step(&self, n: u32) -> u32 {
            if n == 0 {
                0
            } else {
                D.step(n - 1) + 1
            }
        }
    }

    impl Loop for D
    where
        E: Loop,
    {
        fn step(&self, n: u32) -> u32 {
            if n == 0 {
                0
            } else {
                E.step(n - 1) + 1
            }
        }
    }

    impl Loop for E
    where
        A: Loop,
    {
        fn step(&self, n: u32) -> u32 {
            if n == 0 {
                0
            } else {
                A.step(n - 1) + 1
            }
        }
    }
}

#[test]
fn a_chain_past_the_rank_floor_compiles() {
    use past_the_floor::Loop;
    assert_eq!(past_the_floor::A.step(2), 2);
}
