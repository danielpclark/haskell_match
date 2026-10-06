//! Structural type checking of a clause matrix.
//!
//! Ruby values are untyped, so we infer a "shape type" for every position of
//! the patterns and require all clauses to agree: a position matched against
//! `Just _` in one clause cannot be matched against `(x:xs)` or `0` in
//! another.  This mirrors the Haskell type checker's rejection of such a
//! function and is what makes the exhaustiveness check meaningful.

use std::collections::HashMap;

use super::ast::{HKey, LitKind, Pat};
use super::error::{CoreError, ErrorKind, Result};
use super::types::{ConId, TypeEnv, TypeId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColType {
    Con(TypeId),
    Lit(LitKind),
    Hash,
}

fn describe(env: &TypeEnv, t: ColType) -> String {
    match t {
        ColType::Con(ty) => env.ty(ty).name.clone(),
        ColType::Lit(k) => k.name().to_string(),
        ColType::Hash => "Hash".to_string(),
    }
}

/// One step into a sub-pattern: a constructor field or a Hash key.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Step {
    Field(ConId, usize),
    Key(HKey),
}

/// Position of a sub-pattern: the argument index followed by the chain of
/// steps taken to reach it.
type Path = (usize, Vec<Step>);

fn describe_path(env: &TypeEnv, path: &Path) -> String {
    let mut s = format!("argument {}", path.0 + 1);
    for step in &path.1 {
        match step {
            Step::Field(con, field) => {
                let c = env.con(*con);
                let fname = match &c.fields {
                    Some(fs) => format!("field '{}'", fs[*field]),
                    None => format!("field {}", field + 1),
                };
                s.push_str(&format!(", {} of '{}'", fname, c.name));
            }
            Step::Key(k) => s.push_str(&format!(", key {} of the Hash", k)),
        }
    }
    s
}

/// Check that every position has a single type across all clauses.
pub fn check(env: &TypeEnv, clauses: &[Vec<Pat>]) -> Result<()> {
    let mut seen: HashMap<Path, (ColType, usize)> = HashMap::new();
    for (ci, clause) in clauses.iter().enumerate() {
        for (ai, pat) in clause.iter().enumerate() {
            let mut path: Path = (ai, Vec::new());
            walk(env, pat, &mut path, ci, &mut seen)?;
        }
    }
    Ok(())
}

fn walk(
    env: &TypeEnv,
    pat: &Pat,
    path: &mut Path,
    clause: usize,
    seen: &mut HashMap<Path, (ColType, usize)>,
) -> Result<()> {
    match pat {
        Pat::Wild | Pat::Var(_) => Ok(()),
        Pat::As(_, p) | Pat::Lazy(p) => walk(env, p, path, clause, seen),
        Pat::Lit(l) => unify(env, path, ColType::Lit(l.kind()), clause, seen),
        Pat::Con(c, args) => {
            unify(env, path, ColType::Con(env.type_of_con(*c)), clause, seen)?;
            for (i, a) in args.iter().enumerate() {
                path.1.push(Step::Field(*c, i));
                let r = walk(env, a, path, clause, seen);
                path.1.pop();
                r?;
            }
            Ok(())
        }
        Pat::Hash(fields) => {
            unify(env, path, ColType::Hash, clause, seen)?;
            for (k, a) in fields {
                path.1.push(Step::Key(k.clone()));
                let r = walk(env, a, path, clause, seen);
                path.1.pop();
                r?;
            }
            Ok(())
        }
    }
}

fn unify(
    env: &TypeEnv,
    path: &Path,
    t: ColType,
    clause: usize,
    seen: &mut HashMap<Path, (ColType, usize)>,
) -> Result<()> {
    match seen.get(path) {
        None => {
            seen.insert(path.clone(), (t, clause));
            Ok(())
        }
        Some(&(prev, prev_clause)) if prev == t => {
            let _ = prev_clause;
            Ok(())
        }
        Some(&(prev, prev_clause)) => Err(CoreError::new(
            ErrorKind::Type,
            format!(
                "couldn't match type '{}' (clause {}) with '{}' (clause {}) at {}",
                describe(env, prev),
                prev_clause + 1,
                describe(env, t),
                clause + 1,
                describe_path(env, path)
            ),
        )),
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

    fn clauses(env: &mut TypeEnv, srcs: &[&[&str]]) -> Vec<Vec<Pat>> {
        srcs.iter()
            .map(|c| {
                let raws: Vec<_> = c.iter().map(|s| parse_pattern(s).unwrap()).collect();
                resolve_clause(env, &raws).unwrap().0
            })
            .collect()
    }

    #[test]
    fn accepts_consistent_columns() {
        let mut env = env();
        let cs = clauses(
            &mut env,
            &[&["Just 0", "[]"], &["Just n", "(x:xs)"], &["Nothing", "_"]],
        );
        check(&env, &cs).unwrap();
        let cs = clauses(
            &mut env,
            &[&["(Just \"a\", True)"], &["(_, False)"], &["(Nothing, _)"]],
        );
        check(&env, &cs).unwrap();
        let cs = clauses(&mut env, &[&["0"], &["1.5"], &["-3"], &["n"]]);
        check(&env, &cs).unwrap();
    }

    #[test]
    fn rejects_mixed_columns() {
        let mut env = env();
        let cs = clauses(&mut env, &[&["Just x"], &["(x:xs)"]]);
        let e = check(&env, &cs).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Type);
        assert!(
            e.message
                .contains("'Maybe' (clause 1) with '[]' (clause 2) at argument 1"),
            "{}",
            e.message
        );

        let cs = clauses(&mut env, &[&["Just 0"], &["Just 's'"]]);
        let e = check(&env, &cs).unwrap_err();
        assert!(e.message.contains("field 1 of 'Just'"), "{}", e.message);
        // Char and String ([Char]) are different types, as in Haskell
        let cs = clauses(&mut env, &[&["'a'"], &["\"a\""]]);
        assert!(check(&env, &cs).is_err());
        // but string literals unify with list patterns
        let cs = clauses(&mut env, &[&["\"\""], &["(c:cs)"]]);
        check(&env, &cs).unwrap();
        let cs = clauses(&mut env, &[&["\"yes\""], &["['n', _]"], &["_"]]);
        check(&env, &cs).unwrap();

        let cs = clauses(&mut env, &[&["(a, b)"], &["[]"]]);
        assert!(check(&env, &cs).is_err());
        let cs = clauses(&mut env, &[&["(a, b)"], &["(a, b, c)"]]);
        assert!(check(&env, &cs).is_err());
        let cs = clauses(&mut env, &[&["True"], &["0"]]);
        assert!(check(&env, &cs).is_err());
        let cs = clauses(&mut env, &[&["x", "Just y"], &["Nothing", "[]"]]);
        let e = check(&env, &cs).unwrap_err();
        assert!(e.message.contains("argument 2"), "{}", e.message);
    }
}
