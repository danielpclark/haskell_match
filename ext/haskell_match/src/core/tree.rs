//! Compilation of a clause matrix into a decision tree.
//!
//! The tree is evaluated against an environment of *slots*.  Slots `0..arity`
//! hold the function arguments; every constructor test that succeeds copies
//! the constructor's fields into freshly numbered slots, so a leaf can name
//! each bound variable by slot.  Each argument position is inspected at most
//! once per path through the tree.

use super::ast::{Lit, LitKind, Pat, VarId};
use super::types::{ConId, TypeEnv, TypeId};

#[derive(Clone, Debug, PartialEq)]
pub enum Tree {
    /// No clause matches.
    Fail,
    Leaf(Leaf),
    /// A clause whose pattern matched but which has a guard; if the guard
    /// is false, evaluation continues with `fallback`.
    Guard {
        leaf: Leaf,
        fallback: Box<Tree>,
    },
    SwitchCon {
        slot: usize,
        ty: TypeId,
        cases: Vec<ConCase>,
        default: Option<Box<Tree>>,
    },
    SwitchLit {
        slot: usize,
        kind: LitKind,
        cases: Vec<(Lit, Tree)>,
        default: Box<Tree>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct ConCase {
    pub con: ConId,
    pub tag: usize,
    pub arity: usize,
    /// First slot receiving this constructor's fields.
    pub base: usize,
    pub tree: Tree,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Leaf {
    pub clause: usize,
    /// Where each variable of the clause (by `VarId`) gets its value.
    pub binds: Vec<BindSrc>,
    /// Irrefutable sub-patterns to destructure once the clause is chosen.
    pub lazies: Vec<LazyPat>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BindSrc {
    Slot(usize),
    /// Filled in by evaluating `Leaf::lazies[i]`.
    Lazy(usize),
}

#[derive(Clone, Debug, PartialEq)]
pub struct LazyPat {
    pub slot: usize,
    pub pat: Pat,
}

#[derive(Clone, Debug)]
pub struct Compiled {
    pub tree: Tree,
    pub arity: usize,
    pub n_slots: usize,
}

#[derive(Clone, Debug)]
struct Row {
    pats: Vec<Pat>,
    clause: usize,
    binds: Vec<Option<BindSrc>>,
    lazies: Vec<LazyPat>,
    guarded: bool,
}

struct Compiler<'a> {
    env: &'a TypeEnv,
    n_slots: usize,
}

/// Input clause: argument patterns, number of variables, guarded flag.
pub struct Clause {
    pub pats: Vec<Pat>,
    pub n_vars: usize,
    pub guarded: bool,
}

pub fn compile(env: &TypeEnv, clauses: &[Clause], arity: usize) -> Compiled {
    let mut c = Compiler {
        env,
        n_slots: arity,
    };
    let occs: Vec<usize> = (0..arity).collect();
    let mut rows: Vec<Row> = clauses
        .iter()
        .enumerate()
        .map(|(i, cl)| Row {
            pats: cl.pats.clone(),
            clause: i,
            binds: vec![None; cl.n_vars],
            lazies: Vec::new(),
            guarded: cl.guarded,
        })
        .collect();
    for r in rows.iter_mut() {
        normalize(r, &occs);
    }
    let tree = c.compile(&occs, rows);
    Compiled {
        tree,
        arity,
        n_slots: c.n_slots,
    }
}

/// Collect the variables bound inside a pattern.
fn vars_in(p: &Pat, out: &mut Vec<VarId>) {
    match p {
        Pat::Wild | Pat::Lit(_) => {}
        Pat::Var(v) => out.push(*v),
        Pat::As(v, inner) => {
            out.push(*v);
            vars_in(inner, out);
        }
        Pat::Lazy(inner) => vars_in(inner, out),
        Pat::Con(_, args) => args.iter().for_each(|a| vars_in(a, out)),
    }
}

/// Strip binders from every column so each pattern is `Wild`, `Con` or
/// `Lit`, recording where the stripped variables get their values.
fn normalize(row: &mut Row, occs: &[usize]) {
    for (i, slot) in occs.iter().enumerate() {
        loop {
            match std::mem::replace(&mut row.pats[i], Pat::Wild) {
                Pat::Var(v) => {
                    row.binds[v] = Some(BindSrc::Slot(*slot));
                    break;
                }
                Pat::As(v, inner) => {
                    row.binds[v] = Some(BindSrc::Slot(*slot));
                    row.pats[i] = *inner;
                }
                Pat::Lazy(inner) => {
                    let idx = row.lazies.len();
                    let mut vs = Vec::new();
                    vars_in(&inner, &mut vs);
                    for v in vs {
                        row.binds[v] = Some(BindSrc::Lazy(idx));
                    }
                    row.lazies.push(LazyPat {
                        slot: *slot,
                        pat: *inner,
                    });
                    break;
                }
                other => {
                    row.pats[i] = other;
                    break;
                }
            }
        }
    }
}

impl<'a> Compiler<'a> {
    fn alloc(&mut self, n: usize) -> usize {
        let base = self.n_slots;
        self.n_slots += n;
        base
    }

    fn leaf(row: &Row) -> Leaf {
        Leaf {
            clause: row.clause,
            binds: row
                .binds
                .iter()
                .map(|b| b.expect("every clause variable is bound by its pattern"))
                .collect(),
            lazies: row.lazies.clone(),
        }
    }

    fn compile(&mut self, occs: &[usize], rows: Vec<Row>) -> Tree {
        if rows.is_empty() {
            return Tree::Fail;
        }
        let first = &rows[0];
        let col = match first.pats.iter().position(|p| !p.is_wild()) {
            None => {
                let leaf = Self::leaf(first);
                if first.guarded {
                    let rest = rows[1..].to_vec();
                    return Tree::Guard {
                        leaf,
                        fallback: Box::new(self.compile(occs, rest)),
                    };
                }
                return Tree::Leaf(leaf);
            }
            Some(c) => c,
        };
        let slot = occs[col];
        match &first.pats[col] {
            Pat::Con(c0, _) => {
                let ty = self.env.type_of_con(*c0);
                let mut heads: Vec<ConId> = Vec::new();
                for r in &rows {
                    if let Pat::Con(c, _) = &r.pats[col] {
                        if !heads.contains(c) {
                            heads.push(*c);
                        }
                    }
                }
                let mut cases = Vec::with_capacity(heads.len());
                for c in &heads {
                    let arity = self.env.con(*c).arity;
                    let base = self.alloc(arity);
                    let mut new_occs: Vec<usize> = occs[..col].to_vec();
                    new_occs.extend(base..base + arity);
                    new_occs.extend_from_slice(&occs[col + 1..]);
                    let mut new_rows = Vec::new();
                    for r in &rows {
                        match &r.pats[col] {
                            Pat::Con(cc, args) if cc == c => {
                                let mut nr = r.clone();
                                nr.pats = r.pats[..col].to_vec();
                                nr.pats.extend(args.iter().cloned());
                                nr.pats.extend_from_slice(&r.pats[col + 1..]);
                                normalize(&mut nr, &new_occs);
                                new_rows.push(nr);
                            }
                            Pat::Wild => {
                                let mut nr = r.clone();
                                nr.pats = r.pats[..col].to_vec();
                                nr.pats.extend(std::iter::repeat_n(Pat::Wild, arity));
                                nr.pats.extend_from_slice(&r.pats[col + 1..]);
                                new_rows.push(nr);
                            }
                            _ => {}
                        }
                    }
                    cases.push(ConCase {
                        con: *c,
                        tag: self.env.con(*c).tag,
                        arity,
                        base,
                        tree: self.compile(&new_occs, new_rows),
                    });
                }
                let default = if self.env.is_complete(ty, &heads) {
                    None
                } else {
                    Some(Box::new(self.compile_default(occs, col, &rows)))
                };
                Tree::SwitchCon {
                    slot,
                    ty,
                    cases,
                    default,
                }
            }
            Pat::Lit(l0) => {
                let kind = l0.kind();
                let mut lits: Vec<Lit> = Vec::new();
                for r in &rows {
                    if let Pat::Lit(l) = &r.pats[col] {
                        if !lits.iter().any(|x| x.same(l)) {
                            lits.push(l.clone());
                        }
                    }
                }
                let mut cases = Vec::with_capacity(lits.len());
                let mut new_occs: Vec<usize> = occs[..col].to_vec();
                new_occs.extend_from_slice(&occs[col + 1..]);
                for l in &lits {
                    let mut new_rows = Vec::new();
                    for r in &rows {
                        let keep = match &r.pats[col] {
                            Pat::Lit(ll) => ll.same(l),
                            Pat::Wild => true,
                            _ => false,
                        };
                        if keep {
                            let mut nr = r.clone();
                            nr.pats.remove(col);
                            new_rows.push(nr);
                        }
                    }
                    cases.push((l.clone(), self.compile(&new_occs, new_rows)));
                }
                let default = Box::new(self.compile_default(occs, col, &rows));
                Tree::SwitchLit {
                    slot,
                    kind,
                    cases,
                    default,
                }
            }
            _ => unreachable!("normalized rows contain only Wild, Con and Lit"),
        }
    }

    fn compile_default(&mut self, occs: &[usize], col: usize, rows: &[Row]) -> Tree {
        let mut new_occs: Vec<usize> = occs[..col].to_vec();
        new_occs.extend_from_slice(&occs[col + 1..]);
        let new_rows: Vec<Row> = rows
            .iter()
            .filter(|r| r.pats[col].is_wild())
            .map(|r| {
                let mut nr = r.clone();
                nr.pats.remove(col);
                nr
            })
            .collect();
        self.compile(&new_occs, new_rows)
    }
}

#[cfg(test)]
pub mod interp {
    //! A reference interpreter over an abstract value model, used to test the
    //! compiled trees without Ruby.
    use super::*;
    use crate::core::types::{TypeKind, CON_CONS, CON_FALSE, CON_NIL, CON_TRUE};

    #[derive(Clone, Debug, PartialEq)]
    pub enum Val {
        Con(ConId, Vec<Val>),
        List(Vec<Val>),
        Tuple(Vec<Val>),
        Bool(bool),
        Int(i64),
        Str(String),
        Other(String),
    }

    #[derive(Debug, PartialEq)]
    pub enum Outcome {
        Matched { clause: usize, binds: Vec<Val> },
        NoMatch,
        TypeMismatch(TypeId),
        LazyFailed,
    }

    fn con_of(env: &TypeEnv, ty: TypeId, v: &Val) -> Option<(usize, Vec<Val>)> {
        match (env.ty(ty).kind, v) {
            (TypeKind::Adt, Val::Con(c, fields)) if env.type_of_con(*c) == ty => {
                Some((env.con(*c).tag, fields.clone()))
            }
            (TypeKind::Bool, Val::Bool(b)) => Some((if *b { 1 } else { 0 }, vec![])),
            (TypeKind::List, Val::List(items)) => {
                if items.is_empty() {
                    Some((0, vec![]))
                } else {
                    Some((1, vec![items[0].clone(), Val::List(items[1..].to_vec())]))
                }
            }
            (TypeKind::Tuple(n), Val::Tuple(items)) if items.len() == n => Some((0, items.clone())),
            _ => None,
        }
    }

    fn lit_eq(l: &Lit, v: &Val) -> bool {
        match (l, v) {
            (Lit::Int(i), Val::Int(j)) => i == j,
            (Lit::Str(s), Val::Str(t)) => s == t,
            _ => false,
        }
    }

    fn lit_kind_ok(k: LitKind, v: &Val) -> bool {
        matches!(
            (k, v),
            (LitKind::Num, Val::Int(_)) | (LitKind::Str, Val::Str(_))
        )
    }

    /// Match `pat` against `v` interpretively (for lazy patterns).
    fn destructure(env: &TypeEnv, pat: &Pat, v: &Val, out: &mut Vec<Option<Val>>) -> bool {
        match pat {
            Pat::Wild => true,
            Pat::Var(x) => {
                out[*x] = Some(v.clone());
                true
            }
            Pat::As(x, p) => {
                out[*x] = Some(v.clone());
                destructure(env, p, v, out)
            }
            Pat::Lazy(p) => destructure(env, p, v, out),
            Pat::Lit(l) => lit_eq(l, v),
            Pat::Con(c, args) => {
                let ty = env.type_of_con(*c);
                let _ = (CON_CONS, CON_NIL, CON_TRUE, CON_FALSE);
                match con_of(env, ty, v) {
                    Some((tag, fields)) if tag == env.con(*c).tag => args
                        .iter()
                        .zip(fields.iter())
                        .all(|(a, f)| destructure(env, a, f, out)),
                    _ => false,
                }
            }
        }
    }

    pub fn eval(
        env: &TypeEnv,
        compiled: &Compiled,
        args: &[Val],
        guards: &dyn Fn(usize, &[Val]) -> bool,
    ) -> Outcome {
        let mut slots: Vec<Option<Val>> = vec![None; compiled.n_slots];
        for (i, a) in args.iter().enumerate() {
            slots[i] = Some(a.clone());
        }
        let mut node = &compiled.tree;
        loop {
            match node {
                Tree::Fail => return Outcome::NoMatch,
                Tree::Leaf(leaf) => return bind(env, leaf, &slots),
                Tree::Guard { leaf, fallback } => match bind(env, leaf, &slots) {
                    Outcome::Matched { clause, binds } => {
                        if guards(clause, &binds) {
                            return Outcome::Matched { clause, binds };
                        }
                        node = fallback;
                    }
                    other => return other,
                },
                Tree::SwitchCon {
                    slot,
                    ty,
                    cases,
                    default,
                } => {
                    let v = slots[*slot].clone().unwrap();
                    match con_of(env, *ty, &v) {
                        None => return Outcome::TypeMismatch(*ty),
                        Some((tag, fields)) => match cases.iter().find(|c| c.tag == tag) {
                            Some(case) => {
                                for (i, f) in fields.into_iter().enumerate() {
                                    slots[case.base + i] = Some(f);
                                }
                                node = &case.tree;
                            }
                            None => match default {
                                Some(d) => node = d,
                                None => panic!("complete switch without case for tag {}", tag),
                            },
                        },
                    }
                }
                Tree::SwitchLit {
                    slot,
                    kind,
                    cases,
                    default,
                } => {
                    let v = slots[*slot].clone().unwrap();
                    if !lit_kind_ok(*kind, &v) {
                        return Outcome::TypeMismatch(usize::MAX);
                    }
                    match cases.iter().find(|(l, _)| lit_eq(l, &v)) {
                        Some((_, t)) => node = t,
                        None => node = default,
                    }
                }
            }
        }
    }

    fn bind(env: &TypeEnv, leaf: &Leaf, slots: &[Option<Val>]) -> Outcome {
        let mut out: Vec<Option<Val>> = vec![None; leaf.binds.len()];
        for lz in &leaf.lazies {
            let v = slots[lz.slot].clone().unwrap();
            if !destructure(env, &lz.pat, &v, &mut out) {
                return Outcome::LazyFailed;
            }
        }
        for (i, b) in leaf.binds.iter().enumerate() {
            if let BindSrc::Slot(s) = b {
                out[i] = slots[*s].clone();
            }
        }
        Outcome::Matched {
            clause: leaf.clause,
            binds: out.into_iter().map(|v| v.expect("bound")).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::interp::{eval, Outcome, Val};
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

    fn build(env: &mut TypeEnv, clauses: &[&[&str]]) -> (Compiled, Vec<Vec<String>>) {
        let mut cs = Vec::new();
        let mut names = Vec::new();
        let mut arity = None;
        for c in clauses {
            let guarded = c.last() == Some(&"|");
            let pats: Vec<&str> = c.iter().copied().filter(|s| *s != "|").collect();
            arity = Some(pats.len());
            let raws: Vec<_> = pats.iter().map(|s| parse_pattern(s).unwrap()).collect();
            let (ps, b) = resolve_clause(env, &raws).unwrap();
            cs.push(Clause {
                pats: ps,
                n_vars: b.names.len(),
                guarded,
            });
            names.push(b.names);
        }
        (compile(env, &cs, arity.unwrap()), names)
    }

    fn just(env: &TypeEnv, v: Val) -> Val {
        Val::Con(env.lookup_con("Just").unwrap(), vec![v])
    }
    fn nothing(env: &TypeEnv) -> Val {
        Val::Con(env.lookup_con("Nothing").unwrap(), vec![])
    }
    fn list(items: Vec<Val>) -> Val {
        Val::List(items)
    }
    fn no_guards(_: usize, _: &[Val]) -> bool {
        true
    }

    fn matched(clause: usize, binds: Vec<Val>) -> Outcome {
        Outcome::Matched { clause, binds }
    }

    #[test]
    fn maybe_function() {
        let mut env = env();
        let (c, names) = build(&mut env, &[&["Nothing"], &["Just x"]]);
        assert_eq!(names, vec![Vec::<String>::new(), vec!["x".to_string()]]);
        assert_eq!(
            eval(&env, &c, &[nothing(&env)], &no_guards),
            matched(0, vec![])
        );
        assert_eq!(
            eval(&env, &c, &[just(&env, Val::Int(5))], &no_guards),
            matched(1, vec![Val::Int(5)])
        );
        assert_eq!(
            eval(&env, &c, &[Val::Int(5)], &no_guards),
            Outcome::TypeMismatch(env.lookup_type("Maybe").unwrap())
        );
    }

    #[test]
    fn list_functions() {
        let mut env = env();
        let (c, _) = build(&mut env, &[&["[]"], &["[x]"], &["(x:y:rest)"]]);
        assert_eq!(
            eval(&env, &c, &[list(vec![])], &no_guards),
            matched(0, vec![])
        );
        assert_eq!(
            eval(&env, &c, &[list(vec![Val::Int(1)])], &no_guards),
            matched(1, vec![Val::Int(1)])
        );
        assert_eq!(
            eval(
                &env,
                &c,
                &[list(vec![Val::Int(1), Val::Int(2), Val::Int(3)])],
                &no_guards
            ),
            matched(2, vec![Val::Int(1), Val::Int(2), list(vec![Val::Int(3)])])
        );
        // as-pattern
        let (c, names) = build(&mut env, &[&["all@(x:_)"], &["[]"]]);
        assert_eq!(names[0], vec!["all", "x"]);
        let l = list(vec![Val::Int(7), Val::Int(8)]);
        assert_eq!(
            eval(&env, &c, std::slice::from_ref(&l), &no_guards),
            matched(0, vec![l.clone(), Val::Int(7)])
        );
    }

    #[test]
    fn multi_argument_and_order() {
        let mut env = env();
        // zip
        let (c, names) = build(&mut env, &[&["(x:xs)", "(y:ys)"], &["_", "_"]]);
        assert_eq!(names[0], vec!["x", "xs", "y", "ys"]);
        assert_eq!(
            eval(
                &env,
                &c,
                &[
                    list(vec![Val::Int(1), Val::Int(2)]),
                    list(vec![Val::Int(3)])
                ],
                &no_guards
            ),
            matched(
                0,
                vec![
                    Val::Int(1),
                    list(vec![Val::Int(2)]),
                    Val::Int(3),
                    list(vec![])
                ]
            )
        );
        assert_eq!(
            eval(
                &env,
                &c,
                &[list(vec![]), list(vec![Val::Int(3)])],
                &no_guards
            ),
            matched(1, vec![])
        );
        assert_eq!(
            eval(
                &env,
                &c,
                &[list(vec![Val::Int(3)]), list(vec![])],
                &no_guards
            ),
            matched(1, vec![])
        );
        // first-match semantics with overlapping rows
        let (c, _) = build(
            &mut env,
            &[&["(True, _)"], &["(_, True)"], &["(False, False)"]],
        );
        let t = |a, b| Val::Tuple(vec![Val::Bool(a), Val::Bool(b)]);
        assert_eq!(
            eval(&env, &c, &[t(true, true)], &no_guards),
            matched(0, vec![])
        );
        assert_eq!(
            eval(&env, &c, &[t(false, true)], &no_guards),
            matched(1, vec![])
        );
        assert_eq!(
            eval(&env, &c, &[t(false, false)], &no_guards),
            matched(2, vec![])
        );
        assert_eq!(
            eval(&env, &c, &[Val::Tuple(vec![Val::Bool(true)])], &no_guards),
            Outcome::TypeMismatch(env.lookup_type("(,)").unwrap())
        );
    }

    #[test]
    fn literals_and_default() {
        let mut env = env();
        let (c, _) = build(&mut env, &[&["0"], &["1"], &["n"]]);
        assert_eq!(
            eval(&env, &c, &[Val::Int(0)], &no_guards),
            matched(0, vec![])
        );
        assert_eq!(
            eval(&env, &c, &[Val::Int(1)], &no_guards),
            matched(1, vec![])
        );
        assert_eq!(
            eval(&env, &c, &[Val::Int(9)], &no_guards),
            matched(2, vec![Val::Int(9)])
        );
        assert_eq!(
            eval(&env, &c, &[Val::Str("x".into())], &no_guards),
            Outcome::TypeMismatch(usize::MAX)
        );
        let (c, _) = build(&mut env, &[&["Just \"a\""], &["Just s"], &["Nothing"]]);
        assert_eq!(
            eval(&env, &c, &[just(&env, Val::Str("a".into()))], &no_guards),
            matched(0, vec![])
        );
        assert_eq!(
            eval(&env, &c, &[just(&env, Val::Str("b".into()))], &no_guards),
            matched(1, vec![Val::Str("b".into())])
        );
        // partial function
        let (c, _) = build(&mut env, &[&["Just x"]]);
        assert_eq!(
            eval(&env, &c, &[nothing(&env)], &no_guards),
            Outcome::NoMatch
        );
    }

    #[test]
    fn guards_fall_through() {
        let mut env = env();
        let (c, _) = build(&mut env, &[&["Just x", "|"], &["Just _"], &["Nothing"]]);
        let positive =
            |clause: usize, binds: &[Val]| clause != 0 || matches!(binds[0], Val::Int(i) if i > 0);
        assert_eq!(
            eval(&env, &c, &[just(&env, Val::Int(3))], &positive),
            matched(0, vec![Val::Int(3)])
        );
        assert_eq!(
            eval(&env, &c, &[just(&env, Val::Int(-3))], &positive),
            matched(1, vec![])
        );
        assert_eq!(
            eval(&env, &c, &[nothing(&env)], &positive),
            matched(2, vec![])
        );
        // guard on the last clause with nothing after it
        let (c, _) = build(&mut env, &[&["x", "|"]]);
        assert_eq!(
            eval(&env, &c, &[Val::Int(1)], &|_, _| false),
            Outcome::NoMatch
        );
    }

    #[test]
    fn lazy_patterns() {
        let mut env = env();
        let (c, names) = build(&mut env, &[&["~(Just x)"]]);
        assert_eq!(names[0], vec!["x"]);
        assert_eq!(
            eval(&env, &c, &[just(&env, Val::Int(1))], &no_guards),
            matched(0, vec![Val::Int(1)])
        );
        // irrefutable: matches, but destructuring fails
        assert_eq!(
            eval(&env, &c, &[nothing(&env)], &no_guards),
            Outcome::LazyFailed
        );
        // nested lazy with other binders
        let (c, names) = build(&mut env, &[&["(a, ~(b, c))"]]);
        assert_eq!(names[0], vec!["a", "b", "c"]);
        let v = Val::Tuple(vec![
            Val::Int(1),
            Val::Tuple(vec![Val::Int(2), Val::Int(3)]),
        ]);
        assert_eq!(
            eval(&env, &c, &[v], &no_guards),
            matched(0, vec![Val::Int(1), Val::Int(2), Val::Int(3)])
        );
    }

    #[test]
    fn slots_are_allocated_per_case() {
        let mut env = env();
        let (c, _) = build(
            &mut env,
            &[&["Just (Just x)"], &["Just Nothing"], &["Nothing"]],
        );
        assert!(c.n_slots >= 3);
        match &c.tree {
            Tree::SwitchCon {
                slot: 0,
                cases,
                default: None,
                ..
            } => assert_eq!(cases.len(), 2),
            other => panic!("unexpected tree {:?}", other),
        }
    }
}
