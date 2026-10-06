//! Pattern syntax trees.
//!
//! `RawPat` is what the parser produces: constructor names are strings and
//! nothing has been checked.  `Pat` is the resolved form used by the compiler:
//! constructors are `ConId`s, variables are indices into the clause's binding
//! table, and list/tuple/bool sugar has been desugared into constructors.

use std::fmt;

/// Literal patterns.  Integer literals that do not fit in an `i64` are kept as
/// their canonical decimal text in `Big`.
#[derive(Clone, Debug, PartialEq)]
pub enum Lit {
    Int(i64),
    Big(String),
    Float(f64),
    Str(String),
    Sym(String),
}

/// A hashable identity for a literal (floats compared by bit pattern after
/// normalising `-0.0` to `0.0`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum LitKey {
    Int(i64),
    Big(String),
    Float(u64),
    Str(String),
    Sym(String),
}

/// The "type" of a literal for the purposes of type checking columns.  All
/// numeric literals share one type (like Haskell's `Num a => a` defaulting);
/// strings and symbols are separate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LitKind {
    Num,
    Str,
    Sym,
}

impl LitKind {
    pub fn name(self) -> &'static str {
        match self {
            LitKind::Num => "Num",
            LitKind::Str => "String",
            LitKind::Sym => "Symbol",
        }
    }
}

impl Lit {
    pub fn key(&self) -> LitKey {
        match self {
            Lit::Int(i) => LitKey::Int(*i),
            Lit::Big(s) => LitKey::Big(s.clone()),
            Lit::Float(f) => {
                let f = if *f == 0.0 { 0.0 } else { *f };
                LitKey::Float(f.to_bits())
            }
            Lit::Str(s) => LitKey::Str(s.clone()),
            Lit::Sym(s) => LitKey::Sym(s.clone()),
        }
    }

    pub fn kind(&self) -> LitKind {
        match self {
            Lit::Int(_) | Lit::Big(_) | Lit::Float(_) => LitKind::Num,
            Lit::Str(_) => LitKind::Str,
            Lit::Sym(_) => LitKind::Sym,
        }
    }

    pub fn same(&self, other: &Lit) -> bool {
        self.key() == other.key()
    }
}

impl fmt::Display for Lit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Lit::Int(i) => write!(f, "{}", i),
            Lit::Big(s) => write!(f, "{}", s),
            Lit::Float(x) => {
                if x.fract() == 0.0 && x.is_finite() {
                    write!(f, "{:.1}", x)
                } else {
                    write!(f, "{}", x)
                }
            }
            Lit::Str(s) => write!(f, "{:?}", s),
            Lit::Sym(s) => write!(f, ":{}", s),
        }
    }
}

/// Unresolved pattern straight from the parser.
#[derive(Clone, Debug, PartialEq)]
pub enum RawPat {
    Wild,
    Var(String),
    As(String, Box<RawPat>),
    Lazy(Box<RawPat>),
    Bang(Box<RawPat>),
    /// `Con p1 p2 ...` (positional application; `Con` alone is nullary).
    Con(String, Vec<RawPat>),
    /// `Con { f1 = p1, f2, .. }` — the `bool` is whether `..` was present.
    Record(String, Vec<(String, RawPat)>, bool),
    /// `(p1, p2, ...)` with 0 or 2+ elements (`()` is unit).
    Tuple(Vec<RawPat>),
    /// `[p1, p2, ...]`
    List(Vec<RawPat>),
    /// `p1 : p2`
    Cons(Box<RawPat>, Box<RawPat>),
    Lit(Lit),
}

pub type VarId = usize;

/// Resolved pattern.
#[derive(Clone, Debug, PartialEq)]
pub enum Pat {
    Wild,
    Var(VarId),
    As(VarId, Box<Pat>),
    /// Irrefutable pattern: never fails to match, bindings are extracted
    /// (and may fail) only once the clause is selected.
    Lazy(Box<Pat>),
    Con(super::types::ConId, Vec<Pat>),
    Lit(Lit),
}

impl Pat {
    /// Strip binders and laziness, leaving only the refutable skeleton.
    pub fn skeleton(&self) -> Pat {
        match self {
            Pat::Wild | Pat::Var(_) | Pat::Lazy(_) => Pat::Wild,
            Pat::As(_, p) => p.skeleton(),
            Pat::Con(c, args) => Pat::Con(*c, args.iter().map(Pat::skeleton).collect()),
            Pat::Lit(l) => Pat::Lit(l.clone()),
        }
    }

    pub fn is_wild(&self) -> bool {
        matches!(self, Pat::Wild)
    }
}
