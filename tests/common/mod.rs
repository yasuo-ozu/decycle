//! Shared harness for running a scenario under BOTH decycle algorithms: the existing ranked engine
//! (`#[decycle]`) and the structural unroll (`#[decycle(structural)]`). Lives in a subdirectory so
//! cargo does not compile it as its own test binary.

/// Define a module `$name` twice — `$name::ranked` (ranked engine) and `$name::structural`
/// (structural unroll) — from one body.
#[macro_export]
macro_rules! dual_mod {
    ($name:ident { $($items:tt)* }) => {
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
    };
}

/// Run a block against both algorithm variants of a `dual_mod!` module (its items brought in scope
/// with `use $name::{ranked,structural}::*`).
#[macro_export]
macro_rules! on_both {
    ($name:ident, $body:block) => {{
        {
            use $name::ranked::*;
            $body
        }
        {
            use $name::structural::*;
            $body
        }
    }};
}
