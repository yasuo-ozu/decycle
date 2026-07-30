//! `#[decycle(structural)]` needs an inline module body to rewrite; a bodyless `mod m;` declaration
//! has nothing to expand — fail closed.
use decycle::decycle;

#[decycle(structural)]
mod m;

fn main() {}
