//! A `macro_rules!` definition inside a `#[decycle]` module is transparent to both engines: it is
//! re-emitted in place, and its textual scope still reaches the (re-emitted) cyclic impl bodies —
//! including a macro whose body names a cycle type and calls a cyclic method.
//! Only a macro *definition* passes; an item-position macro call is still rejected
//! (`tests/ui/ranked_module_macro.rs`).

use decycle::decycle;

#[decycle]
mod ranked {
    macro_rules! dec {
        ($n:expr) => {
            $n - 1
        };
    }

    /// Names a cycle head and a cyclic method from inside the macro body.
    macro_rules! hop_b {
        ($n:expr) => {
            B.step(dec!($n)) + 1
        };
    }

    #[decycle]
    pub trait Loop {
        fn step(&self, n: u32) -> u32;
    }

    pub struct A;
    pub struct B;

    impl Loop for A
    where
        B: Loop,
    {
        fn step(&self, n: u32) -> u32 {
            if n == 0 {
                0
            } else {
                hop_b!(n)
            }
        }
    }

    impl Loop for B
    where
        A: Loop,
    {
        fn step(&self, n: u32) -> u32 {
            if n == 0 {
                0
            } else {
                A.step(dec!(n)) + 1
            }
        }
    }

    pub fn run(n: u32) -> u32 {
        dec!(n + 1) + A.step(0)
    }
}

#[decycle(structural)]
mod structural {
    macro_rules! dec {
        ($n:expr) => {
            $n - 1
        };
    }

    #[decycle]
    pub trait Loop {
        fn step(&self, n: u32) -> u32;
    }

    pub struct A(pub Option<Box<B>>);
    pub struct B(pub Option<Box<A>>);

    impl Loop for A
    where
        B: Loop,
    {
        fn step(&self, n: u32) -> u32 {
            match &self.0 {
                Some(b) if n > 0 => b.step(dec!(n)) + 1,
                _ => 0,
            }
        }
    }

    impl Loop for B
    where
        A: Loop,
    {
        fn step(&self, n: u32) -> u32 {
            match &self.0 {
                Some(a) if n > 0 => a.step(dec!(n)) + 1,
                _ => 0,
            }
        }
    }
}

#[test]
fn ranked_macro_rules_is_transparent() {
    use ranked::Loop;
    assert_eq!(ranked::A.step(10), 10);
    assert_eq!(ranked::B.step(7), 7);
    assert_eq!(ranked::run(5), 5);
}

#[test]
fn structural_macro_rules_is_transparent() {
    use structural::{Loop, A, B};
    let chain = A(Some(Box::new(B(Some(Box::new(A(None)))))));
    assert_eq!(chain.step(5), 2);
}
