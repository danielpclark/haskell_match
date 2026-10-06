//! Pure-Rust core: no Ruby dependencies.  Everything in this module is unit
//! tested with plain `cargo test` and is shared between the Ruby bindings and
//! the test suite.

pub mod ast;
pub mod error;
pub mod exhaust;
pub mod lexer;
pub mod parser;
pub mod pretty;
pub mod resolve;
pub mod tree;
pub mod typecheck;
pub mod types;

pub use ast::{Lit, LitKey, LitKind, Pat, RawPat, VarId};
pub use error::{CoreError, ErrorKind};
pub use types::{ConId, Constructor, DataType, TypeEnv, TypeId, TypeKind};
