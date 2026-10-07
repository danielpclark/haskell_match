//! AST of the Haskell subset, plus rendering of patterns back to text (the
//! Ruby code generator hands patterns to the pattern compiler as strings).

use crate::core::ast::{Lit, RawPat};
use crate::core::parser::DataDecl;

#[derive(Clone, Debug, PartialEq)]
pub enum Literal {
    Int(String),
    Float(String),
    Char(char),
    Str(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Var(String),
    Con(String),
    Lit(Literal),
    App(Box<Expr>, Vec<Expr>),
    BinOp(String, Box<Expr>, Box<Expr>),
    Neg(Box<Expr>),
    If(Box<Expr>, Box<Expr>, Box<Expr>),
    Case(Box<Expr>, Vec<Alt>),
    Let(Vec<Decl>, Box<Expr>),
    Lambda(Vec<RawPat>, Box<Expr>),
    List(Vec<Expr>),
    Tuple(Vec<Expr>),
    Range {
        from: Box<Expr>,
        then: Option<Box<Expr>>,
        to: Option<Box<Expr>>,
    },
    Comp(Box<Expr>, Vec<Qual>),
    /// `(e op)`
    SectionL(String, Box<Expr>),
    /// `(op e)`
    SectionR(String, Box<Expr>),
    /// `(op)`
    OpFun(String),
    /// MultiWayIf: `if | quals -> e | quals -> e`
    MultiIf(Vec<(Vec<Qual>, Expr)>),
    /// Record construction `Con { f = e, ... }`
    RecCon(String, Vec<(String, Expr)>),
    /// Record update `e { f = e, ... }`
    RecUpdate(Box<Expr>, Vec<(String, Expr)>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Qual {
    Gen(RawPat, Expr),
    Guard(Expr),
    Let(Vec<Decl>),
}

/// A guarded right-hand side: each alternative is a list of qualifiers (a
/// boolean guard, a pattern guard `p <- e`, or `let` bindings) and a body.
#[derive(Clone, Debug, PartialEq)]
pub enum Rhs {
    Plain(Expr),
    Guarded(Vec<(Vec<Qual>, Expr)>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Alt {
    pub pat: RawPat,
    pub rhs: Rhs,
    pub wheres: Vec<Decl>,
    pub line: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Equation {
    pub name: String,
    pub pats: Vec<RawPat>,
    pub rhs: Rhs,
    pub wheres: Vec<Decl>,
    pub line: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Decl {
    Data {
        decl: DataDecl,
        deriving: Vec<String>,
        line: usize,
    },
    Sig {
        names: Vec<String>,
        arity: usize,
        line: usize,
    },
    Fun(Vec<Equation>),
    PatBind {
        pat: RawPat,
        rhs: Rhs,
        wheres: Vec<Decl>,
        line: usize,
    },
    /// `import [qualified] M [as A] [hiding] [(items)]`
    Import {
        module: String,
        qualified: bool,
        alias: Option<String>,
        hiding: bool,
        /// Named items (functions, operators, types); `None` imports everything.
        items: Option<Vec<String>>,
        line: usize,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Module {
    pub name: Option<String>,
    /// Names in the export list (functions, operators, types); `None` when
    /// there is no list, meaning everything is exported.
    pub exports: Option<Vec<String>>,
    pub decls: Vec<Decl>,
}

/// Mangle a Haskell identifier into a name that is a valid Haskell *and* Ruby
/// identifier and cannot clash with Ruby keywords or methods: `x'` → `hs_x_q`.
pub fn mangle(name: &str) -> String {
    let mut s = String::from("hs_");
    for c in name.chars() {
        if c == '\'' {
            s.push_str("_q");
        } else {
            s.push(c);
        }
    }
    s
}

/// Render a pattern as Haskell text with variables mangled, for the pattern
/// compiler.
pub fn render_pat(p: &RawPat) -> String {
    render(p, false)
}

fn render(p: &RawPat, atomic: bool) -> String {
    match p {
        RawPat::Wild => "_".into(),
        RawPat::Var(v) => mangle(v),
        RawPat::As(v, inner) => format!("{}@{}", mangle(v), render(inner, true)),
        RawPat::Lazy(inner) => format!("~{}", render(inner, true)),
        RawPat::Bang(inner) => format!("!{}", render(inner, true)),
        RawPat::Lit(l) => render_lit(l, atomic),
        RawPat::Tuple(items) => format!(
            "({})",
            items
                .iter()
                .map(|i| render(i, false))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        RawPat::List(items) => format!(
            "[{}]",
            items
                .iter()
                .map(|i| render(i, false))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        RawPat::Cons(h, t) => format!("({}:{})", render(h, true), render(t, true)),
        RawPat::Hash(fields) => format!(
            "{{{}}}",
            fields
                .iter()
                .map(|(k, p)| format!("{} = {}", k, render(p, false)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        RawPat::Con(name, args) if name.starts_with(':') => {
            if args.len() == 2 {
                format!(
                    "({} {} {})",
                    render(&args[0], true),
                    name,
                    render(&args[1], true)
                )
            } else if args.is_empty() {
                format!("({})", name)
            } else {
                format!(
                    "(({}) {})",
                    name,
                    args.iter()
                        .map(|a| render(a, true))
                        .collect::<Vec<_>>()
                        .join(" ")
                )
            }
        }
        RawPat::Con(name, args) => {
            if args.is_empty() {
                name.clone()
            } else {
                let s = format!(
                    "{} {}",
                    name,
                    args.iter()
                        .map(|a| render(a, true))
                        .collect::<Vec<_>>()
                        .join(" ")
                );
                if atomic {
                    format!("({})", s)
                } else {
                    s
                }
            }
        }
        RawPat::Record(name, fields, rest) => {
            let mut parts: Vec<String> = fields
                .iter()
                .map(|(f, p)| format!("{} = {}", f, render(p, false)))
                .collect();
            if *rest {
                parts.push("..".into());
            }
            format!("{} {{{}}}", name, parts.join(", "))
        }
    }
}

fn render_lit(l: &Lit, atomic: bool) -> String {
    let s = l.to_string();
    let negative = s.starts_with('-');
    if negative && atomic {
        format!("({})", s)
    } else {
        s
    }
}

/// Variables bound by a pattern, in order of appearance (unmangled).
pub fn pat_vars(p: &RawPat, out: &mut Vec<String>) {
    match p {
        RawPat::Wild | RawPat::Lit(_) => {}
        RawPat::Var(v) => out.push(v.clone()),
        RawPat::As(v, inner) => {
            out.push(v.clone());
            pat_vars(inner, out);
        }
        RawPat::Lazy(inner) | RawPat::Bang(inner) => pat_vars(inner, out),
        RawPat::Tuple(items) | RawPat::List(items) => items.iter().for_each(|i| pat_vars(i, out)),
        RawPat::Cons(h, t) => {
            pat_vars(h, out);
            pat_vars(t, out);
        }
        RawPat::Con(_, args) => args.iter().for_each(|a| pat_vars(a, out)),
        RawPat::Record(_, fields, _) => fields.iter().for_each(|(_, p)| pat_vars(p, out)),
        RawPat::Hash(fields) => fields.iter().for_each(|(_, p)| pat_vars(p, out)),
    }
}
