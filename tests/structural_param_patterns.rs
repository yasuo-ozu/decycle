//! Regression: destructured (non-identifier) parameter patterns in a cyclic method — a tuple pattern
//! `(x, y): (usize, usize)` and a struct pattern. The structural codegen renames such params to fresh
//! idents (so they can be forwarded) and rebinds the original pattern at the top of the body. Run
//! under BOTH algorithms.
mod common;

dual_mod! {
    pat {
        pub struct Point {
            pub x: usize,
            pub y: usize,
        }

        #[decycle]
        pub trait Walk {
            fn walk(&self, pair: (usize, usize), p: Point, n: usize) -> usize;
        }

        pub struct A;
        pub struct B;

        impl Walk for A
        where
            B: Walk,
        {
            fn walk(&self, (x, y): (usize, usize), Point { x: px, y: py }: Point, n: usize) -> usize {
                if n == 0 {
                    x + y + px + py
                } else {
                    B.walk((x, y), Point { x: px, y: py }, n - 1) + 1
                }
            }
        }
        impl Walk for B
        where
            A: Walk,
        {
            fn walk(&self, (x, y): (usize, usize), Point { x: px, y: py }: Point, n: usize) -> usize {
                if n == 0 {
                    x + y + px + py
                } else {
                    A.walk((x, y), Point { x: px, y: py }, n - 1) + 1
                }
            }
        }
    }
}

#[test]
fn destructured_params_forward_across_cycle() {
    on_both!(pat, {
        // floor = 1+2+3+4 = 10, each hop adds 1 → 10 + 50
        assert_eq!(A.walk((1, 2), Point { x: 3, y: 4 }, 50), 60);
    });
}
