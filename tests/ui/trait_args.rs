#[decycle::decycle(recurse_level = 4)]
trait BadTrait {
    fn value(&self) -> i32;
}

fn main() {}
