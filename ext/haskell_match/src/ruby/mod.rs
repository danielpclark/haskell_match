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
use rutie::rubysys::{array, class as rclass, rstruct, value::ValueType};
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
    /// Type scopes: index 0 is the global registry; every other scope is a
    /// snapshot of the global one at its creation plus its own declarations
    /// (a compiled Haskell module gets one, like a Haskell module namespace).
    static ref SCOPES: Mutex<Vec<TypeEnv>> = Mutex::new(vec![TypeEnv::new()]);
    static ref MATCHER_TYPE: MatcherType = MatcherType::new();
}

/// Interned ivar names read on every call.
struct Ids {
    bodies: Id,
    guards: Id,
}

static IDS: OnceLock<Ids> = OnceLock::new();

/// `HaskellMatch::TailCall`, the marker a clause body returns to request a
/// tail call (looked up once, on first use).
static TAIL_CALL: OnceLock<usize> = OnceLock::new();

fn tail_call_class() -> usize {
    *TAIL_CALL.get_or_init(|| {
        let k = hm_class("TailCall");
        unsafe { rutie::rubysys::gc::rb_gc_register_mark_object(k.value()) };
        k.value().value
    })
}

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
    fn rb_fiber_new(func: rutie::rubysys::types::BlockCallFunction, obj: Value) -> Value;
    fn rb_fiber_resume(fiber: Value, argc: c_int, argv: *const Value) -> Value;
}

thread_local! {
    /// Nesting depth of native calls on this thread.
    static DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Every this many nested native calls, the next clause body runs in a fresh
/// Fiber.  Each Fiber brings its own VM stack and machine stack, so chained
/// segments make the recursion depth a matter of memory rather than of
/// Ruby's fixed stack size.  0 disables segmentation.
static STACK_SEGMENT: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(DEFAULT_STACK_SEGMENT);
const DEFAULT_STACK_SEGMENT: usize = 100;

/// Maximum nesting depth of native calls before `StackOverflowError` is
/// raised, so a runaway recursion fails instead of consuming all memory
/// (GHC has the same kind of limit).  0 means unlimited.
static MAX_DEPTH: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(DEFAULT_MAX_DEPTH);
const DEFAULT_MAX_DEPTH: usize = 250_000;

rutie_callback! {
    /// Body of a stack-segment Fiber: `data` is `[body, values]`.
    fn segment_fiber_body(
        _yielded: Value,
        data: Value,
        _argc: c_int,
        _argv: *const Value,
        _block: Value,
    ) -> Value {
        unsafe {
            let body = array::rb_ary_entry(data, 0);
            let vals = array::rb_ary_entry(data, 1);
            rutie::rubysys::rproc::rb_proc_call(body, vals)
        }
    }
}

/// Invoke a clause body, either directly or (every `STACK_SEGMENT` levels)
/// in a fresh Fiber.  Exceptions and non-local jumps are returned as the
/// `rb_protect` state for the caller to resume once it has cleaned up.
unsafe fn invoke_body(body: Value, bound: &Bound, buf: &[Value]) -> Result<Value, i32> {
    let segment = STACK_SEGMENT.load(std::sync::atomic::Ordering::Relaxed);
    let depth = DEPTH.with(|d| d.get());
    let max = MAX_DEPTH.load(std::sync::atomic::Ordering::Relaxed);
    if max > 0 && depth >= max {
        VM::raise_ex(exception(
            "StackOverflowError",
            &format!(
                "recursion deeper than HaskellMatch.max_depth ({}); use tail calls, defer, or raise the limit",
                max
            ),
        ));
    }
    DEPTH.with(|d| d.set(depth + 1));
    let result = if segment > 0 && (depth + 1).is_multiple_of(segment) {
        let vals = runtime::bound_to_array(bound, buf);
        let pair = [body, vals];
        let data = array::rb_ary_new_from_values(2, pair.as_ptr());
        let fiber = rb_fiber_new(
            segment_fiber_body as rutie::rubysys::types::BlockCallFunction,
            data,
        );
        runtime::call_protected(|| rb_fiber_resume(fiber, 0, std::ptr::null()))
    } else {
        runtime::call_protected(|| runtime::invoke(body, bound, buf))
    };
    DEPTH.with(|d| d.set(depth));
    result
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

/// A scope id argument (raises ArgumentError when malformed).
fn scope_arg(scope: Result<Integer, rutie::AnyException>) -> usize {
    let n = scope
        .unwrap_or_else(|_| raise_arg("scope must be an Integer"))
        .to_i64();
    if n < 0 {
        raise_arg("scope must not be negative");
    }
    n as usize
}

/// The type environment of scope `id`.
fn scope_env(scopes: &mut [TypeEnv], id: usize) -> Result<&mut TypeEnv, String> {
    scopes
        .get_mut(id)
        .ok_or_else(|| format!("unknown type scope {}", id))
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
    // parse_data(decl) -> [name, tyvars, [[con, arity, fields_or_nil, types], ...], deriving]
    fn native_parse_data(decl: RString) -> Array {
        let decl = decl.unwrap_or_else(|_| raise_arg("data declaration must be a String"));
        let d = parse_data(&decl.to_string()).unwrap_or_else(|e| raise_core(e));
        let mut cons = Array::with_capacity(d.cons.len());
        for c in &d.cons {
            let mut row = Array::with_capacity(4);
            row.push(RString::new_utf8(&c.name));
            row.push(Integer::new(c.arity as i64));
            match &c.fields {
                Some(fs) => row.push(rstrings(fs).to_any_object()),
                None => row.push(NilClass::new().to_any_object()),
            };
            row.push(rstrings(&c.types));
            cons.push(row);
        }
        let mut out = Array::with_capacity(4);
        out.push(RString::new_utf8(&d.name));
        out.push(rstrings(&d.tyvars));
        out.push(cons);
        out.push(rstrings(&d.deriving));
        out
    },
    // new_scope -> Integer: a type scope seeded from the global registry
    fn native_new_scope() -> Integer {
        let mut scopes = SCOPES.lock().unwrap_or_else(|p| p.into_inner());
        let seed = scopes[0].clone();
        scopes.push(seed);
        Integer::new((scopes.len() - 1) as i64)
    },
    // import_scope_in(dst, src, names_or_nil) -> [type names imported]
    fn native_import_scope(dst: Integer, src: Integer, names: AnyObject) -> AnyObject {
        let dst = scope_arg(dst);
        let src = scope_arg(src);
        let names = names.unwrap_or_else(|_| raise_arg("names must be an Array or nil"));
        let wanted: Option<Vec<String>> = if names.is_nil() {
            None
        } else {
            let arr = array_arg(&names, "type names").unwrap_or_else(|m| raise_arg(&m));
            Some(
                arr.into_iter()
                    .map(|n| str_arg(&n, "type name").unwrap_or_else(|m| raise_arg(&m)))
                    .collect(),
            )
        };
        let mut scopes = SCOPES.lock().unwrap_or_else(|p| p.into_inner());
        if src >= scopes.len() || dst >= scopes.len() {
            raise_arg("unknown type scope");
        }
        let types = scopes[src].user_types();
        let env = &mut scopes[dst];
        let mut imported = Vec::new();
        for (name, cons) in types {
            if let Some(w) = &wanted {
                if !w.contains(&name) {
                    continue;
                }
            }
            if env.has_same_type(&name, &cons) {
                imported.push(name);
                continue;
            }
            match env.register(&name, &cons) {
                Ok(_) => imported.push(name),
                Err(e) => raise_core(e),
            }
        }
        rstrings(&imported).to_any_object()
    },
    // register_type(name, [[con_name, arity, fields_or_nil, klass], ...], scope) -> nil
    fn native_register_type(name: RString, specs: Array, scope: Integer) -> NilClass {
        let name = name
            .unwrap_or_else(|_| raise_arg("type name must be a String"))
            .to_string();
        let specs = specs.unwrap_or_else(|_| raise_arg("constructor specs must be an Array"));
        let scope = scope_arg(scope);
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
            let mut scopes = SCOPES.lock().unwrap_or_else(|p| p.into_inner());
            let env = scope_env(&mut scopes, scope)?;
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
    fn native_render_pattern(src: RString, scope: Integer) -> RString {
        let src = src
            .unwrap_or_else(|_| raise_arg("pattern must be a String"))
            .to_string();
        let scope = scope_arg(scope);
        let result = (|| -> Result<String, CoreError> {
            let raw = parse_pattern(&src)?;
            let mut scopes = SCOPES.lock().unwrap_or_else(|p| p.into_inner());
            let env = scope_env(&mut scopes, scope).unwrap_or_else(|m| raise_arg(&m));
            let (pats, b) = resolve_clause(env, &[raw])?;
            Ok(render_pat(env, &pats[0], &b.names))
        })();
        match result {
            Ok(s) => RString::new_utf8(&s),
            Err(e) => raise_core(e),
        }
    },
    // stack_segment -> Integer (0 = disabled)
    fn native_stack_segment() -> Integer {
        Integer::new(STACK_SEGMENT.load(std::sync::atomic::Ordering::Relaxed) as i64)
    },
    // stack_segment = n
    fn native_set_stack_segment(n: Integer) -> Integer {
        let n = n.map(|i| i.to_i64()).unwrap_or(-1);
        if n < 0 {
            raise_arg("stack_segment must be a non-negative Integer (0 disables segmentation)");
        }
        STACK_SEGMENT.store(n as usize, std::sync::atomic::Ordering::Relaxed);
        Integer::new(n)
    },
    // max_depth -> Integer (0 = unlimited)
    fn native_max_depth() -> Integer {
        Integer::new(MAX_DEPTH.load(std::sync::atomic::Ordering::Relaxed) as i64)
    },
    // max_depth = n
    fn native_set_max_depth(n: Integer) -> Integer {
        let n = n.map(|i| i.to_i64()).unwrap_or(-1);
        if n < 0 {
            raise_arg("max_depth must be a non-negative Integer (0 means unlimited)");
        }
        MAX_DEPTH.store(n as usize, std::sync::atomic::Ordering::Relaxed);
        Integer::new(n)
    },
    // parse_haskell(source) -> JSON string of the module AST
    fn native_parse_haskell(src: RString) -> RString {
        let src = src
            .unwrap_or_else(|_| raise_arg("Haskell source must be a String"))
            .to_string();
        match crate::core::hs::parse_module(&src) {
            Ok(m) => RString::new_utf8(&crate::core::hs::json::module(&m)),
            Err(e) => raise_core(e),
        }
    },
    // constructor_info(name) -> [type_name, arity, fields_or_nil] or nil
    fn native_constructor_info(name: RString, scope: Integer) -> AnyObject {
        let name = name
            .unwrap_or_else(|_| raise_arg("constructor name must be a String"))
            .to_string();
        let scope = scope_arg(scope);
        let mut scopes = SCOPES.lock().unwrap_or_else(|p| p.into_inner());
        let env = scope_env(&mut scopes, scope).unwrap_or_else(|m| raise_arg(&m));
        match env.lookup_con(&name) {
            Some(c) => {
                let con = env.con(c);
                let mut row = Array::with_capacity(3);
                row.push(RString::new_utf8(&env.ty(con.ty).name));
                row.push(Integer::new(con.arity as i64));
                match &con.fields {
                    Some(fs) => row.push(rstrings(fs).to_any_object()),
                    None => row.push(NilClass::new().to_any_object()),
                };
                row.to_any_object()
            }
            None => NilClass::new().to_any_object(),
        }
    },
    // constructors(type_name) -> [names] or nil
    fn native_constructors(name: RString, scope: Integer) -> AnyObject {
        let name = name
            .unwrap_or_else(|_| raise_arg("type name must be a String"))
            .to_string();
        let scope = scope_arg(scope);
        let mut scopes = SCOPES.lock().unwrap_or_else(|p| p.into_inner());
        let env = scope_env(&mut scopes, scope).unwrap_or_else(|m| raise_arg(&m));
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
    scope: usize,
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
    let mut scopes = SCOPES.lock().unwrap_or_else(|p| p.into_inner());
    let env = scope_env(&mut scopes, scope).map_err(BuildError::Arg)?;
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
        let (ps, b) = resolve_clause(env, &raws)
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
    typecheck::check(env, &only_pats)?;
    let analysis = exhaust::analyze(env, &resolved, limit);
    let missing: Vec<String> = analysis
        .missing
        .iter()
        .map(|w| render_witness(env, w))
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
    let compiled = tree::compile(env, &clauses_for_tree, arity);
    let snapshot: TypeEnv = env.clone();
    drop(scopes);
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
    scope: Value,
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
            let scope = AnyObject::from(scope)
                .try_convert_to::<Integer>()
                .map(|i| i.to_i64())
                .map_err(|_| BuildError::Arg("scope must be an Integer".into()))?;
            if scope < 0 {
                return Err(BuildError::Arg("scope must not be negative".into()));
            }
            let built = build(name, &clauses, &guarded, limit, scope as usize)?;
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

/// Continuations pending in a trampolined call, kept as a chain of small
/// Ruby arrays rather than one growing array.  A full chunk is never written
/// again, so once Ruby's generational GC promotes it, minor collections skip
/// it; with a single array every push would re-mark the whole stack on each
/// minor GC, making GC cost grow with recursion depth.
const CONT_CHUNK: i64 = 256;

struct ContStack {
    /// Current chunk: element 0 links to the previous chunk (or nil).
    chunk: Value,
}

impl ContStack {
    fn new() -> Self {
        ContStack {
            chunk: Value::from(rutie::rubysys::value::RubySpecialConsts::Nil as usize),
        }
    }

    #[inline]
    unsafe fn push(&mut self, k: Value) {
        if self.chunk.is_nil() || array::rb_ary_len(self.chunk) as i64 > CONT_CHUNK {
            let fresh = array::rb_ary_new_capa((CONT_CHUNK + 1) as _);
            array::rb_ary_push(fresh, self.chunk);
            self.chunk = fresh;
        }
        array::rb_ary_push(self.chunk, k);
    }

    /// Pop the most recent continuation, or nil when none is pending.
    #[inline]
    unsafe fn pop(&mut self) -> Value {
        loop {
            if self.chunk.is_nil() {
                return self.chunk;
            }
            if array::rb_ary_len(self.chunk) <= 1 {
                self.chunk = array::rb_ary_entry(self.chunk, 0);
                continue;
            }
            return array::rb_ary_pop(self.chunk);
        }
    }
}

/// matcher.call(*args): select a clause and call its body (from `@bodies`,
/// with guards from `@guards`).
///
/// The call runs as a trampoline.  A body may return a
/// `HaskellMatch::TailCall(function, args, continuation)`:
///
/// * with a nil continuation (`Function#tail`) the call simply continues
///   with that function and those arguments, in constant stack space;
/// * with a continuation (`Function#defer`) the continuation is pushed on a
///   chunked stack owned by this frame and the call continues; once a body
///   returns an ordinary value the pending continuations are applied to it
///   one after another (each may itself return a `TailCall`).
///
/// Recursion depth is therefore bounded by memory, as in GHC, rather than
/// by Ruby's VM stack.
unsafe extern "C" fn matcher_call(argc: c_int, argv: *const Value, rtself: Value) -> Value {
    let mut buf = [Value::from(0); STACK_BINDS];
    let mut target = rtself;
    // Pending continuations (Ruby arrays, rooted by this frame).
    let mut conts = ContStack::new();
    // The current marker keeps the next arguments alive while they are matched.
    let mut marker = Value::from(0);
    let mut args_vec: Vec<Value> = Vec::new();
    let mut first = true;
    let tail_class = tail_call_class();
    loop {
        let args: &[Value] = if first {
            std::slice::from_raw_parts(argv, argc.max(0) as usize)
        } else {
            &args_vec[..]
        };
        let outcome = catch_unwind(AssertUnwindSafe(|| -> Result<Prepared, RtError> {
            let m = matcher_of(target);
            let bodies = rclass::rb_ivar_get(target, ids().bodies);
            let guards = rclass::rb_ivar_get(target, ids().guards);
            prepare(m, args, bodies, guards, &mut buf)
        }));
        let p = match outcome {
            Err(payload) => {
                drop(args_vec);
                raise_panic(payload)
            }
            Ok(Err(e)) => {
                drop(args_vec);
                raise_rt(&matcher_of(target).name, e)
            }
            Ok(Ok(p)) => p,
        };
        if p.body.is_nil() {
            drop(args_vec);
            raise_arg("this matcher has no clause bodies attached");
        }
        // Nothing needing Drop may be live across a call into Ruby, which
        // may longjmp: `args_vec` is emptied first (its arguments are rooted
        // by `marker` anyway).
        drop(std::mem::take(&mut args_vec));
        let mut result = match invoke_body(p.body, &p.bound, &buf) {
            Ok(v) => v,
            Err(state) => rb_jump_tag(state),
        };
        // Apply pending continuations until one asks for another call.
        while rclass::rb_obj_class(result).value != tail_class {
            let k = conts.pop();
            if k.is_nil() {
                std::hint::black_box(marker);
                return result;
            }
            result = rutie::rubysys::rproc::rb_proc_call_with_block(
                k,
                1,
                &result,
                Value::from(rutie::rubysys::value::RubySpecialConsts::Nil as usize),
            );
        }
        // TailCall(function, args, continuation): continue with the next call.
        marker = result;
        let function = rstruct::rb_struct_aref(result, Integer::new(0).value());
        let next_args = rstruct::rb_struct_aref(result, Integer::new(1).value());
        let continuation = rstruct::rb_struct_aref(result, Integer::new(2).value());
        if next_args.ty() != ValueType::Array {
            raise_arg("TailCall arguments must be an Array");
        }
        if !continuation.is_nil() {
            conts.push(continuation);
        }
        // The matcher type is checked by `matcher_of` on the next iteration.
        target = function;
        let n = array::rb_ary_len(next_args) as usize;
        args_vec = (0..n)
            .map(|i| array::rb_ary_entry(next_args, i as _))
            .collect();
        first = false;
    }
}

/// matcher.prepare(*args) -> [values..., body, depth]: the selected clause's
/// bound values, its body (from `@bodies`, guards from `@guards`) and the
/// nesting depth after counting this call, for callers that invoke the body
/// from Ruby code (see `Function` deep mode); they call `Native.depth_pop`
/// once the body has returned.
unsafe extern "C" fn matcher_prepare(argc: c_int, argv: *const Value, rtself: Value) -> Value {
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
            let out = match p.bound {
                Bound::Stack(n) => array::rb_ary_new_from_values(n as _, buf.as_ptr()),
                Bound::Array(a) => a,
            };
            array::rb_ary_push(out, p.body);
            // Count this level now; the Ruby caller pops it after the body.
            let depth = native_depth_push(rtself);
            array::rb_ary_push(out, depth);
            out
        }
    }
}

/// Native.depth_push -> the new nesting depth (raises StackOverflowError past
/// max_depth); Native.depth_pop undoes it.  Used by deep-mode functions,
/// whose body invocation happens in Ruby.
unsafe extern "C" fn native_depth_push(_rtself: Value) -> Value {
    let depth = DEPTH.with(|d| d.get());
    let max = MAX_DEPTH.load(std::sync::atomic::Ordering::Relaxed);
    if max > 0 && depth >= max {
        VM::raise_ex(exception(
            "StackOverflowError",
            &format!(
                "recursion deeper than HaskellMatch.max_depth ({}); use tail calls, defer, or raise the limit",
                max
            ),
        ));
    }
    DEPTH.with(|d| d.set(depth + 1));
    Integer::new((depth + 1) as i64).value()
}

unsafe extern "C" fn native_depth_pop(_rtself: Value) -> Value {
    DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
    Value::from(rutie::rubysys::value::RubySpecialConsts::Nil as usize)
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
    unsafe {
        let n = native.value();
        let push = CString::new("depth_push").unwrap();
        let pop = CString::new("depth_pop").unwrap();
        rclass::rb_define_singleton_method(n, push.as_ptr(), native_depth_push as CallbackPtr, 0);
        rclass::rb_define_singleton_method(n, pop.as_ptr(), native_depth_pop as CallbackPtr, 0);
    }
    native.define(|m| {
        m.def_self("parse_data", native_parse_data);
        m.def_self("new_scope", native_new_scope);
        m.def_self("import_scope_in", native_import_scope);
        m.def_self("register_type_in", native_register_type);
        m.def_self("render_pattern_in", native_render_pattern);
        m.def_self("constructors_in", native_constructors);
        m.def_self("parse_haskell", native_parse_haskell);
        m.def_self("constructor_info_in", native_constructor_info);
        m.def_self("stack_segment", native_stack_segment);
        m.def_self("stack_segment=", native_set_stack_segment);
        m.def_self("max_depth", native_max_depth);
        m.def_self("max_depth=", native_set_max_depth);
    });
    let object = Class::from_existing("Object");
    let mut matcher = native.define_nested_class("Matcher", Some(&object));
    // Instances are only ever created through `new` with wrapped data.
    matcher.undef_alloc_func();
    unsafe {
        let k = matcher.value();
        let new_name = CString::new("new").unwrap();
        rclass::rb_define_singleton_method(k, new_name.as_ptr(), matcher_new as CallbackPtr, 5);
        def_method(k, "call", matcher_call as CallbackPtr, -1);
        def_method(k, "prepare", matcher_prepare as CallbackPtr, -1);
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
