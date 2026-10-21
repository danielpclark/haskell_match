//! The functions of the `HaskellMatch::Native` module, declared with
//! `#[rubymodule]`: parsing `data` declarations and Haskell source, the type
//! scopes, and the recursion settings.
//!
//! Each function takes Rust types (a wrong argument raises `TypeError`, an
//! integer out of range `RangeError`, a well-typed argument with a bad value
//! `ArgumentError`) and returns a `Result`, whose `Err` is raised once the
//! function's values have been dropped.  `HaskellMatch::Native.rbs` is the
//! module's RBS declaration.

use rutie::derive::FromArgument;
use rutie::{AnyException, Array, Object};

use super::error::argument_error;
use crate::core::types::ConSpec;

/// One constructor of a parsed `data` declaration: name, arity, record
/// field names (or `nil`) and field types.
pub type ParsedCon = (String, usize, Option<Vec<String>>, Vec<String>);

/// A parsed `data` declaration: type name, type variables, constructors and
/// the classes it derives.
pub type ParsedData = (String, Vec<String>, Vec<ParsedCon>, Vec<String>);

/// What `constructor_info_in` reports: type name, arity and record field
/// names (or `nil`).
pub type ConInfo = (String, usize, Option<Vec<String>>);

/// A `[name, arity, fields_or_nil, class]` row of `register_type_in`.
fn constructor_spec(row: &Array) -> Result<ConSpec, AnyException> {
    if row.length() != 4 {
        return Err(argument_error(
            "constructor spec must be [name, arity, fields, class]",
        ));
    }
    let name = String::hold(row.at(0))?;
    let arity = usize::hold(row.at(1))?;
    let fields = <Option<Vec<String>>>::hold(row.at(2))?;
    let klass = row.at(3);
    if klass.is_nil() {
        return Err(argument_error("constructor class must not be nil"));
    }
    Ok(ConSpec {
        name,
        arity,
        fields,
        handle: klass.value().value,
    })
}

#[rubymodule(path = "HaskellMatch::Native")]
pub mod functions {
    use rutie::rubysys::gc::rb_gc_register_mark_object;
    use rutie::types::Value;
    use rutie::{AnyException, Array};

    use super::{constructor_spec, ConInfo, ParsedCon, ParsedData};
    use crate::core::parser::parse_pattern;
    use crate::core::pretty::render_pat;
    use crate::core::resolve::resolve_clause;
    use crate::ruby::error::{argument_error, HmError};
    use crate::ruby::state;

    /// `Native.parse_data(decl)` ->
    /// `[name, tyvars, [[con, arity, fields_or_nil, types], ...], deriving]`
    pub fn parse_data(decl: &str) -> Result<ParsedData, HmError> {
        let d = crate::core::parser::parse_data(decl)?;
        let cons: Vec<ParsedCon> = d
            .cons
            .into_iter()
            .map(|c| (c.name, c.arity, c.fields, c.types))
            .collect();
        Ok((d.name, d.tyvars, cons, d.deriving))
    }

    /// `Native.new_scope` -> a type scope seeded from the global registry.
    pub fn new_scope() -> usize {
        let mut scopes = state::scopes();
        let seed = scopes[0].clone();
        scopes.push(seed);
        scopes.len() - 1
    }

    /// `Native.import_scope_in(dst, src, names_or_nil)` -> the type names
    /// imported: every user type of `src` (or those named) that `dst` does
    /// not already declare identically.
    pub fn import_scope_in(
        dst: usize,
        src: usize,
        names: Option<Vec<String>>,
    ) -> Result<Vec<String>, AnyException> {
        let mut scopes = state::scopes();
        if src >= scopes.len() || dst >= scopes.len() {
            return Err(argument_error("unknown type scope"));
        }
        let types = scopes[src].user_types();
        let env = &mut scopes[dst];
        let mut imported = Vec::new();
        for (name, cons) in types {
            if let Some(wanted) = &names {
                if !wanted.contains(&name) {
                    continue;
                }
            }
            if !env.has_same_type(&name, &cons) {
                env.register(&name, &cons)?;
            }
            imported.push(name);
        }
        Ok(imported)
    }

    /// `Native.register_type_in(name, [[con, arity, fields_or_nil, class], ...], scope)`
    pub fn register_type_in(
        name: &str,
        specs: Vec<Array>,
        scope: usize,
    ) -> Result<(), AnyException> {
        let mut cons = Vec::with_capacity(specs.len());
        for row in &specs {
            cons.push(constructor_spec(row)?);
        }
        let mut scopes = state::scopes();
        let env = state::scope_env(&mut scopes, scope)?;
        env.register(name, &cons)?;
        for c in &cons {
            // Keep constructor classes alive for as long as the process
            // runs: compiled matchers refer to them by VALUE.
            unsafe { rb_gc_register_mark_object(Value::from(c.handle)) };
        }
        Ok(())
    }

    /// `Native.render_pattern_in(src, scope)` -> the pattern's canonical
    /// Haskell rendering (checking it along the way).
    pub fn render_pattern_in(src: &str, scope: usize) -> Result<String, AnyException> {
        let raw = parse_pattern(src)?;
        let mut scopes = state::scopes();
        let env = state::scope_env(&mut scopes, scope)?;
        let (pats, bindings) = resolve_clause(env, &[raw])?;
        Ok(render_pat(env, &pats[0], &bindings.names))
    }

    /// `Native.constructors_in(type_name, scope)` -> the constructor names,
    /// or `nil` for an unknown type.
    pub fn constructors_in(name: &str, scope: usize) -> Result<Option<Vec<String>>, AnyException> {
        let mut scopes = state::scopes();
        let env = state::scope_env(&mut scopes, scope)?;
        Ok(env.lookup_type(name).map(|t| {
            env.ty(t)
                .cons
                .iter()
                .map(|c| env.con(*c).name.clone())
                .collect()
        }))
    }

    /// `Native.constructor_info_in(name, scope)` ->
    /// `[type_name, arity, fields_or_nil]`, or `nil` for an unknown
    /// constructor.
    pub fn constructor_info_in(name: &str, scope: usize) -> Result<Option<ConInfo>, AnyException> {
        let mut scopes = state::scopes();
        let env = state::scope_env(&mut scopes, scope)?;
        Ok(env.lookup_con(name).map(|c| {
            let con = env.con(c);
            (env.ty(con.ty).name.clone(), con.arity, con.fields.clone())
        }))
    }

    /// `Native.parse_haskell(source)` -> the module's AST as JSON.
    pub fn parse_haskell(source: &str) -> Result<String, HmError> {
        let module = crate::core::hs::parse_module(source)?;
        Ok(crate::core::hs::json::module(&module))
    }

    /// `Native.stack_segment` -> levels between Fiber segments (0: disabled).
    pub fn stack_segment() -> usize {
        state::stack_segment()
    }

    /// `Native.stack_segment = levels`
    #[ruby(name = "stack_segment=")]
    pub fn set_stack_segment(levels: i64) -> Result<i64, AnyException> {
        if levels < 0 {
            return Err(argument_error(
                "stack_segment must be a non-negative Integer (0 disables segmentation)",
            ));
        }
        state::set_stack_segment(levels as usize);
        Ok(levels)
    }

    /// `Native.max_depth` -> deepest allowed nesting (0: unlimited).
    pub fn max_depth() -> usize {
        state::max_depth()
    }

    /// `Native.max_depth = levels`
    #[ruby(name = "max_depth=")]
    pub fn set_max_depth(levels: i64) -> Result<i64, AnyException> {
        if levels < 0 {
            return Err(argument_error(
                "max_depth must be a non-negative Integer (0 means unlimited)",
            ));
        }
        state::set_max_depth(levels as usize);
        Ok(levels)
    }

    /// `Native.depth_push` -> the new nesting depth (raises
    /// `StackOverflowError` past `max_depth`); `Native.depth_pop` undoes it.
    /// Used by deep-mode functions, whose body invocation happens in Ruby.
    pub fn depth_push() -> Result<usize, HmError> {
        state::depth_push()
    }

    /// `Native.depth_pop`
    pub fn depth_pop() {
        state::depth_pop();
    }

    /// `Native.rbs` -> the module's RBS declaration.
    #[ruby(name = "rbs")]
    pub fn signature() -> String {
        rbs()
    }
}
