//! A Haskell front end: a positioned lexer, the layout algorithm, and a
//! parser for the subset of Haskell 2010 that haskell_match compiles to Ruby
//! (data declarations, type signatures, function equations with guards and
//! `where`, and expressions).  The parser emits a JSON AST consumed by the
//! Ruby code generator in `lib/haskell_match/haskell/`.

pub mod ast;
pub mod json;
pub mod layout;
pub mod lexer;
pub mod parser;

pub use ast::*;
pub use parser::parse_module;
