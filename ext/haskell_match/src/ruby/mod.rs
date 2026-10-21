//! Ruby-facing API: the `HaskellMatch::Native` module (`native`, declared
//! with `#[rubymodule]`), the exceptions it raises (`error`, declared with
//! `#[rubyerror]`) and the `HaskellMatch::Native::Matcher` class defined
//! here.
//!
//! `Matcher` keeps a hand-written typed-data definition rather than
//! `#[rubyclass]`: it sets `RUBY_TYPED_FROZEN_SHAREABLE`, which a frozen
//! matcher (and the `Function` objects built on it) needs to pass
//! `Ractor.make_shareable`, and the derive data type has no way to add that
//! flag.  Its hot-path methods (`call`, `prepare`, `select`, `select_with`,
//! `run`) are raw `extern "C"` functions (argc/argv style) so that a call
//! allocates nothing on the Rust side and clause bodies are invoked from a
//! frame that owns no values needing `Drop` (a body may leave by exception,
//! `throw`, `break` or `return`).  The remaining methods go through
//! `methods!` and return `Result`s, raised once their values are dropped.

pub mod error;
pub mod native;
pub mod runtime;
pub mod state;

use std::ffi::CString;
use std::os::raw::c_int;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{LazyLock, OnceLock};

use rutie::derive::IntoException;
use rutie::rubysys::typed_data::{RUBY_TYPED_FREE_IMMEDIATELY, RUBY_TYPED_FROZEN_SHAREABLE};
use rutie::rubysys::{array, class as rclass, rstruct, value::ValueType};
use rutie::typed_data::DataTypeWrapper;
use rutie::types::{CallbackPtr, DataType, DataTypeFunction, Id, Value};
use rutie::{
    AnyException, AnyObject, Array, Boolean, Class, Exception, Integer, Module, Object, RString, VM,
};

use crate::core::error::{CoreError, ErrorKind};
use crate::core::exhaust;
use crate::core::parser::parse_pattern;
use crate::core::pretty::render_witness;
use crate::core::resolve::resolve_clause;
use crate::core::tree;
use crate::core::typecheck;
use crate::core::types::TypeEnv;
use error::{argument_error, HmError};
use runtime::{Bound, Matcher, RtError, STACK_BINDS};

static MATCHER_TYPE: LazyLock<MatcherType> = LazyLock::new(MatcherType::new);

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
        let k = Module::from_existing("HaskellMatch").get_nested_class("TailCall");
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

/// Invoke a clause body, either directly or (every `stack_segment` levels)
/// in a fresh Fiber.  Exceptions and non-local jumps are returned as the
/// `rb_protect` state for the caller to resume once it has cleaned up.
///
/// # Safety
/// Must run on a Ruby thread holding the GVL with no Rust values needing
/// `Drop` live in the caller (a `StackOverflowError` is raised from here).
unsafe fn invoke_body(body: Value, bound: &Bound, buf: &[Value]) -> Result<Value, i32> {
    let depth = match state::depth_push() {
        Ok(depth) => depth,
        Err(e) => raise(e),
    };
    let segment = state::stack_segment();
    let result = if segment > 0 && depth.is_multiple_of(segment) {
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
    state::set_depth(depth - 1);
    result
}

/// Typed-data definition for `Matcher`.  Unlike Rutie's `wrappable_struct!`
/// and `#[rubyclass]` it sets `RUBY_TYPED_FROZEN_SHAREABLE`, so a frozen
/// matcher (and the `Function` objects built on it) can be passed to
/// `Ractor.make_shareable`.
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

class!(RMatcher, "HaskellMatch::Native::Matcher");

// ---------------------------------------------------------------------------
// error helpers for the raw methods
// ---------------------------------------------------------------------------

/// Raise `e` from a raw method.  Callers hold no Rust values needing `Drop`
/// at this point: `VM::raise_ex` `longjmp`s past the frame.
fn raise(e: impl IntoException) -> ! {
    VM::raise_ex(e.into_exception());
    unreachable!()
}

fn raise_arg(msg: &str) -> ! {
    raise(argument_error(msg))
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

/// Report a failed match from a raw method: a `HaskellMatch` exception, an
/// `ArgumentError`, or the resumption of a pending Ruby exception or jump.
fn raise_rt(name: &str, e: RtError) -> ! {
    match e {
        RtError::Ruby(state) => unsafe { rb_jump_tag(state) },
        RtError::NoMatch => raise(HmError::NoMatch(format!(
            "Non-exhaustive patterns in {}",
            name
        ))),
        RtError::TypeMismatch { expected, got } => raise(HmError::TypeMismatch(format!(
            "{}: expected a value of type {} but got {} ({})",
            name,
            expected,
            inspect(got),
            class_name(got)
        ))),
        RtError::LazyFailed { clause, got } => raise(HmError::IrrefutablePattern(format!(
            "{}: irrefutable pattern failed for clause {}: {} does not match",
            name,
            clause + 1,
            inspect(got)
        ))),
        RtError::Arity { expected, got } => raise_arg(&format!(
            "wrong number of arguments (given {}, expected {})",
            got, expected
        )),
    }
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
    raise(AnyException::new(
        "RuntimeError",
        Some(&format!("haskell_match internal error: {}", msg)),
    ))
}

fn rstrings(items: &[String]) -> Array {
    let mut a = Array::with_capacity(items.len());
    for s in items {
        a.push(RString::new_utf8(s));
    }
    a
}

// ---------------------------------------------------------------------------
// Matcher construction
// ---------------------------------------------------------------------------

struct Built {
    matcher: Matcher,
    missing: Vec<String>,
    truncated: bool,
    redundant: Vec<usize>,
}

fn build(
    name: String,
    clauses: &Array,
    guarded: &Array,
    limit: usize,
    scope: usize,
) -> Result<Built, AnyException> {
    let n = clauses.length();
    if n == 0 {
        return Err(argument_error("at least one clause is required"));
    }
    if guarded.length() != n {
        return Err(argument_error("guard flags must have one entry per clause"));
    }
    let mut scopes = state::scopes();
    let env = state::scope_env(&mut scopes, scope)?;
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
// Matcher methods (`methods!`, returning `Result`s)
// ---------------------------------------------------------------------------

methods!(
    RMatcher,
    rtself,
    // Matcher.new(name, clauses, guarded_flags, witness_limit, scope):
    // compiles the clauses and wraps the matcher in an instance of the
    // receiver (`Matcher` or a subclass such as `Function`), with the
    // exhaustiveness report in `@missing`, `@missing_truncated` and
    // `@redundant`.
    fn matcher_new(
        name: RString,
        clauses: Array,
        guarded: Array,
        limit: Integer,
        scope: Integer,
    ) -> Result<AnyObject, AnyException> {
        let name = name?.to_string();
        let clauses = clauses?;
        let guarded = guarded?;
        let limit = limit.map(|i| i.to_i64()).unwrap_or(20).max(1) as usize;
        let scope = scope?.to_i64();
        if scope < 0 {
            return Err(argument_error("scope must not be negative"));
        }
        let built = build(name, &clauses, &guarded, limit, scope as usize)?;
        let klass = Class::from(rtself.value());
        let mut obj: AnyObject = klass.wrap_data(built.matcher, &*MATCHER_TYPE);
        obj.instance_variable_set("@missing", rstrings(&built.missing));
        obj.instance_variable_set("@missing_truncated", Boolean::new(built.truncated));
        let mut redundant = Array::with_capacity(built.redundant.len());
        for r in built.redundant {
            redundant.push(Integer::new(r as i64));
        }
        obj.instance_variable_set("@redundant", redundant);
        Ok(obj)
    },
    // matcher.arity -> number of arguments
    fn matcher_arity() -> Integer {
        Integer::new(matcher_of(rtself.value()).arity as i64)
    },
    // matcher.name -> the function's name
    fn matcher_name() -> RString {
        RString::new_utf8(&matcher_of(rtself.value()).name)
    },
    // matcher.names -> variables bound by each clause, in order
    fn matcher_names() -> Array {
        let m = matcher_of(rtself.value());
        let mut out = Array::with_capacity(m.names.len());
        for ns in &m.names {
            out.push(rstrings(ns));
        }
        out
    },
    // matcher.guarded -> whether each clause has a guard
    fn matcher_guarded() -> Array {
        let m = matcher_of(rtself.value());
        let mut out = Array::with_capacity(m.guarded.len());
        for g in &m.guarded {
            out.push(Boolean::new(*g));
        }
        out
    },
    // matcher.slots -> value slots of the decision tree
    fn matcher_slots() -> Integer {
        Integer::new(matcher_of(rtself.value()).n_slots as i64)
    },
    // matcher.tree -> human-readable dump of the decision tree
    fn matcher_tree() -> RString {
        let m = matcher_of(rtself.value());
        RString::new_utf8(&runtime::dump(&m.env, &m.tree, &m.names, 0))
    }
);

// ---------------------------------------------------------------------------
// Matcher hot-path methods (raw extern "C")
// ---------------------------------------------------------------------------

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
            let depth = match state::depth_push() {
                Ok(depth) => depth,
                Err(e) => raise(e),
            };
            array::rb_ary_push(out, Integer::new(depth as i64).value());
            out
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

unsafe fn def_method(klass: Value, name: &str, f: CallbackPtr, argc: c_int) {
    let cname = CString::new(name).unwrap();
    rclass::rb_define_method(klass, cname.as_ptr(), f, argc);
}

pub fn init() {
    // Nothing here keeps per-Ractor state: the type registry is behind a
    // mutex and matchers are immutable once built.
    unsafe { VM::ext_ractor_safe(true) };
    let mut native = native::functions::define_ruby();
    HmError::define_ruby();
    let object = Class::from_existing("Object");
    let mut matcher = native.define_nested_class("Matcher", Some(&object));
    // Instances are only ever created through `new` with wrapped data.
    matcher.undef_alloc_func();
    matcher.define(|k| {
        k.def_self("new", matcher_new);
        k.def("arity", matcher_arity);
        k.def("name", matcher_name);
        k.def("names", matcher_names);
        k.def("guarded", matcher_guarded);
        k.def("slots", matcher_slots);
        k.def("tree", matcher_tree);
    });
    unsafe {
        let k = matcher.value();
        def_method(k, "call", matcher_call as CallbackPtr, -1);
        def_method(k, "prepare", matcher_prepare as CallbackPtr, -1);
        def_method(k, "select", matcher_select as CallbackPtr, -1);
        def_method(k, "select_with", matcher_select_with as CallbackPtr, 2);
        def_method(k, "run", matcher_run as CallbackPtr, 3);
    }
    ids();
}
