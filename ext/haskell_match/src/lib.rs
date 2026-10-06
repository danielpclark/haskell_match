//! haskell_match: Haskell-style pattern matching for Ruby.
//!
//! The `core` module is pure Rust (parser, type checking, exhaustiveness
//! analysis, decision-tree compilation) and is unit tested on its own.  The
//! `ruby` module binds it to Ruby through Rutie.

#[macro_use]
extern crate rutie;
#[macro_use]
extern crate lazy_static;

pub mod core;
pub mod ruby;

#[allow(non_snake_case)]
#[no_mangle]
pub extern "C" fn Init_haskell_match() {
    ruby::init();
}
