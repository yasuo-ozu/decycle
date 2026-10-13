use std::fmt::Debug;
use std::hash::Hash;

// Local trait that exists only in this crate
#[decycle::decycle]
pub trait LocalTrait {
    fn local_method(&self) -> usize;
}

pub trait TestTrait {
    fn test_method(&self) -> String;
}

pub trait CircularTrait {
    fn circular_method(&self) -> Box<dyn CircularTrait>;
}

pub trait ExtendedTrait {
    fn extended_method(&self) -> bool;
}

// Implementations for arrays in LocalTrait
impl<T, const N: usize> LocalTrait for [T; N]
where
    T: Clone + Default,
{
    fn local_method(&self) -> usize {
        let _ = T::default();
        self.len()
    }
}

// Implementations for slices in LocalTrait
impl<T> LocalTrait for [T]
where
    T: PartialEq + Clone,
{
    fn local_method(&self) -> usize {
        if !self.is_empty() {
            let _ = self[0].clone();
        }
        self.len()
    }
}

// Implementations for tuples in LocalTrait
impl<T, U> LocalTrait for (T, U)
where
    T: Clone + Send,
    U: Sync + Default,
{
    fn local_method(&self) -> usize {
        let _ = self.0.clone();
        let _ = U::default();
        2
    }
}

impl<T, U, V> LocalTrait for (T, U, V)
where
    T: Clone + Send,
    U: Sync,
    V: Default + Send,
{
    fn local_method(&self) -> usize {
        let _ = self.0.clone();
        let _ = V::default();
        3
    }
}

// Implementations for tuples in TestTrait
impl<T, U> TestTrait for (T, U)
where
    T: Clone + Send,
    U: Debug + Default,
{
    fn test_method(&self) -> String {
        let _cloned = self.0.clone();
        let default_u = U::default();
        format!("tuple(2): {:?}", default_u)
    }
}

impl<T, U, V> TestTrait for (T, U, V)
where
    T: Send + Sync,
    U: Clone,
    V: Debug + Hash,
{
    fn test_method(&self) -> String {
        let _ = self.1.clone();
        format!("tuple(3): {:?}", self.2)
    }
}

// Implementations for arrays in CircularTrait
impl<T, const N: usize> CircularTrait for [T; N]
where
    T: Clone + Send + Sync + Default + 'static,
{
    fn circular_method(&self) -> Box<dyn CircularTrait> {
        Box::new((T::default(), 42u32))
    }
}

// Implementations for tuples in CircularTrait
impl<T, U> CircularTrait for (T, U)
where
    T: Default + Clone + Send + Sync + 'static,
    U: Send + Sync,
{
    fn circular_method(&self) -> Box<dyn CircularTrait> {
        let default_t = T::default();
        Box::new((default_t.clone(), default_t))
    }
}

// Implementations for slices in ExtendedTrait
impl<T> ExtendedTrait for [T]
where
    T: PartialEq + Clone,
{
    fn extended_method(&self) -> bool {
        if self.len() >= 2 {
            self[0] == self[1]
        } else {
            !self.is_empty()
        }
    }
}

// Implementations for tuples in ExtendedTrait
impl<T, U, V> ExtendedTrait for (T, U, V)
where
    T: Hash + Eq,
    U: Debug + Clone,
    V: Send + Sync,
{
    fn extended_method(&self) -> bool {
        let _ = self.1.clone();
        use std::collections::hash_map::DefaultHasher;
        use std::hash::Hasher;
        let mut hasher = DefaultHasher::new();
        self.0.hash(&mut hasher);
        hasher.finish().is_multiple_of(2)
    }
}

// A foreign `#[decycle]` trait whose default method body holds a `macro_rules!` using `$crate`
// (AUDIT decycle #1), and one whose default body holds a `$(..)*` repetition (AUDIT decycle #2).
pub fn dollar_crate_helper() -> usize {
    7
}

#[decycle::decycle]
pub trait DollarCrateDefault {
    fn base(&self) -> usize;
    fn with_helper(&self) -> usize {
        macro_rules! helper {
            () => {
                $crate::dollar_crate_helper()
            };
        }
        self.base() + helper!()
    }
}

#[decycle::decycle]
pub trait RepetitionDefault {
    fn base2(&self) -> usize;
    fn summed(&self) -> usize {
        macro_rules! sum {
            ($($x:expr),*) => { 0 $(+ $x)* };
        }
        sum!(self.base2(), 1, 2)
    }
}

// `mock = path`: a stand-in for a real trait that is not itself `#[decycle]` (here a plain trait
// of this crate; it could be any crate's). The stand-in is not emitted as a trait — only its
// trampoline, at the item's path (`traitdef::stand::Weigh!`), carrying a copy that `#[decycle]
// use` sites treat as `crate::real::Weigh`.
pub mod real {
    pub trait Weigh {
        fn weigh(&self, depth: usize) -> usize;
        fn doubled(&self) -> usize {
            self.weigh(0) * 2
        }
    }
}

pub mod stand {
    #[decycle::decycle(mock = crate::real::Weigh)]
    pub trait Weigh {
        fn weigh(&self, depth: usize) -> usize;
        fn doubled(&self) -> usize {
            self.weigh(0) * 2
        }
    }
}

// A stand-in named differently from its real trait: `#[decycle] use` it `as` the real name.
// Its copy keeps a relative path (`tys::Unit`, via `allowed_paths`): announce type-checks the
// mock copy in a hidden module, where it must still resolve.
pub mod stand_renamed {
    use crate::tys;

    #[decycle::decycle(mock = crate::real_unit::Measure, allowed_paths = [tys, usize])]
    pub trait MeasureStandIn {
        fn measure(&self, depth: usize) -> tys::Unit;
    }
}

pub mod real_unit {
    pub trait Measure {
        fn measure(&self, depth: usize) -> crate::tys::Unit;
    }
}

// `allowed_paths`: `tys` is not interned, it is promised to be in scope wherever the trait is
// used. Announced through announce's module wrapper, so announce does not reject the relative
// `tys::Unit` (AUDIT part 3, decycle #1).
pub mod tys {
    pub struct Unit(pub usize);
}

pub mod allowed {
    use crate::tys;

    #[decycle::decycle(allowed_paths = [tys, usize])]
    pub trait FromTys {
        fn unit(&self, depth: usize) -> tys::Unit;
    }
}
