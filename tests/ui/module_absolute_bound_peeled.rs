//! The part of a wrapped bound that names an own type by its path is the opt-out too:
//! `(B, crate::m::C): Loop` is the edge `B: Loop` and the premise `crate::m::C: Loop`, as written.
//! `C`'s impl leads back to `A`, so the premise overflows; the error is at the part, and alone (the
//! fallback emitted after it drops the bounds that would overflow on their own).
#[decycle::decycle(module = crate::m)]
pub mod m {
    #[decycle]
    pub trait Loop { fn step(&self, n: u32) -> u32; }
    pub struct A; pub struct B; pub struct C;
    impl Loop for A where (B, crate::m::C): Loop {
        fn step(&self, n: u32) -> u32 { if n == 0 { 0 } else { B.step(n - 1) + C.step(n - 1) + 1 } }
    }
    impl Loop for B where A: Loop { fn step(&self, n: u32) -> u32 { A.step(n) } }
    impl Loop for C where A: Loop { fn step(&self, n: u32) -> u32 { A.step(n) } }
}

fn main() {
    use m::Loop;
    assert_eq!(m::A.step(3), 7);
}
