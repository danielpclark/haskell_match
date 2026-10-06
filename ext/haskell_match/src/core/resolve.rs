//! Resolution of `RawPat` into `Pat`: constructor lookup, arity checking,
//! record field handling, variable numbering and duplicate detection.

use super::ast::{Pat, RawPat, VarId};
use super::error::{CoreError, ErrorKind, Result};
use super::types::{TypeEnv, CON_CONS, CON_FALSE, CON_NIL, CON_TRUE};

/// Per-clause binding table.  Variables are numbered in order of first
/// appearance, left to right across all argument patterns of the clause.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bindings {
    pub names: Vec<String>,
}

impl Bindings {
    fn bind(&mut self, name: &str) -> Result<VarId> {
        if self.names.iter().any(|n| n == name) {
            return Err(CoreError::new(
                ErrorKind::DuplicateVariable,
                format!("conflicting definitions for '{}' in the same clause", name),
            ));
        }
        self.names.push(name.to_string());
        Ok(self.names.len() - 1)
    }
}

/// Resolve one clause consisting of `arity` argument patterns.
pub fn resolve_clause(env: &mut TypeEnv, raws: &[RawPat]) -> Result<(Vec<Pat>, Bindings)> {
    let mut b = Bindings::default();
    let mut pats = Vec::with_capacity(raws.len());
    for raw in raws {
        pats.push(resolve(env, raw, &mut b)?);
    }
    Ok((pats, b))
}

fn resolve(env: &mut TypeEnv, raw: &RawPat, b: &mut Bindings) -> Result<Pat> {
    Ok(match raw {
        RawPat::Wild => Pat::Wild,
        RawPat::Var(v) => Pat::Var(b.bind(v)?),
        RawPat::As(v, p) => {
            let id = b.bind(v)?;
            Pat::As(id, Box::new(resolve(env, p, b)?))
        }
        RawPat::Lazy(p) => {
            let inner = resolve(env, p, b)?;
            // `~_`, `~x` are already irrefutable; keep the tree small
            match inner {
                Pat::Wild | Pat::Var(_) | Pat::Lazy(_) => inner,
                other => Pat::Lazy(Box::new(other)),
            }
        }
        RawPat::Bang(p) => resolve(env, p, b)?,
        RawPat::Lit(l) => Pat::Lit(l.clone()),
        RawPat::Tuple(items) => {
            let con = env.tuple_con(items.len());
            let mut args = Vec::with_capacity(items.len());
            for it in items {
                args.push(resolve(env, it, b)?);
            }
            Pat::Con(con, args)
        }
        RawPat::List(items) => {
            let mut acc = Pat::Con(CON_NIL, vec![]);
            let mut resolved = Vec::with_capacity(items.len());
            for it in items {
                resolved.push(resolve(env, it, b)?);
            }
            for p in resolved.into_iter().rev() {
                acc = Pat::Con(CON_CONS, vec![p, acc]);
            }
            acc
        }
        RawPat::Cons(h, t) => {
            let h = resolve(env, h, b)?;
            let t = resolve(env, t, b)?;
            Pat::Con(CON_CONS, vec![h, t])
        }
        RawPat::Con(name, args) => {
            let con = lookup(env, name)?;
            let arity = env.con(con).arity;
            if arity != args.len() {
                return Err(CoreError::new(
                    ErrorKind::Arity,
                    format!(
                        "the constructor '{}' should have {} argument{}, but has been given {}",
                        name,
                        arity,
                        if arity == 1 { "" } else { "s" },
                        args.len()
                    ),
                ));
            }
            let mut rargs = Vec::with_capacity(args.len());
            for a in args {
                rargs.push(resolve(env, a, b)?);
            }
            Pat::Con(con, rargs)
        }
        RawPat::Hash(fields) => {
            let mut out = Vec::with_capacity(fields.len());
            for (k, p) in fields {
                out.push((k.clone(), resolve(env, p, b)?));
            }
            Pat::Hash(out)
        }
        RawPat::Record(name, fields, wildcard) => {
            let con = lookup(env, name)?;
            let (arity, field_names) = {
                let c = env.con(con);
                (c.arity, c.fields.clone())
            };
            let mut args = vec![None; arity];
            if !fields.is_empty() || *wildcard {
                let field_names = field_names.ok_or_else(|| {
                    CoreError::new(
                        ErrorKind::Field,
                        format!("constructor '{}' does not have named fields", name),
                    )
                })?;
                for (f, p) in fields {
                    let idx = field_names.iter().position(|n| n == f).ok_or_else(|| {
                        CoreError::new(
                            ErrorKind::Field,
                            format!(
                                "constructor '{}' does not have a field named '{}' (fields: {})",
                                name,
                                f,
                                field_names.join(", ")
                            ),
                        )
                    })?;
                    args[idx] = Some(resolve(env, p, b)?);
                }
                if *wildcard {
                    // RecordWildCards: bind every remaining field to its own name
                    for (i, slot) in args.iter_mut().enumerate() {
                        if slot.is_none() {
                            *slot = Some(Pat::Var(b.bind(&field_names[i])?));
                        }
                    }
                }
            }
            Pat::Con(
                con,
                args.into_iter().map(|a| a.unwrap_or(Pat::Wild)).collect(),
            )
        }
    })
}

fn lookup(env: &TypeEnv, name: &str) -> Result<super::types::ConId> {
    match name {
        "True" => return Ok(CON_TRUE),
        "False" => return Ok(CON_FALSE),
        _ => {}
    }
    env.lookup_con(name).ok_or_else(|| {
        CoreError::new(
            ErrorKind::UnknownConstructor,
            format!("not in scope: data constructor '{}'", name),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ast::Lit;
    use crate::core::parser::parse_pattern;
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
            "Person",
            &[ConSpec {
                name: "Person".into(),
                arity: 2,
                fields: Some(vec!["name".into(), "age".into()]),
                handle: 0,
            }],
        )
        .unwrap();
        env
    }

    fn res(env: &mut TypeEnv, srcs: &[&str]) -> Result<(Vec<Pat>, Bindings)> {
        let mut raws: Vec<RawPat> = Vec::new();
        for s in srcs {
            raws.push(parse_pattern(s)?);
        }
        resolve_clause(env, &raws)
    }

    #[test]
    fn resolves_constructors_and_sugar() {
        let mut env = env();
        let just = env.lookup_con("Just").unwrap();
        let (pats, b) = res(&mut env, &["Just x"]).unwrap();
        assert_eq!(pats, vec![Pat::Con(just, vec![Pat::Var(0)])]);
        assert_eq!(b.names, vec!["x"]);

        let (pats, b) = res(&mut env, &["[a, b]"]).unwrap();
        assert_eq!(
            pats[0],
            Pat::Con(
                CON_CONS,
                vec![
                    Pat::Var(0),
                    Pat::Con(CON_CONS, vec![Pat::Var(1), Pat::Con(CON_NIL, vec![])])
                ]
            )
        );
        assert_eq!(b.names, vec!["a", "b"]);

        let t2 = env.tuple_con(2);
        let (pats, _) = res(&mut env, &["(True, False)"]).unwrap();
        assert_eq!(
            pats[0],
            Pat::Con(
                t2,
                vec![Pat::Con(CON_TRUE, vec![]), Pat::Con(CON_FALSE, vec![])]
            )
        );

        let (pats, b) = res(&mut env, &["all@(x:xs)", "~(Just y)", "!z"]).unwrap();
        assert_eq!(b.names, vec!["all", "x", "xs", "y", "z"]);
        assert_eq!(
            pats[0],
            Pat::As(
                0,
                Box::new(Pat::Con(CON_CONS, vec![Pat::Var(1), Pat::Var(2)]))
            )
        );
        assert_eq!(
            pats[1],
            Pat::Lazy(Box::new(Pat::Con(just, vec![Pat::Var(3)])))
        );
        assert_eq!(pats[2], Pat::Var(4));
        // ~x is just x
        let (pats, _) = res(&mut env, &["~x"]).unwrap();
        assert_eq!(pats[0], Pat::Var(0));
    }

    #[test]
    fn records() {
        let mut env = env();
        let person = env.lookup_con("Person").unwrap();
        let (pats, b) = res(&mut env, &["Person { age = a }"]).unwrap();
        assert_eq!(pats[0], Pat::Con(person, vec![Pat::Wild, Pat::Var(0)]));
        assert_eq!(b.names, vec!["a"]);
        let (pats, b) = res(&mut env, &["Person { name, .. }"]).unwrap();
        assert_eq!(pats[0], Pat::Con(person, vec![Pat::Var(0), Pat::Var(1)]));
        assert_eq!(b.names, vec!["name", "age"]);
        let (pats, _) = res(&mut env, &["Person {}"]).unwrap();
        assert_eq!(pats[0], Pat::Con(person, vec![Pat::Wild, Pat::Wild]));
        // `Con {}` is allowed for positional constructors too
        let (pats, _) = res(&mut env, &["Just {}"]).unwrap();
        assert_eq!(
            pats[0],
            Pat::Con(env.lookup_con("Just").unwrap(), vec![Pat::Wild])
        );
        let (pats, _) = res(&mut env, &["Person 'b' 3"]).unwrap();
        assert_eq!(
            pats[0],
            Pat::Con(
                person,
                vec![Pat::Lit(Lit::Char('b')), Pat::Lit(Lit::Int(3))]
            )
        );
    }

    #[test]
    fn errors() {
        let mut env = env();
        assert_eq!(
            res(&mut env, &["Nope x"]).unwrap_err().kind,
            ErrorKind::UnknownConstructor
        );
        assert_eq!(res(&mut env, &["Just"]).unwrap_err().kind, ErrorKind::Arity);
        assert_eq!(
            res(&mut env, &["Just a b"]).unwrap_err().kind,
            ErrorKind::Arity
        );
        assert_eq!(
            res(&mut env, &["Nothing x"]).unwrap_err().kind,
            ErrorKind::Arity
        );
        assert_eq!(
            res(&mut env, &["(x, x)"]).unwrap_err().kind,
            ErrorKind::DuplicateVariable
        );
        assert_eq!(
            res(&mut env, &["x", "Just x"]).unwrap_err().kind,
            ErrorKind::DuplicateVariable
        );
        assert_eq!(
            res(&mut env, &["Person { nome = n }"]).unwrap_err().kind,
            ErrorKind::Field
        );
        assert_eq!(
            res(&mut env, &["Just { value = v }"]).unwrap_err().kind,
            ErrorKind::Field
        );
        assert_eq!(
            res(&mut env, &["Just { .. }"]).unwrap_err().kind,
            ErrorKind::Field
        );
        assert_eq!(
            res(&mut env, &["Person { name = n, name = m }"])
                .unwrap_err()
                .kind,
            ErrorKind::Field
        );
    }
}
