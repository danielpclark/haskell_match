//! The exceptions the native layer raises, declared with `#[rubyerror]`.
//!
//! The Ruby classes themselves, with their `CompileError`/`MatchError` tiers
//! and `PatternSyntaxError.with_caret`, are defined in
//! `lib/haskell_match/errors.rb` before the extension loads; `#[rubyerror]`
//! finds each class by its path.  A method that returns `Err(HmError)` raises
//! the matching exception once its Rust values have been dropped, instead of
//! `longjmp`ing past them with `VM::raise_ex`.

use std::fmt;

use rutie::derive::IntoException;
use rutie::{rubyerror, AnyException, Exception};

use crate::core::error::{CoreError, ErrorKind};

/// Every `HaskellMatch` exception the extension raises, one variant per
/// Ruby class, each carrying its message.
#[rubyerror(path = "HaskellMatch::Error", superclass = "StandardError")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HmError {
    /// Malformed pattern text.
    #[ruby(path = "HaskellMatch::PatternSyntaxError")]
    Syntax(String),
    /// A constructor name that no registered data type declares.
    #[ruby(path = "HaskellMatch::UnknownConstructorError")]
    UnknownConstructor(String),
    /// A constructor applied to the wrong number of arguments.
    #[ruby(path = "HaskellMatch::ArityError")]
    Arity(String),
    /// Patterns of different types in the same position.
    #[ruby(path = "HaskellMatch::PatternTypeError")]
    Type(String),
    /// The same variable bound twice in one clause.
    #[ruby(path = "HaskellMatch::DuplicateVariableError")]
    DuplicateVariable(String),
    /// Record pattern problems (unknown field, positional constructor, ...).
    #[ruby(path = "HaskellMatch::FieldError")]
    Field(String),
    /// Malformed `data` declaration.
    #[ruby(path = "HaskellMatch::DataDeclarationError")]
    DataDeclaration(String),
    /// Clauses of one function with different numbers of arguments.
    #[ruby(path = "HaskellMatch::ClauseArityError")]
    ClauseArity(String),
    /// No clause matched at call time.
    #[ruby(path = "HaskellMatch::MatchError")]
    NoMatch(String),
    /// A value is not of the type the patterns expect.
    #[ruby(path = "HaskellMatch::TypeMismatchError")]
    TypeMismatch(String),
    /// An irrefutable (`~`) pattern failed to destructure.
    #[ruby(path = "HaskellMatch::IrrefutablePatternError")]
    IrrefutablePattern(String),
    /// Recursion through compiled functions exceeded `HaskellMatch.max_depth`.
    #[ruby(path = "HaskellMatch::StackOverflowError")]
    StackOverflow(String),
}

impl HmError {
    /// The exception's message.
    pub fn message(&self) -> &str {
        match self {
            HmError::Syntax(m)
            | HmError::UnknownConstructor(m)
            | HmError::Arity(m)
            | HmError::Type(m)
            | HmError::DuplicateVariable(m)
            | HmError::Field(m)
            | HmError::DataDeclaration(m)
            | HmError::ClauseArity(m)
            | HmError::NoMatch(m)
            | HmError::TypeMismatch(m)
            | HmError::IrrefutablePattern(m)
            | HmError::StackOverflow(m) => m,
        }
    }
}

impl fmt::Display for HmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for HmError {}

/// A compile-time error of the pure core, by kind.
impl From<CoreError> for HmError {
    fn from(e: CoreError) -> Self {
        let m = e.message;
        match e.kind {
            ErrorKind::Syntax => HmError::Syntax(m),
            ErrorKind::UnknownConstructor => HmError::UnknownConstructor(m),
            ErrorKind::Arity => HmError::Arity(m),
            ErrorKind::Type => HmError::Type(m),
            ErrorKind::DuplicateVariable => HmError::DuplicateVariable(m),
            ErrorKind::Field => HmError::Field(m),
            ErrorKind::DataDeclaration => HmError::DataDeclaration(m),
            ErrorKind::ClauseArity => HmError::ClauseArity(m),
        }
    }
}

/// So `?` can mix `HmError`s and `ArgumentError`s in a method returning
/// `Result<T, AnyException>`.
impl From<HmError> for AnyException {
    fn from(e: HmError) -> Self {
        e.into_exception()
    }
}

impl From<CoreError> for AnyException {
    fn from(e: CoreError) -> Self {
        HmError::from(e).into_exception()
    }
}

/// An `ArgumentError`: a well-typed argument with a value that makes no
/// sense (a wrong type is the `TypeError` the argument conversion raises).
pub fn argument_error(message: &str) -> AnyException {
    AnyException::new("ArgumentError", Some(message))
}
