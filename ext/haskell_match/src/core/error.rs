use std::fmt;

/// Classification of a compile-time error.  The Ruby layer maps each kind to a
/// dedicated exception class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    /// Malformed pattern text.
    Syntax,
    /// Constructor name that is not registered in the type environment.
    UnknownConstructor,
    /// Constructor applied to the wrong number of arguments.
    Arity,
    /// Patterns of different types in the same position.
    Type,
    /// The same variable bound twice in one clause.
    DuplicateVariable,
    /// Record pattern problems (unknown field, positional constructor used
    /// with named fields, duplicate field).
    Field,
    /// Malformed `data` declaration.
    DataDeclaration,
    /// Clauses of one function with different numbers of arguments.
    ClauseArity,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoreError {
    pub kind: ErrorKind,
    pub message: String,
}

impl CoreError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        CoreError {
            kind,
            message: message.into(),
        }
    }
    pub fn syntax(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Syntax, message)
    }
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for CoreError {}

pub type Result<T> = std::result::Result<T, CoreError>;
