//! `module = path` names the module absolutely: a relative path (`m`, `self::m`, `super::m`) means
//! something else at each place the path is written, and announce rejects it anyway. The module is
//! still expanded (without the path), so the error stays alone.
#[decycle::decycle(module = self::m)]
mod m {
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
            if n == 0 { 0 } else { B.step(n - 1) + 1 }
        }
    }

    impl Loop for B
    where
        A: Loop,
    {
        fn step(&self, n: u32) -> u32 {
            if n == 0 { 0 } else { A.step(n - 1) + 1 }
        }
    }
}

fn main() {}
