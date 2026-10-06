//! Pattern syntax trees.
//!
//! `RawPat` is what the parser produces: constructor names are strings and
//! nothing has been checked.  `Pat` is the resolved form used by the compiler:
//! constructors are `ConId`s, variables are indices into the clause's binding
//! table, and list/tuple/bool sugar has been desugared into constructors.

use std::fmt;

/// Literal patterns.  Integer literals that do not fit in an `i64` are kept as
/// their canonical decimal text in `Big`.  There is no string literal: as in
/// Haskell, `"abc"` is the list `['a', 'b', 'c']` and is desugared by the
/// parser; `Char` holds one character (matched against a one-character Ruby
/// String, or a character of a String viewed as a list).
#[derive(Clone, Debug, PartialEq)]
pub enum Lit {
    Int(i64),
    Big(String),
    Float(f64),
    Char(char),
    Sym(String),
}

/// A hashable identity for a literal (floats compared by bit pattern after
/// normalising `-0.0` to `0.0`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum LitKey {
    Int(i64),
    Big(String),
    Float(u64),
    Char(char),
    Sym(String),
}

/// The "type" of a literal for the purposes of type checking columns.  All
/// numeric literals share one type (like Haskell's `Num a => a` defaulting);
/// strings and symbols are separate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LitKind {
    Num,
    Char,
    Sym,
}

impl LitKind {
    pub fn name(self) -> &'static str {
        match self {
            LitKind::Num => "Num",
            LitKind::Char => "Char",
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
            Lit::Char(c) => LitKey::Char(*c),
            Lit::Sym(s) => LitKey::Sym(s.clone()),
        }
    }

    pub fn kind(&self) -> LitKind {
        match self {
            Lit::Int(_) | Lit::Big(_) | Lit::Float(_) => LitKind::Num,
            Lit::Char(_) => LitKind::Char,
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
            Lit::Char(c) => write!(f, "{:?}", c),
            Lit::Sym(s) => write!(f, ":{}", s),
        }
    }
}

/// A key of a Hash pattern: a Ruby Symbol or String.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum HKey {
    Sym(String),
    Str(String),
}

impl fmt::Display for HKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HKey::Sym(s) => {
                let plain = s
                    .chars()
                    .next()
                    .map(|c| c.is_ascii_lowercase() || c == '_')
                    .unwrap_or(false)
                    && s.chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '\'');
                if plain {
                    write!(f, "{}", s)
                } else {
                    write!(f, ":{:?}", s)
                }
            }
            HKey::Str(s) => write!(f, "{:?}", s),
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
    /// `{ name = p1, "key" = p2 }`: a Ruby Hash with (at least) these keys.
    Hash(Vec<(HKey, RawPat)>),
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
    /// A Hash having every listed key, whose values match the sub-patterns
    /// (other keys are ignored, as fields not mentioned in a record pattern).
    Hash(Vec<(HKey, Pat)>),
}

impl Pat {
    /// Strip binders and laziness, leaving only the refutable skeleton.
    pub fn skeleton(&self) -> Pat {
        match self {
            Pat::Wild | Pat::Var(_) | Pat::Lazy(_) => Pat::Wild,
            Pat::As(_, p) => p.skeleton(),
            Pat::Con(c, args) => Pat::Con(*c, args.iter().map(Pat::skeleton).collect()),
            Pat::Lit(l) => Pat::Lit(l.clone()),
            Pat::Hash(fields) => Pat::Hash(
                fields
                    .iter()
                    .map(|(k, p)| (k.clone(), p.skeleton()))
                    .collect(),
            ),
        }
    }

    pub fn is_wild(&self) -> bool {
        matches!(self, Pat::Wild)
    }
}
