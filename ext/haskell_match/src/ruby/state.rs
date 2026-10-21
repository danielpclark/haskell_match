//! Process-wide state shared by the module functions and the matcher: the
//! type scopes, the recursion settings and the per-thread nesting depth.

use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{LazyLock, Mutex, MutexGuard};

use rutie::AnyException;

use super::error::{argument_error, HmError};
use crate::core::types::TypeEnv;

/// Type scopes: index 0 is the global registry; every other scope is a
/// snapshot of the global one at its creation plus its own declarations
/// (a compiled Haskell module gets one, like a Haskell module namespace).
static SCOPES: LazyLock<Mutex<Vec<TypeEnv>>> = LazyLock::new(|| Mutex::new(vec![TypeEnv::new()]));

/// The type scopes, locked.
pub fn scopes() -> MutexGuard<'static, Vec<TypeEnv>> {
    SCOPES.lock().unwrap_or_else(|p| p.into_inner())
}

/// The type environment of scope `id`.
pub fn scope_env(scopes: &mut [TypeEnv], id: usize) -> Result<&mut TypeEnv, AnyException> {
    scopes
        .get_mut(id)
        .ok_or_else(|| argument_error(&format!("unknown type scope {}", id)))
}

thread_local! {
    /// Nesting depth of native calls on this thread.
    static DEPTH: Cell<usize> = const { Cell::new(0) };
}

/// Every this many nested native calls, the next clause body runs in a fresh
/// Fiber.  Each Fiber brings its own VM stack and machine stack, so chained
/// segments make the recursion depth a matter of memory rather than of
/// Ruby's fixed stack size.  0 disables segmentation.
static STACK_SEGMENT: AtomicUsize = AtomicUsize::new(DEFAULT_STACK_SEGMENT);
const DEFAULT_STACK_SEGMENT: usize = 100;

/// Maximum nesting depth of native calls before `StackOverflowError` is
/// raised, so a runaway recursion fails instead of consuming all memory
/// (GHC has the same kind of limit).  0 means unlimited.
static MAX_DEPTH: AtomicUsize = AtomicUsize::new(DEFAULT_MAX_DEPTH);
const DEFAULT_MAX_DEPTH: usize = 250_000;

pub fn stack_segment() -> usize {
    STACK_SEGMENT.load(Ordering::Relaxed)
}

pub fn set_stack_segment(levels: usize) {
    STACK_SEGMENT.store(levels, Ordering::Relaxed);
}

pub fn max_depth() -> usize {
    MAX_DEPTH.load(Ordering::Relaxed)
}

pub fn set_max_depth(levels: usize) {
    MAX_DEPTH.store(levels, Ordering::Relaxed);
}

/// The nesting depth of native calls on this thread.
pub fn depth() -> usize {
    DEPTH.with(|d| d.get())
}

pub fn set_depth(depth: usize) {
    DEPTH.with(|d| d.set(depth));
}

/// Count one more nesting level and return the new depth, or the
/// `StackOverflowError` for a recursion past `max_depth`.
pub fn depth_push() -> Result<usize, HmError> {
    let depth = depth();
    let max = max_depth();
    if max > 0 && depth >= max {
        return Err(HmError::StackOverflow(format!(
            "recursion deeper than HaskellMatch.max_depth ({}); use tail calls, defer, or raise the limit",
            max
        )));
    }
    set_depth(depth + 1);
    Ok(depth + 1)
}

/// Undo one `depth_push`.
pub fn depth_pop() {
    DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
}
