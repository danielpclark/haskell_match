//! The type environment: algebraic data types and their constructors.
//!
//! Built-in types `Bool`, lists and tuples are registered up front so the
//! exhaustiveness checker knows their complete constructor signatures.
//! User types are added through `register`.

use std::collections::HashMap;

use super::error::{CoreError, ErrorKind, Result};

pub type TypeId = usize;
pub type ConId = usize;

pub const BOOL: TypeId = 0;
pub const LIST: TypeId = 1;

pub const CON_FALSE: ConId = 0;
pub const CON_TRUE: ConId = 1;
pub const CON_NIL: ConId = 2;
pub const CON_CONS: ConId = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypeKind {
    /// User-defined algebraic data type.  Values are instances of a Ruby class
    /// per constructor.
    Adt,
    /// `true` / `false`.
    Bool,
    /// Ruby `Array` viewed as a cons list.
    List,
    /// Ruby `Array` of exactly `n` elements (`Tuple(0)` is unit `()`).
    Tuple(usize),
}

#[derive(Clone, Debug)]
pub struct DataType {
    pub name: String,
    pub kind: TypeKind,
    pub cons: Vec<ConId>,
    /// `false` once the type has been replaced by a redefinition.
    pub live: bool,
}

#[derive(Clone, Debug)]
pub struct Constructor {
    pub name: String,
    pub ty: TypeId,
    /// Index within the owning type's constructor list.
    pub tag: usize,
    pub arity: usize,
    /// Field names for record constructors.
    pub fields: Option<Vec<String>>,
    /// Opaque host handle (the Ruby class, as a raw `VALUE`) for ADT
    /// constructors; `0` for built-ins.
    pub handle: usize,
}

#[derive(Clone, Debug)]
pub struct ConSpec {
    pub name: String,
    pub arity: usize,
    pub fields: Option<Vec<String>>,
    pub handle: usize,
}

#[derive(Clone, Debug)]
pub struct TypeEnv {
    types: Vec<DataType>,
    cons: Vec<Constructor>,
    by_con_name: HashMap<String, ConId>,
    by_type_name: HashMap<String, TypeId>,
    tuples: HashMap<usize, TypeId>,
}

impl Default for TypeEnv {
    fn default() -> Self {
        Self::new()
    }
}

impl TypeEnv {
    pub fn new() -> Self {
        let mut env = TypeEnv {
            types: Vec::new(),
            cons: Vec::new(),
            by_con_name: HashMap::new(),
            by_type_name: HashMap::new(),
            tuples: HashMap::new(),
        };
        let b = env.add_type("Bool", TypeKind::Bool);
        debug_assert_eq!(b, BOOL);
        let f = env.add_con(b, "False", 0, None, 0);
        let t = env.add_con(b, "True", 0, None, 0);
        debug_assert_eq!((f, t), (CON_FALSE, CON_TRUE));
        let l = env.add_type("[]", TypeKind::List);
        debug_assert_eq!(l, LIST);
        let n = env.add_con(l, "[]", 0, None, 0);
        let c = env.add_con(l, ":", 2, None, 0);
        debug_assert_eq!((n, c), (CON_NIL, CON_CONS));
        env
    }

    fn add_type(&mut self, name: &str, kind: TypeKind) -> TypeId {
        let id = self.types.len();
        self.types.push(DataType {
            name: name.to_string(),
            kind,
            cons: Vec::new(),
            live: true,
        });
        self.by_type_name.insert(name.to_string(), id);
        id
    }

    fn add_con(
        &mut self,
        ty: TypeId,
        name: &str,
        arity: usize,
        fields: Option<Vec<String>>,
        handle: usize,
    ) -> ConId {
        let id = self.cons.len();
        let tag = self.types[ty].cons.len();
        self.cons.push(Constructor {
            name: name.to_string(),
            ty,
            tag,
            arity,
            fields,
            handle,
        });
        self.types[ty].cons.push(id);
        self.by_con_name.insert(name.to_string(), id);
        id
    }

    /// Register (or re-register) a user data type.  Re-registering a type with
    /// the same name replaces its constructors; constructor names must not be
    /// in use by another live type.
    pub fn register(&mut self, name: &str, cons: &[ConSpec]) -> Result<TypeId> {
        if cons.is_empty() {
            return Err(CoreError::new(
                ErrorKind::DataDeclaration,
                format!("type '{}' must have at least one constructor", name),
            ));
        }
        if name == "Bool" || name == "[]" {
            return Err(CoreError::new(
                ErrorKind::DataDeclaration,
                format!("cannot redefine built-in type '{}'", name),
            ));
        }
        for (i, c) in cons.iter().enumerate() {
            if !c
                .name
                .chars()
                .next()
                .map(|ch| ch.is_uppercase())
                .unwrap_or(false)
            {
                return Err(CoreError::new(
                    ErrorKind::DataDeclaration,
                    format!(
                        "constructor name '{}' must start with an upper-case letter",
                        c.name
                    ),
                ));
            }
            if cons[..i].iter().any(|d| d.name == c.name) {
                return Err(CoreError::new(
                    ErrorKind::DataDeclaration,
                    format!(
                        "constructor '{}' is declared twice in type '{}'",
                        c.name, name
                    ),
                ));
            }
            if let Some(fields) = &c.fields {
                if fields.len() != c.arity {
                    return Err(CoreError::new(
                        ErrorKind::DataDeclaration,
                        format!(
                            "constructor '{}' has {} fields but arity {}",
                            c.name,
                            fields.len(),
                            c.arity
                        ),
                    ));
                }
            }
            if let Some(&existing) = self.by_con_name.get(&c.name) {
                let owner = self.cons[existing].ty;
                if self.types[owner].name != name && self.types[owner].live {
                    return Err(CoreError::new(
                        ErrorKind::DataDeclaration,
                        format!(
                            "constructor '{}' is already defined by type '{}'",
                            c.name, self.types[owner].name
                        ),
                    ));
                }
            }
        }
        // retire a previous definition with this name
        if let Some(&old) = self.by_type_name.get(name) {
            let old_cons = self.types[old].cons.clone();
            for cid in old_cons {
                if self.by_con_name.get(&self.cons[cid].name) == Some(&cid) {
                    self.by_con_name.remove(&self.cons[cid].name);
                }
            }
            self.types[old].live = false;
        }
        let ty = self.add_type(name, TypeKind::Adt);
        for c in cons {
            self.add_con(ty, &c.name, c.arity, c.fields.clone(), c.handle);
        }
        Ok(ty)
    }

    pub fn tuple(&mut self, n: usize) -> TypeId {
        if let Some(&t) = self.tuples.get(&n) {
            return t;
        }
        let name = if n == 0 {
            "()".to_string()
        } else {
            format!("({})", ",".repeat(n - 1))
        };
        let ty = self.add_type(&name, TypeKind::Tuple(n));
        self.add_con(ty, &name, n, None, 0);
        self.tuples.insert(n, ty);
        ty
    }

    /// The single constructor of the `n`-tuple type (creating the type if
    /// needed).
    pub fn tuple_con(&mut self, n: usize) -> ConId {
        let ty = self.tuple(n);
        self.types[ty].cons[0]
    }

    pub fn lookup_con(&self, name: &str) -> Option<ConId> {
        self.by_con_name.get(name).copied()
    }

    pub fn lookup_type(&self, name: &str) -> Option<TypeId> {
        self.by_type_name
            .get(name)
            .copied()
            .filter(|&t| self.types[t].live)
    }

    pub fn con(&self, id: ConId) -> &Constructor {
        &self.cons[id]
    }

    pub fn ty(&self, id: TypeId) -> &DataType {
        &self.types[id]
    }

    pub fn type_of_con(&self, id: ConId) -> TypeId {
        self.cons[id].ty
    }

    /// Constructor with `tag` within type `ty`.
    pub fn con_by_tag(&self, ty: TypeId, tag: usize) -> ConId {
        self.types[ty].cons[tag]
    }

    pub fn is_complete(&self, ty: TypeId, present: &[ConId]) -> bool {
        self.types[ty].cons.iter().all(|c| present.contains(c))
    }

    pub fn types_len(&self) -> usize {
        self.types.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(name: &str, arity: usize) -> ConSpec {
        ConSpec {
            name: name.into(),
            arity,
            fields: None,
            handle: 0,
        }
    }

    #[test]
    fn builtins() {
        let mut env = TypeEnv::new();
        assert_eq!(env.lookup_con("True"), Some(CON_TRUE));
        assert_eq!(env.lookup_con("[]"), Some(CON_NIL));
        assert_eq!(env.con(CON_CONS).arity, 2);
        let t2 = env.tuple(2);
        assert_eq!(env.ty(t2).name, "(,)");
        assert_eq!(env.tuple(2), t2);
        let t0 = env.tuple(0);
        assert_eq!(env.ty(t0).name, "()");
        let t3 = env.tuple_con(3);
        assert_eq!(env.con(t3).arity, 3);
    }

    #[test]
    fn register_and_redefine() {
        let mut env = TypeEnv::new();
        let maybe = env
            .register("Maybe", &[spec("Nothing", 0), spec("Just", 1)])
            .unwrap();
        let just = env.lookup_con("Just").unwrap();
        assert_eq!(env.type_of_con(just), maybe);
        assert_eq!(env.con(just).tag, 1);
        assert!(env.is_complete(maybe, &[just, env.lookup_con("Nothing").unwrap()]));
        assert!(!env.is_complete(maybe, &[just]));

        // clash with another type
        let e = env.register("Other", &[spec("Just", 2)]).unwrap_err();
        assert_eq!(e.kind, ErrorKind::DataDeclaration);
        assert!(e.message.contains("already defined by type 'Maybe'"));

        // redefinition replaces constructors
        let maybe2 = env
            .register(
                "Maybe",
                &[spec("Nothing", 0), spec("Just", 1), spec("Unknown", 0)],
            )
            .unwrap();
        assert_ne!(maybe, maybe2);
        assert_eq!(env.lookup_type("Maybe"), Some(maybe2));
        assert!(!env.ty(maybe).live);
        assert_eq!(env.type_of_con(env.lookup_con("Unknown").unwrap()), maybe2);
        // and now "Just" can move to another type after Maybe is redefined without it
        env.register("Maybe", &[spec("Nothing", 0)]).unwrap();
        assert_eq!(env.lookup_con("Just"), None);
        env.register("Other", &[spec("Just", 2)]).unwrap();
    }

    #[test]
    fn register_errors() {
        let mut env = TypeEnv::new();
        assert!(env.register("T", &[]).is_err());
        assert!(env.register("Bool", &[spec("X", 0)]).is_err());
        assert!(env.register("T", &[spec("lower", 0)]).is_err());
        assert!(env.register("T", &[spec("A", 0), spec("A", 1)]).is_err());
        assert!(env
            .register(
                "T",
                &[ConSpec {
                    name: "R".into(),
                    arity: 2,
                    fields: Some(vec!["a".into()]),
                    handle: 0
                }]
            )
            .is_err());
        assert!(env.register("T", &[spec("True", 0)]).is_err());
    }
}
