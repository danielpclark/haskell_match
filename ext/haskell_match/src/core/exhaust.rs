//! Exhaustiveness and redundancy analysis (Maranget, "Warnings for pattern
//! matching", JFP 2007).
//!
//! * `useful(P, q)` decides whether row `q` can match a value no row of `P`
//!   matches.  A clause whose pattern is not useful with respect to the
//!   clauses before it is redundant (GHC: `-Woverlapping-patterns`).
//! * `missing(P, n)` enumerates witnesses for values no row matches
//!   (GHC: `-Wincomplete-patterns`, "Patterns not matched: ...").
//!
//! Guarded clauses may fall through, so they are excluded from the matrix
//! when computing what later clauses or the whole function cover, exactly
//! as GHC treats guards other than `otherwise`.

use super::ast::{Lit, LitKey, Pat};
use super::pretty::WPat;
use super::types::{ConId, TypeEnv, TypeId};

type Row = Vec<Pat>;
type Matrix = Vec<Row>;

#[derive(Clone, Debug, PartialEq)]
pub struct Analysis {
    /// Witness vectors (one pattern per argument) for unmatched values.
    pub missing: Vec<Vec<WPat>>,
    /// Whether `missing` was cut off at the requested limit.
    pub truncated: bool,
    /// Indices of clauses that can never be selected.
    pub redundant: Vec<usize>,
}

enum Heads {
    None,
    Cons(TypeId, Vec<ConId>),
    Lits(Vec<Lit>),
}

fn heads(env: &TypeEnv, m: &Matrix) -> Heads {
    let mut cons: Vec<ConId> = Vec::new();
    let mut lits: Vec<Lit> = Vec::new();
    let mut keys: Vec<LitKey> = Vec::new();
    let mut ty = None;
    let mut kind = None;
    for row in m {
        match &row[0] {
            Pat::Con(c, _) => {
                ty = Some(env.type_of_con(*c));
                if !cons.contains(c) {
                    cons.push(*c);
                }
            }
            Pat::Lit(l) => {
                kind = Some(l.kind());
                let k = l.key();
                if !keys.contains(&k) {
                    keys.push(k);
                    lits.push(l.clone());
                }
            }
            _ => {}
        }
    }
    match (ty, kind) {
        (Some(t), _) => Heads::Cons(t, cons),
        (None, Some(_)) => Heads::Lits(lits),
        (None, None) => Heads::None,
    }
}

fn wilds(n: usize) -> impl Iterator<Item = Pat> {
    std::iter::repeat_n(Pat::Wild, n)
}

/// S(c, P): rows that can match constructor `c`, with its fields expanded.
fn specialize_con(env: &TypeEnv, m: &Matrix, c: ConId) -> Matrix {
    let arity = env.con(c).arity;
    let mut out = Vec::new();
    for row in m {
        match &row[0] {
            Pat::Con(cc, args) if *cc == c => {
                let mut r: Row = args.clone();
                r.extend_from_slice(&row[1..]);
                out.push(r);
            }
            Pat::Wild => {
                let mut r: Row = wilds(arity).collect();
                r.extend_from_slice(&row[1..]);
                out.push(r);
            }
            _ => {}
        }
    }
    out
}

fn specialize_lit(m: &Matrix, l: &Lit) -> Matrix {
    let mut out = Vec::new();
    for row in m {
        match &row[0] {
            Pat::Lit(ll) if ll.same(l) => out.push(row[1..].to_vec()),
            Pat::Wild => out.push(row[1..].to_vec()),
            _ => {}
        }
    }
    out
}

/// D(P): rows with a wildcard in the first column, first column dropped.
fn default(m: &Matrix) -> Matrix {
    m.iter()
        .filter(|row| matches!(row[0], Pat::Wild))
        .map(|row| row[1..].to_vec())
        .collect()
}

/// Is `q` useful with respect to `m`?  All patterns must be skeletons.
pub fn useful(env: &TypeEnv, m: &Matrix, q: &[Pat]) -> bool {
    if q.is_empty() {
        return m.is_empty();
    }
    if m.is_empty() {
        return true;
    }
    match &q[0] {
        Pat::Con(c, args) => {
            let mut nq: Vec<Pat> = args.clone();
            nq.extend_from_slice(&q[1..]);
            useful(env, &specialize_con(env, m, *c), &nq)
        }
        Pat::Lit(l) => useful(env, &specialize_lit(m, l), &q[1..]),
        _ => match heads(env, m) {
            Heads::Cons(ty, cs) if env.is_complete(ty, &cs) => cs.iter().any(|c| {
                let mut nq: Vec<Pat> = wilds(env.con(*c).arity).collect();
                nq.extend_from_slice(&q[1..]);
                useful(env, &specialize_con(env, m, *c), &nq)
            }),
            _ => useful(env, &default(m), &q[1..]),
        },
    }
}

struct Collector<'a> {
    env: &'a TypeEnv,
    limit: usize,
    truncated: bool,
}

impl<'a> Collector<'a> {
    fn missing(&mut self, m: &Matrix, n: usize) -> Vec<Vec<WPat>> {
        if n == 0 {
            return if m.is_empty() { vec![vec![]] } else { vec![] };
        }
        let mut out: Vec<Vec<WPat>> = Vec::new();
        match heads(self.env, m) {
            Heads::Cons(ty, cs) => {
                // Constructors absent from the column are unmatched outright
                // (combined with whatever the wildcard rows leave unmatched);
                // constructors present in the column may still be incomplete
                // in their fields or in the remaining columns.
                let dflt = if self.env.is_complete(ty, &cs) {
                    None
                } else {
                    Some(self.missing(&default(m), n - 1))
                };
                let all: Vec<ConId> = self.env.ty(ty).cons.clone();
                'cons: for c in all {
                    let arity = self.env.con(c).arity;
                    if cs.contains(&c) {
                        let sub = self.missing(&specialize_con(self.env, m, c), arity + n - 1);
                        for w in sub {
                            if out.len() >= self.limit {
                                self.truncated = true;
                                break 'cons;
                            }
                            let (args, rest) = w.split_at(arity);
                            let mut row = vec![WPat::Con(c, args.to_vec())];
                            row.extend_from_slice(rest);
                            out.push(row);
                        }
                    } else if let Some(dflt) = &dflt {
                        for w in dflt {
                            if out.len() >= self.limit {
                                self.truncated = true;
                                break 'cons;
                            }
                            let mut row = vec![WPat::Con(
                                c,
                                std::iter::repeat_n(WPat::Wild, arity).collect(),
                            )];
                            row.extend_from_slice(w);
                            out.push(row);
                        }
                    }
                }
            }
            Heads::Lits(lits) => {
                'lits: for l in &lits {
                    let sub = self.missing(&specialize_lit(m, l), n - 1);
                    for w in sub {
                        if out.len() >= self.limit {
                            self.truncated = true;
                            break 'lits;
                        }
                        let mut row = vec![WPat::Lit(l.clone())];
                        row.extend_from_slice(&w);
                        out.push(row);
                    }
                }
                let sub = self.missing(&default(m), n - 1);
                for w in sub {
                    if out.len() >= self.limit {
                        self.truncated = true;
                        break;
                    }
                    let mut row = vec![WPat::NotLit(lits.clone())];
                    row.extend_from_slice(&w);
                    out.push(row);
                }
            }
            Heads::None => {
                let sub = self.missing(&default(m), n - 1);
                for w in sub {
                    if out.len() >= self.limit {
                        self.truncated = true;
                        break;
                    }
                    let mut row = vec![WPat::Wild];
                    row.extend_from_slice(&w);
                    out.push(row);
                }
            }
        }
        out
    }
}

/// Analyse a function's clauses.  `clauses` holds the (already resolved)
/// argument patterns of each clause and whether it carries a guard.
pub fn analyze(env: &TypeEnv, clauses: &[(Vec<Pat>, bool)], limit: usize) -> Analysis {
    let arity = clauses.first().map(|c| c.0.len()).unwrap_or(0);
    let skeletons: Vec<Row> = clauses
        .iter()
        .map(|(pats, _)| pats.iter().map(Pat::skeleton).collect())
        .collect();

    // redundancy: each clause against the unguarded clauses before it
    let mut redundant = Vec::new();
    let mut covering: Matrix = Vec::new();
    for (i, (_, guarded)) in clauses.iter().enumerate() {
        if !useful(env, &covering, &skeletons[i]) {
            redundant.push(i);
        }
        if !guarded {
            covering.push(skeletons[i].clone());
        }
    }

    // Ask for one witness more than the caller wants so an exact hit on the
    // limit is distinguishable from a cut-off list.
    let limit = limit.max(1);
    let mut col = Collector {
        env,
        limit: limit + 1,
        truncated: false,
    };
    let mut missing = col.missing(&covering, arity);
    let truncated = col.truncated || missing.len() > limit;
    missing.truncate(limit);
    Analysis {
        missing,
        truncated,
        redundant,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::parser::parse_pattern;
    use crate::core::pretty::render_witness;
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
        env.register(
            "Shape",
            &[
                ConSpec {
                    name: "Circle".into(),
                    arity: 1,
                    fields: None,
                    handle: 0,
                },
                ConSpec {
                    name: "Rect".into(),
                    arity: 2,
                    fields: None,
                    handle: 0,
                },
                ConSpec {
                    name: "Tri".into(),
                    arity: 3,
                    fields: None,
                    handle: 0,
                },
            ],
        )
        .unwrap();
        env
    }

    /// Clause spec: list of pattern strings; a trailing "|" marks a guard.
    fn run(env: &mut TypeEnv, clauses: &[&[&str]]) -> (Vec<String>, Vec<usize>, bool) {
        let mut resolved = Vec::new();
        for c in clauses {
            let guarded = c.last() == Some(&"|");
            let pats: Vec<&str> = c.iter().copied().filter(|s| *s != "|").collect();
            let raws: Vec<_> = pats.iter().map(|s| parse_pattern(s).unwrap()).collect();
            let (ps, _) = resolve_clause(env, &raws).unwrap();
            resolved.push((ps, guarded));
        }
        let a = analyze(env, &resolved, 50);
        let missing = a.missing.iter().map(|w| render_witness(env, w)).collect();
        (missing, a.redundant, a.truncated)
    }

    #[test]
    fn exhaustive_functions() {
        let mut env = env();
        let empty: Vec<String> = vec![];
        assert_eq!(
            run(&mut env, &[&["Nothing"], &["Just _"]]),
            (empty.clone(), vec![], false)
        );
        assert_eq!(
            run(&mut env, &[&["[]"], &["(x:xs)"]]),
            (empty.clone(), vec![], false)
        );
        assert_eq!(
            run(&mut env, &[&["[]"], &["[x]"], &["(x:y:rest)"]]),
            (empty.clone(), vec![], false)
        );
        assert_eq!(
            run(&mut env, &[&["True"], &["False"]]),
            (empty.clone(), vec![], false)
        );
        assert_eq!(
            run(&mut env, &[&["0"], &["n"]]),
            (empty.clone(), vec![], false)
        );
        assert_eq!(
            run(&mut env, &[&["(a, b)"]]),
            (empty.clone(), vec![], false)
        );
        assert_eq!(run(&mut env, &[&["()"]]), (empty.clone(), vec![], false));
        assert_eq!(
            run(&mut env, &[&["_", "_"]]),
            (empty.clone(), vec![], false)
        );
        // multi-argument zip
        assert_eq!(
            run(&mut env, &[&["(x:xs)", "(y:ys)"], &["_", "_"]]),
            (empty.clone(), vec![], false)
        );
        // guard followed by catch-all
        assert_eq!(
            run(&mut env, &[&["Just x", "|"], &["Just _"], &["Nothing"]]),
            (empty.clone(), vec![], false)
        );
        // lazy and as patterns are irrefutable / transparent
        assert_eq!(
            run(&mut env, &[&["~(Just x)"]]),
            (empty.clone(), vec![], false)
        );
        assert_eq!(
            run(&mut env, &[&["a@(Just x)"], &["Nothing"]]),
            (empty, vec![], false)
        );
    }

    #[test]
    fn missing_witnesses() {
        let mut env = env();
        assert_eq!(run(&mut env, &[&["Just _"]]).0, vec!["Nothing"]);
        assert_eq!(
            run(&mut env, &[&["Nothing"], &["Just Nothing"]]).0,
            vec!["Just (Just _)"]
        );
        assert_eq!(run(&mut env, &[&["(x:xs)"]]).0, vec!["[]"]);
        assert_eq!(run(&mut env, &[&["[]"], &["[x]"]]).0, vec!["(_:_:_)"]);
        assert_eq!(run(&mut env, &[&["[]"], &["(x:y:rest)"]]).0, vec!["[_]"]);
        assert_eq!(run(&mut env, &[&["True"]]).0, vec!["False"]);
        assert_eq!(
            run(&mut env, &[&["0"], &["1"]]).0,
            vec!["p1 where p1 is not one of {0, 1}"]
        );
        assert_eq!(
            run(&mut env, &[&["\"a\""]]).0,
            vec!["[]", "('a':_:_)", "(p1:_) where p1 is not one of {'a'}"]
        );
        assert_eq!(
            run(&mut env, &[&["\"\""], &["(c:cs)"]]).0,
            Vec::<String>::new()
        );
        assert_eq!(
            run(&mut env, &[&["'a'"]]).0,
            vec!["p1 where p1 is not one of {'a'}"]
        );
        assert_eq!(
            run(&mut env, &[&["Circle _"]]).0,
            vec!["Rect _ _", "Tri _ _ _"]
        );
        assert_eq!(
            run(&mut env, &[&["(True, Nothing)"], &["(False, _)"]]).0,
            vec!["(True, Just _)"]
        );
        assert_eq!(
            run(&mut env, &[&["(x:xs)", "(y:ys)"], &["[]", "[]"]]).0,
            vec!["[] (_:_)", "(_:_) []"]
        );
        assert_eq!(
            run(&mut env, &[&["Circle _", "Circle _"]]).0,
            vec![
                "(Circle _) (Rect _ _)",
                "(Circle _) (Tri _ _ _)",
                "(Rect _ _) _",
                "(Tri _ _ _) _"
            ]
        );
        assert_eq!(
            run(&mut env, &[&["0", "[]"], &["n", "(x:xs)"]]).0,
            vec!["p1 [] where p1 is not one of {0}"]
        );
        assert_eq!(
            run(&mut env, &[&["0", "[]"], &["n", "[]"]]).0,
            vec!["0 (_:_)", "p1 (_:_) where p1 is not one of {0}"]
        );
        assert_eq!(
            run(&mut env, &[&["0", "[]"], &["1", "_"]]).0,
            vec!["0 (_:_)", "p1 _ where p1 is not one of {0, 1}"]
        );
        assert_eq!(
            run(&mut env, &[&["Just 0"], &["Just n"]]).0,
            vec!["Nothing"]
        );
        // a guard alone does not cover
        assert_eq!(
            run(&mut env, &[&["Just x", "|"], &["Nothing"]]).0,
            vec!["Just _"]
        );
        // nested literal inside constructor
        assert_eq!(
            run(&mut env, &[&["Just 0"], &["Nothing"]]).0,
            vec!["Just p1 where p1 is not one of {0}"]
        );
        // nothing at all
        let r = run(&mut env, &[]);
        assert_eq!(r.0, vec![""]);
    }

    #[test]
    fn redundant_clauses() {
        let mut env = env();
        assert_eq!(run(&mut env, &[&["_"], &["Just x"]]).1, vec![1]);
        assert_eq!(
            run(&mut env, &[&["Nothing"], &["Just _"], &["Just 0"]]).1,
            vec![2]
        );
        assert_eq!(
            run(&mut env, &[&["[]"], &["(x:xs)"], &["[a, b]"], &["_"]]).1,
            vec![2, 3]
        );
        assert_eq!(run(&mut env, &[&["0"], &["0"]]).1, vec![1]);
        assert_eq!(run(&mut env, &[&["x"], &["0"]]).1, vec![1]);
        // guarded clause does not make a later identical clause redundant...
        assert_eq!(run(&mut env, &[&["Just x", "|"], &["Just x"]]).1, vec![]);
        // ...but is itself redundant after an unguarded cover
        assert_eq!(
            run(&mut env, &[&["Just _"], &["Just x", "|"], &["Nothing"]]).1,
            vec![1]
        );
        assert_eq!(
            run(
                &mut env,
                &[
                    &["(True, _)"],
                    &["(_, True)"],
                    &["(False, False)"],
                    &["(True, True)"]
                ]
            )
            .1,
            vec![3]
        );
        assert_eq!(
            run(
                &mut env,
                &[
                    &["(x:xs)", "_"],
                    &["_", "(y:ys)"],
                    &["[]", "[]"],
                    &["[]", "_"]
                ]
            )
            .1,
            vec![3]
        );
    }

    #[test]
    fn witness_limit() {
        let mut env = env();
        // 4 witnesses (see missing_witnesses); limit 3 truncates
        let mut resolved = Vec::new();
        let raws = vec![
            parse_pattern("Circle _").unwrap(),
            parse_pattern("Circle _").unwrap(),
        ];
        resolved.push((resolve_clause(&mut env, &raws).unwrap().0, false));
        let a = analyze(&env, &resolved, 3);
        assert_eq!(a.missing.len(), 3);
        assert!(a.truncated);
        let a = analyze(&env, &resolved, 100);
        assert_eq!(a.missing.len(), 4);
        assert!(!a.truncated);
    }
}
