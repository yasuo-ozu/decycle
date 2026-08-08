//! `structural` selects the module-level engine; on a TRAIT item it used to be silently
//! ignored (ranked machinery emitted, the selector dropped) while its siblings
//! `recurse_level`/`support_infinite_cycle` were rejected. Now it is rejected like them.
#[decycle::decycle(structural)]
trait BadTrait {
    fn value(&self) -> i32;
}

fn main() {}
