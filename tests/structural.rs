//! Equivalence tests: each scenario is instantiated under BOTH algorithms — the existing ranked engine
//! (`#[decycle]`) and the structural unroll (`#[decycle(structural)]`) — and the same assertions are
//! run against both, proving the structural algorithm is a behavioural drop-in for the common case
//! (method-based recursion, no growing type arguments).

/// Emit the module body twice (ranked + structural) and run `$body` against each.
macro_rules! both {
    ($name:ident { $($items:tt)* } run $body:block) => {
        mod $name {
            #[decycle::decycle]
            pub mod ranked {
                $($items)*
            }
            #[decycle::decycle(structural)]
            pub mod structural {
                $($items)*
            }
        }
        #[test]
        fn $name() {
            {
                use $name::ranked::*;
                $body
            }
            {
                use $name::structural::*;
                $body
            }
        }
    };
}

// ---- self-cycle ----
both! {
    self_cycle {
        #[decycle]
        pub trait Eval: Sized {
            fn eval(&self) -> i64;
        }
        pub enum Expr {
            Lit(i64),
            Node(i64, Option<Box<Expr>>),
        }
        impl Eval for Expr
        where
            Expr: Eval,
        {
            fn eval(&self) -> i64 {
                match self {
                    Expr::Lit(n) => *n,
                    Expr::Node(n, c) => *n + c.as_ref().map_or(0, |x| x.eval()),
                }
            }
        }
    }
    run {
        let e = Expr::Node(1, Some(Box::new(Expr::Node(2, Some(Box::new(Expr::Lit(3)))))));
        assert_eq!(e.eval(), 6);
    }
}

// ---- two-type cross-edge cycle ----
both! {
    cross_edge {
        #[decycle]
        pub trait Sum: Sized {
            fn sum(&self) -> i64;
        }
        pub struct A {
            pub v: i64,
            pub b: Option<Box<B>>,
        }
        pub struct B {
            pub v: i64,
            pub a: Option<Box<A>>,
        }
        impl Sum for A
        where
            B: Sum,
        {
            fn sum(&self) -> i64 {
                self.v + self.b.as_ref().map_or(0, |b| b.sum())
            }
        }
        impl Sum for B
        where
            A: Sum,
        {
            fn sum(&self) -> i64 {
                self.v + self.a.as_ref().map_or(0, |a| a.sum())
            }
        }
    }
    run {
        // A(1) -> B(2) -> A(3) = 6
        let a = A {
            v: 1,
            b: Some(Box::new(B {
                v: 2,
                a: Some(Box::new(A { v: 3, b: None })),
            })),
        };
        assert_eq!(a.sum(), 6);
    }
}

// ---- deeper chain (both algorithms handle unbounded depth) ----
both! {
    deep_chain {
        #[decycle]
        pub trait Depth: Sized {
            fn depth(&self) -> usize;
        }
        pub struct Cons(pub Option<Box<Cons>>);
        impl Depth for Cons
        where
            Cons: Depth,
        {
            fn depth(&self) -> usize {
                1 + self.0.as_ref().map_or(0, |n| n.depth())
            }
        }
    }
    run {
        let mut c = Cons(None);
        for _ in 0..2000 {
            c = Cons(Some(Box::new(c)));
        }
        assert_eq!(c.depth(), 2001);
    }
}
