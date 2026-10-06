//! Ruby-facing API: the `HaskellMatch::Native` module and
//! `HaskellMatch::Native::Matcher` class.
//!
//! Matcher methods are defined as raw `extern "C"` functions (argc/argv
//! style) rather than through Rutie's `methods!` macro so that the hot path
//! allocates nothing on the Rust side and clause bodies are invoked from a
//! frame that owns no values needing `Drop` (a body may leave by exception,
//! `throw`, `break` or `return`).

pub mod runtime;

use std::ffi::CString;
use std::os::raw::c_int;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Mutex, OnceLock};

use rutie::rubysys::typed_data::{RUBY_TYPED_FREE_IMMEDIATELY, RUBY_TYPED_FROZEN_SHAREABLE};
use rutie::rubysys::{array, class as rclass, value::ValueType};
use rutie::typed_data::DataTypeWrapper;
use rutie::types::{CallbackPtr, DataType, DataTypeFunction, Id, Value};
use rutie::{
    AnyException, AnyObject, Array, Boolean, Class, Exception, Integer, Module, NilClass, Object,
    RString, VM,
};

use crate::core::error::{CoreError, ErrorKind};
use crate::core::exhaust;
use crate::core::parser::{parse_data, parse_pattern};
use crate::core::pretty::{render_pat, render_witness};
use crate::core::resolve::resolve_clause;
use crate::core::tree;
use crate::core::typecheck;
use crate::core::types::{ConSpec, TypeEnv};
use runtime::{Bound, Matcher, RtError, STACK_BINDS};

lazy_static! {
    static ref ENV: Mutex<TypeEnv> = Mutex::new(TypeEnv::new());
    static ref MATCHER_TYPE: MatcherType = MatcherType::new();
}

/// Interned ivar names read on every call.
struct Ids {
    bodies: Id,
    guards: Id,
}

static IDS: OnceLock<Ids> = OnceLock::new();

fn ids() -> &'static Ids {
    IDS.get_or_init(|| unsafe {
        let b = CString::new("@bodies").unwrap();
        let g = CString::new("@guards").unwrap();
        Ids {
            bodies: rutie::rubysys::symbol::rb_intern(b.as_ptr()),
            guards: rutie::rubysys::symbol::rb_intern(g.as_ptr()),
        }
    })
}

extern "C" {
    /// Resume a pending exception or non-local jump captured by `rb_protect`.
    fn rb_jump_tag(state: c_int) -> !;
}

/// Typed-data definition for `Matcher`.  Unlike Rutie's `wrappable_struct!`
/// it sets `RUBY_TYPED_FROZEN_SHAREABLE`, so a frozen matcher (and the
/// `Function` objects built on it) can be passed to `Ractor.make_shareable`.
pub struct MatcherType {
    data_type: DataType,
}

unsafe impl Send for MatcherType {}
unsafe impl Sync for MatcherType {}

impl MatcherType {
    fn new() -> Self {
        let name = CString::new("HaskellMatch/Matcher").unwrap();
        MatcherType {
            data_type: DataType {
                wrap_struct_name: name.into_raw(),
                parent: std::ptr::null(),
                data: std::ptr::null_mut(),
                flags: Value::from(RUBY_TYPED_FREE_IMMEDIATELY | RUBY_TYPED_FROZEN_SHAREABLE),
                function: DataTypeFunction {
                    dmark: None,
                    dfree: Some(rutie::typed_data::free::<Matcher>),
                    dsize: None,
                    reserved: [std::ptr::null_mut(), std::ptr::null_mut()],
                },
            },
        }
    }
}

impl DataTypeWrapper<Matcher> for MatcherType {
    fn data_type(&self) -> &DataType {
        &self.data_type
    }
}

class!(RMatcher);
module!(Native);

// ---------------------------------------------------------------------------
// error helpers
// ---------------------------------------------------------------------------

fn hm_class(name: &str) -> Class {
    Module::from_existing("HaskellMatch").get_nested_class(name)
}

fn exception(class_name: &str, message: &str) -> AnyException {
    AnyException::from_class(&hm_class(class_name), message)
}

fn error_class(kind: ErrorKind) -> &'static str {
    match kind {
        ErrorKind::Syntax => "PatternSyntaxError",
        ErrorKind::UnknownConstructor => "UnknownConstructorError",
        ErrorKind::Arity => "ArityError",
        ErrorKind::Type => "PatternTypeError",
        ErrorKind::DuplicateVariable => "DuplicateVariableError",
        ErrorKind::Field => "FieldError",
        ErrorKind::DataDeclaration => "DataDeclarationError",
        ErrorKind::ClauseArity => "ClauseArityError",
    }
}

fn raise_core(e: CoreError) -> ! {
    VM::raise_ex(exception(error_class(e.kind), &e.message));
    unreachable!()
}

fn raise_arg(msg: &str) -> ! {
    VM::raise_ex(AnyException::new("ArgumentError", Some(msg)));
    unreachable!()
}

fn inspect(v: Value) -> String {
    let s = unsafe { rutie::rubysys::object::rb_inspect(v) };
    RString::from(s).to_string()
}

fn class_name(v: Value) -> String {
    let k = unsafe { rclass::rb_obj_class(v) };
    Class::from(k)
        .name()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "?".into())
}

fn raise_rt(name: &str, e: RtError) -> ! {
    match e {
        RtError::Ruby(state) => unsafe { rb_jump_tag(state) },
        RtError::NoMatch => VM::raise_ex(exception(
            "MatchError",
            &format!("Non-exhaustive patterns in {}", name),
        )),
        RtError::TypeMismatch { expected, got } => VM::raise_ex(exception(
            "TypeMismatchError",
            &format!(
                "{}: expected a value of type {} but got {} ({})",
                name,
                expected,
                inspect(got),
                class_name(got)
            ),
        )),
        RtError::LazyFailed { clause, got } => VM::raise_ex(exception(
            "IrrefutablePatternError",
            &format!(
                "{}: irrefutable pattern failed for clause {}: {} does not match",
                name,
                clause + 1,
                inspect(got)
            ),
        )),
        RtError::Arity { expected, got } => VM::raise_ex(AnyException::new(
            "ArgumentError",
            Some(&format!(
                "wrong number of arguments (given {}, expected {})",
                got, expected
            )),
        )),
    }
    unreachable!()
}

fn raise_panic(payload: Box<dyn std::any::Any + Send>) -> ! {
    let msg = if let Some(m) = payload.downcast_ref::<&str>() {
        (*m).to_string()
    } else if let Some(m) = payload.downcast_ref::<String>() {
        m.clone()
    } else {
        "unknown panic".to_string()
    };
    drop(payload);
    VM::raise_ex(AnyException::new(
        "RuntimeError",
        Some(&format!("haskell_match internal error: {}", msg)),
    ));
    unreachable!()
}

fn str_arg(v: &AnyObject, what: &str) -> Result<String, String> {
    v.try_convert_to::<RString>()
        .map(|s| s.to_string())
        .map_err(|_| format!("{} must be a String, got {}", what, inspect(v.value())))
}

fn array_arg(v: &AnyObject, what: &str) -> Result<Array, String> {
    v.try_convert_to::<Array>()
        .map_err(|_| format!("{} must be an Array, got {}", what, inspect(v.value())))
}

fn rstrings(items: &[String]) -> Array {
    let mut a = Array::with_capacity(items.len());
    for s in items {
        a.push(RString::new_utf8(s));
    }
    a
}

// ---------------------------------------------------------------------------
// module functions
// ---------------------------------------------------------------------------

methods!(
    Native,
    _rtself,
    // parse_data(decl) -> [name, tyvars, [[con, arity, fields_or_nil], ...]]
    fn native_parse_data(decl: RString) -> Array {
        let decl = decl.unwrap_or_else(|_| raise_arg("data declaration must be a String"));
        let d = parse_data(&decl.to_string()).unwrap_or_else(|e| raise_core(e));
        let mut cons = Array::with_capacity(d.cons.len());
        for c in &d.cons {
            let mut row = Array::with_capacity(3);
            row.push(RString::new_utf8(&c.name));
            row.push(Integer::new(c.arity as i64));
            match &c.fields {
                Some(fs) => row.push(rstrings(fs).to_any_object()),
                None => row.push(NilClass::new().to_any_object()),
            };
            cons.push(row);
        }
        let mut out = Array::with_capacity(3);
        out.push(RString::new_utf8(&d.name));
        out.push(rstrings(&d.tyvars));
        out.push(cons);
        out
    },
    // register_type(name, [[con_name, arity, fields_or_nil, klass], ...]) -> nil
    fn native_register_type(name: RString, specs: Array) -> NilClass {
        let name = name
            .unwrap_or_else(|_| raise_arg("type name must be a String"))
            .to_string();
        let specs = specs.unwrap_or_else(|_| raise_arg("constructor specs must be an Array"));
        let outcome = (|| -> Result<Result<(), CoreError>, String> {
            let mut cons = Vec::new();
            for spec in specs.into_iter() {
                let row = array_arg(&spec, "constructor spec")?;
                if row.length() != 4 {
                    return Err("constructor spec must be [name, arity, fields, class]".into());
                }
                let cname = str_arg(&row.at(0), "constructor name")?;
                let arity = row
                    .at(1)
                    .try_convert_to::<Integer>()
                    .map_err(|_| "constructor arity must be an Integer".to_string())?
                    .to_i64();
                if arity < 0 {
                    return Err("constructor arity must not be negative".into());
                }
                let fields_obj = row.at(2);
                let fields = if fields_obj.is_nil() {
                    None
                } else {
                    let fa = array_arg(&fields_obj, "field names")?;
                    let mut names = Vec::new();
                    for f in fa.into_iter() {
                        names.push(str_arg(&f, "field name")?);
                    }
                    Some(names)
                };
                let klass = row.at(3);
                if klass.is_nil() {
                    return Err("constructor class must not be nil".into());
                }
                cons.push(ConSpec {
                    name: cname,
                    arity: arity as usize,
                    fields,
                    handle: klass.value().value,
                });
            }
            let mut env = ENV.lock().unwrap_or_else(|p| p.into_inner());
            let r = env.register(&name, &cons);
            if r.is_ok() {
                for c in &cons {
                    // keep constructor classes alive for as long as the
                    // process runs: compiled matchers refer to them by VALUE
                    unsafe {
                        rutie::rubysys::gc::rb_gc_register_mark_object(Value::from(c.handle))
                    };
                }
            }
            Ok(r.map(|_| ()))
        })();
        match outcome {
            Err(msg) => raise_arg(&msg),
            Ok(Err(e)) => raise_core(e),
            Ok(Ok(())) => NilClass::new(),
        }
    },
    // render_pattern(src) -> canonical Haskell rendering (checks the pattern)
    fn native_render_pattern(src: RString) -> RString {
        let src = src
            .unwrap_or_else(|_| raise_arg("pattern must be a String"))
            .to_string();
        let result = (|| -> Result<String, CoreError> {
            let raw = parse_pattern(&src)?;
            let mut env = ENV.lock().unwrap_or_else(|p| p.into_inner());
            let (pats, b) = resolve_clause(&mut env, &[raw])?;
            Ok(render_pat(&env, &pats[0], &b.names))
        })();
        match result {
            Ok(s) => RString::new_utf8(&s),
            Err(e) => raise_core(e),
        }
    },
    // constructors(type_name) -> [names] or nil
    fn native_constructors(name: RString) -> AnyObject {
        let name = name
            .unwrap_or_else(|_| raise_arg("type name must be a String"))
            .to_string();
        let env = ENV.lock().unwrap_or_else(|p| p.into_inner());
        match env.lookup_type(&name) {
            Some(t) => {
                let names: Vec<String> = env
                    .ty(t)
                    .cons
                    .iter()
                    .map(|c| env.con(*c).name.clone())
                    .collect();
                rstrings(&names).to_any_object()
            }
            None => NilClass::new().to_any_object(),
        }
    }
);

// ---------------------------------------------------------------------------
// Matcher construction
// ---------------------------------------------------------------------------

struct Built {
    matcher: Matcher,
    missing: Vec<String>,
    truncated: bool,
    redundant: Vec<usize>,
}

enum BuildError {
    Arg(String),
    Core(CoreError),
}

impl From<CoreError> for BuildError {
    fn from(e: CoreError) -> Self {
        BuildError::Core(e)
    }
}

fn build(
    name: String,
    clauses: &Array,
    guarded: &Array,
    limit: usize,
) -> Result<Built, BuildError> {
    let n = clauses.length();
    if n == 0 {
        return Err(BuildError::Arg("at least one clause is required".into()));
    }
    if guarded.length() != n {
        return Err(BuildError::Arg(
            "guard flags must have one entry per clause".into(),
        ));
    }
    let mut env = ENV.lock().unwrap_or_else(|p| p.into_inner());
    let mut resolved: Vec<(Vec<crate::core::ast::Pat>, bool)> = Vec::with_capacity(n);
    let mut names: Vec<Vec<String>> = Vec::with_capacity(n);
    let mut guard_flags: Vec<bool> = Vec::with_capacity(n);
    let mut arity: Option<usize> = None;
    for (i, clause) in clauses.dup().into_iter().enumerate() {
        let pats = clause.try_convert_to::<Array>().map_err(|_| {
            CoreError::syntax(format!(
                "clause {} must be an Array of pattern strings",
                i + 1
            ))
        })?;
        let mut raws = Vec::with_capacity(pats.length());
        for p in pats.into_iter() {
            let src = p
                .try_convert_to::<RString>()
                .map_err(|_| {
                    CoreError::syntax(format!("clause {}: patterns must be Strings", i + 1))
                })?
                .to_string();
            let raw = parse_pattern(&src)
                .map_err(|e| CoreError::new(e.kind, format!("clause {}: {}", i + 1, e.message)))?;
            raws.push(raw);
        }
        if raws.is_empty() {
            return Err(CoreError::new(
                ErrorKind::ClauseArity,
                format!(
                    "clause {} has no patterns (a function needs at least one argument)",
                    i + 1
                ),
            )
            .into());
        }
        match arity {
            None => arity = Some(raws.len()),
            Some(a) if a != raws.len() => {
                return Err(CoreError::new(
                    ErrorKind::ClauseArity,
                    format!(
                        "equations for '{}' have different numbers of arguments: clause 1 has {}, clause {} has {}",
                        name,
                        a,
                        i + 1,
                        raws.len()
                    ),
                )
                .into());
            }
            _ => {}
        }
        let (ps, b) = resolve_clause(&mut env, &raws)
            .map_err(|e| CoreError::new(e.kind, format!("clause {}: {}", i + 1, e.message)))?;
        let g = guarded.at(i as i64);
        let g = !(g.is_nil() || g.value().is_false());
        resolved.push((ps, g));
        names.push(b.names);
        guard_flags.push(g);
    }
    let arity = arity.expect("at least one clause");
    let only_pats: Vec<Vec<crate::core::ast::Pat>> =
        resolved.iter().map(|(p, _)| p.clone()).collect();
    typecheck::check(&env, &only_pats)?;
    let analysis = exhaust::analyze(&env, &resolved, limit);
    let missing: Vec<String> = analysis
        .missing
        .iter()
        .map(|w| render_witness(&env, w))
        .collect();
    let clauses_for_tree: Vec<tree::Clause> = resolved
        .iter()
        .zip(names.iter())
        .map(|((pats, g), ns)| tree::Clause {
            pats: pats.clone(),
            n_vars: ns.len(),
            guarded: *g,
        })
        .collect();
    let compiled = tree::compile(&env, &clauses_for_tree, arity);
    let snapshot: TypeEnv = env.clone();
    drop(env);
    let rt = runtime::translate(&snapshot, &compiled.tree);
    let mut lits = Vec::new();
    runtime::lazy_literals(&compiled.tree, &mut lits);
    let lazy_lits = lits
        .into_iter()
        .map(|l| (l.key(), runtime::literal_object(&l)))
        .collect();
    Ok(Built {
        matcher: Matcher {
            env: snapshot,
            tree: rt,
            arity,
            n_slots: compiled.n_slots,
            names,
            guarded: guard_flags,
            name,
            lazy_lits,
        },
        missing,
        truncated: analysis.truncated,
        redundant: analysis.redundant,
    })
}

#[inline]
fn matcher_of(obj: Value) -> &'static Matcher {
    // `get_data` checks the typed-data tag and raises TypeError otherwise.
    let any = AnyObject::from(obj);
    let m: &Matcher = any.get_data(&*MATCHER_TYPE);
    // SAFETY: the matcher lives as long as the Ruby object, which the caller
    // keeps alive for the duration of the call.
    unsafe { &*(m as *const Matcher) }
}

// ---------------------------------------------------------------------------
// Matcher methods (raw extern "C")
// ---------------------------------------------------------------------------

/// Matcher.new(name, clauses, guarded_flags, witness_limit)
unsafe extern "C" fn matcher_new(
    klass: Value,
    name: Value,
    clauses: Value,
    guarded: Value,
    limit: Value,
) -> Value {
    let outcome = catch_unwind(AssertUnwindSafe(
        || -> Result<(AnyObject, Vec<String>, bool, Vec<usize>), BuildError> {
            let name = AnyObject::from(name)
                .try_convert_to::<RString>()
                .map(|s| s.to_string())
                .map_err(|_| BuildError::Arg("name must be a String".into()))?;
            let clauses = AnyObject::from(clauses)
                .try_convert_to::<Array>()
                .map_err(|_| BuildError::Arg("clauses must be an Array".into()))?;
            let guarded = AnyObject::from(guarded)
                .try_convert_to::<Array>()
                .map_err(|_| BuildError::Arg("guard flags must be an Array".into()))?;
            let limit = AnyObject::from(limit)
                .try_convert_to::<Integer>()
                .map(|i| i.to_i64())
                .unwrap_or(20)
                .max(1) as usize;
            let built = build(name, &clauses, &guarded, limit)?;
            let obj: AnyObject = Class::from(klass).wrap_data(built.matcher, &*MATCHER_TYPE);
            Ok((obj, built.missing, built.truncated, built.redundant))
        },
    ));
    match outcome {
        Err(payload) => raise_panic(payload),
        Ok(Err(BuildError::Arg(msg))) => raise_arg(&msg),
        Ok(Err(BuildError::Core(e))) => raise_core(e),
        Ok(Ok((mut obj, missing, truncated, redundant))) => {
            obj.instance_variable_set("@missing", rstrings(&missing));
            obj.instance_variable_set("@missing_truncated", Boolean::new(truncated));
            let mut red = Array::with_capacity(redundant.len());
            for r in redundant {
                red.push(Integer::new(r as i64));
            }
            obj.instance_variable_set("@redundant", red);
            obj.value()
        }
    }
}

/// Outcome of the pure-Rust part of a call: the body to invoke and where its
/// arguments are.
struct Prepared {
    body: Value,
    bound: Bound,
}

/// Run `select` and locate the body, without calling into Ruby.
unsafe fn prepare(
    m: &Matcher,
    args: &[Value],
    bodies: Value,
    guards: Value,
    buf: &mut [Value; STACK_BINDS],
) -> Result<Prepared, RtError> {
    let (idx, bound) = m.select(args, guards, buf)?;
    let body = if bodies.ty() == ValueType::Array {
        array::rb_ary_entry(bodies, idx as _)
    } else {
        Value::from(rutie::rubysys::value::RubySpecialConsts::Nil as usize)
    };
    Ok(Prepared { body, bound })
}

fn check_guards(guards: Value) -> Result<(), String> {
    if guards.is_nil() || guards.ty() == ValueType::Array {
        Ok(())
    } else {
        Err("guards must be an Array or nil".into())
    }
}

/// matcher.call(*args): select a clause and call its body (from `@bodies`,
/// with guards from `@guards`).
unsafe extern "C" fn matcher_call(argc: c_int, argv: *const Value, rtself: Value) -> Value {
    let mut buf = [Value::from(0); STACK_BINDS];
    let args = std::slice::from_raw_parts(argv, argc.max(0) as usize);
    let outcome = catch_unwind(AssertUnwindSafe(|| -> Result<Prepared, RtError> {
        let m = matcher_of(rtself);
        let bodies = rclass::rb_ivar_get(rtself, ids().bodies);
        let guards = rclass::rb_ivar_get(rtself, ids().guards);
        prepare(m, args, bodies, guards, &mut buf)
    }));
    match outcome {
        Err(payload) => raise_panic(payload),
        Ok(Err(e)) => raise_rt(&matcher_of(rtself).name, e),
        Ok(Ok(p)) => {
            if p.body.is_nil() {
                raise_arg("this matcher has no clause bodies attached");
            }
            // No Rust value needing Drop is live here: the body may longjmp.
            runtime::invoke(p.body, &p.bound, &buf)
        }
    }
}

/// matcher.select(*args) -> [clause_index, [values]] or nil (guards from
/// `@guards`).
unsafe extern "C" fn matcher_select(argc: c_int, argv: *const Value, rtself: Value) -> Value {
    let mut buf = [Value::from(0); STACK_BINDS];
    let args = std::slice::from_raw_parts(argv, argc.max(0) as usize);
    let outcome = catch_unwind(AssertUnwindSafe(|| -> Result<(usize, Bound), RtError> {
        let m = matcher_of(rtself);
        let guards = rclass::rb_ivar_get(rtself, ids().guards);
        m.select(args, guards, &mut buf)
    }));
    select_result(outcome, rtself, &buf)
}

unsafe fn select_result(
    outcome: Result<Result<(usize, Bound), RtError>, Box<dyn std::any::Any + Send>>,
    rtself: Value,
    buf: &[Value],
) -> Value {
    match outcome {
        Err(payload) => raise_panic(payload),
        Ok(Err(RtError::NoMatch)) => {
            Value::from(rutie::rubysys::value::RubySpecialConsts::Nil as usize)
        }
        Ok(Err(e)) => raise_rt(&matcher_of(rtself).name, e),
        Ok(Ok((idx, bound))) => {
            let vals = runtime::bound_to_array(&bound, buf);
            let pair = [Integer::new(idx as i64).value(), vals];
            array::rb_ary_new_from_values(2, pair.as_ptr())
        }
    }
}

/// Read an argument array into a Vec of values (rooted by the array itself).
fn args_of(args: Value) -> Result<Vec<Value>, String> {
    if args.ty() != ValueType::Array {
        return Err("arguments must be an Array".into());
    }
    unsafe {
        let n = array::rb_ary_len(args) as usize;
        Ok((0..n).map(|i| array::rb_ary_entry(args, i as _)).collect())
    }
}

/// matcher.select_with(args_array, guards) -> [clause_index, [values]] or nil
unsafe extern "C" fn matcher_select_with(rtself: Value, args: Value, guards: Value) -> Value {
    let mut buf = [Value::from(0); STACK_BINDS];
    let outcome = catch_unwind(AssertUnwindSafe(
        || -> Result<Result<(usize, Bound), RtError>, String> {
            let m = matcher_of(rtself);
            let args = args_of(args)?;
            check_guards(guards)?;
            Ok(m.select(&args, guards, &mut buf))
        },
    ));
    let outcome = match outcome {
        Ok(Err(msg)) => raise_arg(&msg),
        Ok(Ok(r)) => Ok(r),
        Err(p) => Err(p),
    };
    select_result(outcome, rtself, &buf)
}

/// matcher.run(args_array, bodies, guards) -> result of the selected body.
unsafe extern "C" fn matcher_run(
    rtself: Value,
    args: Value,
    bodies: Value,
    guards: Value,
) -> Value {
    let mut buf = [Value::from(0); STACK_BINDS];
    let outcome = catch_unwind(AssertUnwindSafe(
        || -> Result<Result<Prepared, RtError>, String> {
            let m = matcher_of(rtself);
            let args = args_of(args)?;
            check_guards(guards)?;
            if bodies.ty() != ValueType::Array {
                return Err("bodies must be an Array".into());
            }
            Ok(prepare(m, &args, bodies, guards, &mut buf))
        },
    ));
    match outcome {
        Err(payload) => raise_panic(payload),
        Ok(Err(msg)) => raise_arg(&msg),
        Ok(Ok(Err(e))) => raise_rt(&matcher_of(rtself).name, e),
        Ok(Ok(Ok(p))) => {
            if p.body.is_nil() {
                raise_arg("no body for the selected clause");
            }
            runtime::invoke(p.body, &p.bound, &buf)
        }
    }
}

unsafe extern "C" fn matcher_arity(rtself: Value) -> Value {
    Integer::new(matcher_of(rtself).arity as i64).value()
}

unsafe extern "C" fn matcher_name(rtself: Value) -> Value {
    RString::new_utf8(&matcher_of(rtself).name).value()
}

unsafe extern "C" fn matcher_names(rtself: Value) -> Value {
    let m = matcher_of(rtself);
    let mut out = Array::with_capacity(m.names.len());
    for ns in &m.names {
        out.push(rstrings(ns));
    }
    out.value()
}

unsafe extern "C" fn matcher_guarded(rtself: Value) -> Value {
    let m = matcher_of(rtself);
    let mut out = Array::with_capacity(m.guarded.len());
    for g in &m.guarded {
        out.push(Boolean::new(*g));
    }
    out.value()
}

unsafe extern "C" fn matcher_slots(rtself: Value) -> Value {
    Integer::new(matcher_of(rtself).n_slots as i64).value()
}

unsafe extern "C" fn matcher_tree(rtself: Value) -> Value {
    let m = matcher_of(rtself);
    RString::new_utf8(&runtime::dump(&m.env, &m.tree, &m.names, 0)).value()
}

unsafe fn def_method(klass: Value, name: &str, f: CallbackPtr, argc: c_int) {
    let cname = CString::new(name).unwrap();
    rclass::rb_define_method(klass, cname.as_ptr(), f, argc);
}

pub fn init() {
    // Nothing here keeps per-Ractor state: the type registry is behind a
    // mutex and matchers are immutable once built.
    unsafe { VM::ext_ractor_safe(true) };
    let mut hm = Module::from_existing("HaskellMatch");
    let mut native = hm.define_nested_module("Native");
    native.define(|m| {
        m.def_self("parse_data", native_parse_data);
        m.def_self("register_type", native_register_type);
        m.def_self("render_pattern", native_render_pattern);
        m.def_self("constructors", native_constructors);
    });
    let object = Class::from_existing("Object");
    let mut matcher = native.define_nested_class("Matcher", Some(&object));
    // Instances are only ever created through `new` with wrapped data.
    matcher.undef_alloc_func();
    unsafe {
        let k = matcher.value();
        let new_name = CString::new("new").unwrap();
        rclass::rb_define_singleton_method(k, new_name.as_ptr(), matcher_new as CallbackPtr, 4);
        def_method(k, "call", matcher_call as CallbackPtr, -1);
        def_method(k, "select", matcher_select as CallbackPtr, -1);
        def_method(k, "select_with", matcher_select_with as CallbackPtr, 2);
        def_method(k, "run", matcher_run as CallbackPtr, 3);
        def_method(k, "arity", matcher_arity as CallbackPtr, 0);
        def_method(k, "name", matcher_name as CallbackPtr, 0);
        def_method(k, "names", matcher_names as CallbackPtr, 0);
        def_method(k, "guarded", matcher_guarded as CallbackPtr, 0);
        def_method(k, "slots", matcher_slots as CallbackPtr, 0);
        def_method(k, "tree", matcher_tree as CallbackPtr, 0);
    }
    ids();
}
