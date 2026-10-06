//! Haskell-style rendering of patterns, used for diagnostics.

use super::ast::{Lit, Pat};
use super::types::{TypeEnv, TypeKind, CON_CONS, CON_NIL};

/// A witness pattern produced by the exhaustiveness checker.
#[derive(Clone, Debug, PartialEq)]
pub enum WPat {
    Wild,
    /// A literal position where the listed literals *are* handled: any other
    /// value of that type is unmatched.
    NotLit(Vec<Lit>),
    Lit(Lit),
    Con(super::types::ConId, Vec<WPat>),
}

impl WPat {
    pub fn from_pat(p: &Pat) -> WPat {
        match p {
            Pat::Wild | Pat::Var(_) | Pat::Lazy(_) => WPat::Wild,
            Pat::As(_, p) => WPat::from_pat(p),
            Pat::Lit(l) => WPat::Lit(l.clone()),
            Pat::Con(c, args) => WPat::Con(*c, args.iter().map(WPat::from_pat).collect()),
        }
    }
}

/// Render a witness vector the way GHC lists unmatched patterns:
/// `Just (Just _) []` plus a trailing note for literal positions.
pub fn render_witness(env: &TypeEnv, pats: &[WPat]) -> String {
    let mut parts = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    // GHC parenthesises constructor applications only when a clause has
    // several arguments: `Just (Just _)` but `(Circle _) (Rect _ _)`.
    let atomic = pats.len() > 1;
    for p in pats {
        parts.push(render(env, p, atomic, &mut notes));
    }
    let mut s = parts.join(" ");
    if !notes.is_empty() {
        s.push_str(" where ");
        s.push_str(&notes.join(" and "));
    }
    s
}

fn render(env: &TypeEnv, p: &WPat, atomic: bool, notes: &mut Vec<String>) -> String {
    match p {
        WPat::Wild => "_".to_string(),
        WPat::Lit(l) => render_lit(l, atomic),
        WPat::NotLit(lits) => {
            let name = format!("p{}", notes.len() + 1);
            let shown: Vec<String> = lits.iter().map(|l| l.to_string()).collect();
            notes.push(format!("{} is not one of {{{}}}", name, shown.join(", ")));
            name
        }
        WPat::Con(c, args) => {
            let con = env.con(*c);
            match env.ty(con.ty).kind {
                TypeKind::Bool => con.name.clone(),
                TypeKind::Tuple(_) => {
                    let inner: Vec<String> =
                        args.iter().map(|a| render(env, a, false, notes)).collect();
                    format!("({})", inner.join(", "))
                }
                TypeKind::List => {
                    if *c == CON_NIL {
                        return "[]".to_string();
                    }
                    // try to render as a list literal when the spine is fully known
                    let mut items = Vec::new();
                    let mut cur = p;
                    loop {
                        match cur {
                            WPat::Con(cc, a) if *cc == CON_CONS => {
                                items.push(&a[0]);
                                cur = &a[1];
                            }
                            WPat::Con(cc, _) if *cc == CON_NIL => {
                                let inner: Vec<String> =
                                    items.iter().map(|a| render(env, a, false, notes)).collect();
                                return format!("[{}]", inner.join(", "));
                            }
                            _ => break,
                        }
                    }
                    let head = render(env, &args[0], true, notes);
                    let tail = render_cons_tail(env, &args[1], notes);
                    format!("({}:{})", head, tail)
                }
                TypeKind::Adt => {
                    if args.is_empty() {
                        con.name.clone()
                    } else {
                        let inner: Vec<String> =
                            args.iter().map(|a| render(env, a, true, notes)).collect();
                        let s = format!("{} {}", con.name, inner.join(" "));
                        if atomic {
                            format!("({})", s)
                        } else {
                            s
                        }
                    }
                }
            }
        }
    }
}

fn render_lit(l: &Lit, atomic: bool) -> String {
    let negative = match l {
        Lit::Int(i) => *i < 0,
        Lit::Big(s) => s.starts_with('-'),
        Lit::Float(f) => *f < 0.0,
        _ => false,
    };
    if negative && atomic {
        format!("({})", l)
    } else {
        l.to_string()
    }
}

/// Inside `(h:t)`, a cons tail is rendered without its own parentheses.
fn render_cons_tail(env: &TypeEnv, p: &WPat, notes: &mut Vec<String>) -> String {
    match p {
        WPat::Con(c, args) if *c == CON_CONS => {
            let head = render(env, &args[0], true, notes);
            let tail = render_cons_tail(env, &args[1], notes);
            format!("{}:{}", head, tail)
        }
        other => render(env, other, true, notes),
    }
}

/// Render a resolved pattern (used for messages about user clauses).
pub fn render_pat(env: &TypeEnv, p: &Pat, names: &[String]) -> String {
    render_pat_inner(env, p, names, false)
}

fn render_pat_inner(env: &TypeEnv, p: &Pat, names: &[String], atomic: bool) -> String {
    match p {
        Pat::Wild => "_".into(),
        Pat::Var(v) => names.get(*v).cloned().unwrap_or_else(|| format!("v{}", v)),
        Pat::As(v, inner) => format!(
            "{}@{}",
            names.get(*v).cloned().unwrap_or_else(|| format!("v{}", v)),
            render_pat_inner(env, inner, names, true)
        ),
        Pat::Lazy(inner) => format!("~{}", render_pat_inner(env, inner, names, true)),
        Pat::Lit(l) => render_lit(l, atomic),
        Pat::Con(c, args) => {
            let con = env.con(*c);
            match env.ty(con.ty).kind {
                TypeKind::Bool => con.name.clone(),
                TypeKind::Tuple(_) => {
                    let inner: Vec<String> = args
                        .iter()
                        .map(|a| render_pat_inner(env, a, names, false))
                        .collect();
                    format!("({})", inner.join(", "))
                }
                TypeKind::List => {
                    if *c == CON_NIL {
                        return "[]".into();
                    }
                    let mut items = Vec::new();
                    let mut cur = p;
                    loop {
                        match cur {
                            Pat::Con(cc, a) if *cc == CON_CONS => {
                                items.push(&a[0]);
                                cur = &a[1];
                            }
                            Pat::Con(cc, _) if *cc == CON_NIL => {
                                let inner: Vec<String> = items
                                    .iter()
                                    .map(|a| render_pat_inner(env, a, names, false))
                                    .collect();
                                return format!("[{}]", inner.join(", "));
                            }
                            _ => break,
                        }
                    }
                    let mut parts = vec![render_pat_inner(env, &args[0], names, true)];
                    let mut tail = &args[1];
                    while let Pat::Con(cc, a) = tail {
                        if *cc != CON_CONS {
                            break;
                        }
                        parts.push(render_pat_inner(env, &a[0], names, true));
                        tail = &a[1];
                    }
                    parts.push(render_pat_inner(env, tail, names, true));
                    format!("({})", parts.join(":"))
                }
                TypeKind::Adt => {
                    if args.is_empty() {
                        con.name.clone()
                    } else {
                        let inner: Vec<String> = args
                            .iter()
                            .map(|a| render_pat_inner(env, a, names, true))
                            .collect();
                        let s = format!("{} {}", con.name, inner.join(" "));
                        if atomic {
                            format!("({})", s)
                        } else {
                            s
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::parser::parse_pattern;
    use crate::core::resolve::resolve_clause;
    use crate::core::types::ConSpec;

    fn env() -> TypeEnv {
        let mut env = TypeEnv::new();
        env.register(
            "Maybe",
            &[
                ConSpec {
                    name: "Nothing".into(),
                    arity: 0,
                    fields: None,
                    handle: 0,
                },
                ConSpec {
                    name: "Just".into(),
                    arity: 1,
                    fields: None,
                    handle: 0,
                },
            ],
        )
        .unwrap();
        env
    }

    fn roundtrip(env: &mut TypeEnv, src: &str) -> String {
        let raw = parse_pattern(src).unwrap();
        let (pats, b) = resolve_clause(env, &[raw]).unwrap();
        render_pat(env, &pats[0], &b.names)
    }

    #[test]
    fn renders_patterns() {
        let mut env = env();
        for (src, want) in [
            ("Just x", "Just x"),
            ("Just (Just _)", "Just (Just _)"),
            ("(x:xs)", "(x:xs)"),
            ("x:y:rest", "(x:y:rest)"),
            ("[a, b]", "[a, b]"),
            ("[]", "[]"),
            ("(a, Nothing, [])", "(a, Nothing, [])"),
            ("()", "()"),
            ("all@(x:_)", "all@(x:_)"),
            ("~(Just y)", "~(Just y)"),
            ("Just (-1)", "Just (-1)"),
            ("-1", "-1"),
            ("Just \"s\"", "Just \"s\""),
            ("Just :ok", "Just :ok"),
            ("Just 1.5", "Just 1.5"),
            ("True", "True"),
            ("Just (x:xs)", "Just (x:xs)"),
        ] {
            assert_eq!(roundtrip(&mut env, src), want);
        }
    }

    #[test]
    fn renders_witnesses() {
        let mut env = env();
        let just = env.lookup_con("Just").unwrap();
        let nothing = env.lookup_con("Nothing").unwrap();
        let t2 = env.tuple_con(2);
        let w = vec![
            WPat::Con(just, vec![WPat::Con(nothing, vec![])]),
            WPat::Con(
                CON_CONS,
                vec![
                    WPat::Wild,
                    WPat::Con(CON_CONS, vec![WPat::Wild, WPat::Wild]),
                ],
            ),
        ];
        assert_eq!(render_witness(&env, &w), "(Just Nothing) (_:_:_)");
        assert_eq!(render_witness(&env, &w[..1]), "Just Nothing");
        assert_eq!(
            render_witness(&env, &[WPat::Con(just, vec![WPat::Lit(Lit::Int(-1))])]),
            "Just (-1)"
        );
        let w = vec![WPat::Con(
            t2,
            vec![
                WPat::NotLit(vec![Lit::Int(0), Lit::Int(1)]),
                WPat::Con(CON_NIL, vec![]),
            ],
        )];
        assert_eq!(
            render_witness(&env, &w),
            "(p1, []) where p1 is not one of {0, 1}"
        );
        let w = vec![WPat::Con(
            CON_CONS,
            vec![WPat::Wild, WPat::Con(CON_NIL, vec![])],
        )];
        assert_eq!(render_witness(&env, &w), "[_]");
    }
}
