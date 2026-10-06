//! Runtime evaluation of compiled decision trees over live Ruby values.
//!
//! The core `Tree` is translated once into an `RtTree` whose constructor
//! cases carry the Ruby class of each constructor and whose literal cases
//! carry a GC-pinned Ruby object for slow-path comparisons.  Matching then
//! never allocates until a clause is selected: list tails are represented as
//! `(array, offset)` views and materialised with `rb_ary_subseq` (which
//! shares the backing store) only when bound to a variable.

use rutie::rubysys::{array, class, rproc, rstruct, value::ValueType};
use rutie::types::Value;
use rutie::{AnyObject, Object, VM};

use std::collections::HashMap;

use crate::core::ast::{Lit, LitKey, LitKind, Pat};
use crate::core::tree::{BindSrc, LazyPat, Leaf, Tree};
use crate::core::types::{TypeEnv, TypeId, TypeKind};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SlotKind {
    /// An ordinary Ruby value.
    Plain,
    /// A list view: elements `off..` of the Array or String in `v`.
    View,
    /// Character `off` of the String in `v` (a list head not yet
    /// materialised as a one-character String).
    Char,
}

#[derive(Clone, Copy)]
pub struct Slot {
    pub v: Value,
    pub off: i64,
    pub kind: SlotKind,
}

impl Slot {
    #[inline]
    fn plain(v: Value) -> Slot {
        Slot {
            v,
            off: 0,
            kind: SlotKind::Plain,
        }
    }
    #[inline]
    fn empty() -> Slot {
        Slot::plain(Value::from(0))
    }
}

extern "C" {
    fn rb_str_substr(
        string: Value,
        begin: std::os::raw::c_long,
        len: std::os::raw::c_long,
    ) -> Value;
}

pub struct RtCase {
    pub handle: usize,
    pub tag: usize,
    pub arity: usize,
    pub base: usize,
    pub tree: RtTree,
}

pub struct RtLitCase {
    pub lit: Lit,
    /// Ruby object equal to the literal, pinned with
    /// `rb_gc_register_mark_object`.
    pub obj: Value,
    pub tree: RtTree,
}

pub enum RtTree {
    Fail,
    Leaf(Leaf),
    Guard {
        leaf: Leaf,
        fallback: Box<RtTree>,
    },
    SwitchCon {
        slot: usize,
        ty: TypeId,
        cases: Vec<RtCase>,
        default: Option<Box<RtTree>>,
    },
    SwitchLit {
        slot: usize,
        kind: LitKind,
        cases: Vec<RtLitCase>,
        default: Box<RtTree>,
    },
}

/// Why a match did not produce a clause.
pub enum RtError {
    /// A guard or body left through a Ruby exception or a non-local jump
    /// (`throw`, `break`, ...): the `rb_protect` state to resume with
    /// `rb_jump_tag` once all Rust frames have been unwound.
    Ruby(i32),
    /// No clause matched (partial function or all guards failed).
    NoMatch,
    /// Value is not of the type the patterns in this position expect.
    TypeMismatch { expected: String, got: Value },
    /// A lazy (`~`) pattern failed to destructure after the clause was chosen.
    LazyFailed { clause: usize, got: Value },
    /// Wrong number of arguments.
    Arity { expected: usize, got: usize },
}

pub struct Matcher {
    pub env: TypeEnv,
    pub tree: RtTree,
    pub arity: usize,
    pub n_slots: usize,
    pub names: Vec<Vec<String>>,
    pub guarded: Vec<bool>,
    pub name: String,
    /// Pinned Ruby objects for literals inside lazy patterns.
    pub lazy_lits: HashMap<LitKey, Value>,
}

/// Literals occurring inside lazy (`~`) patterns anywhere in the tree.
pub fn lazy_literals(t: &Tree, out: &mut Vec<Lit>) {
    fn in_pat(p: &Pat, out: &mut Vec<Lit>) {
        match p {
            Pat::Lit(l) => {
                if !out.iter().any(|x| x.same(l)) {
                    out.push(l.clone());
                }
            }
            Pat::As(_, p) | Pat::Lazy(p) => in_pat(p, out),
            Pat::Con(_, args) => args.iter().for_each(|a| in_pat(a, out)),
            Pat::Wild | Pat::Var(_) => {}
        }
    }
    fn in_leaf(l: &Leaf, out: &mut Vec<Lit>) {
        l.lazies.iter().for_each(|lz| in_pat(&lz.pat, out));
    }
    match t {
        Tree::Fail => {}
        Tree::Leaf(l) => in_leaf(l, out),
        Tree::Guard { leaf, fallback } => {
            in_leaf(leaf, out);
            lazy_literals(fallback, out);
        }
        Tree::SwitchCon { cases, default, .. } => {
            cases.iter().for_each(|c| lazy_literals(&c.tree, out));
            if let Some(d) = default {
                lazy_literals(d, out);
            }
        }
        Tree::SwitchLit { cases, default, .. } => {
            cases.iter().for_each(|(_, t)| lazy_literals(t, out));
            lazy_literals(default, out);
        }
    }
}

pub fn literal_object(l: &Lit) -> Value {
    let v = match l {
        Lit::Int(i) => rutie::Integer::new(*i).value(),
        Lit::Big(s) => {
            let cs = std::ffi::CString::new(s.as_str()).expect("digits contain no NUL");
            unsafe { rutie::rubysys::numeric::rb_cstr_to_inum(cs.as_ptr(), 10, 1) }
        }
        Lit::Float(f) => rutie::Float::new(*f).value(),
        Lit::Char(c) => unsafe {
            rutie::rubysys::string::rb_str_freeze(rutie::RString::new_utf8(&c.to_string()).value())
        },
        Lit::Sym(s) => rutie::Symbol::new(s).value(),
    };
    unsafe { rutie::rubysys::gc::rb_gc_register_mark_object(v) };
    v
}

pub fn translate(env: &TypeEnv, t: &Tree) -> RtTree {
    match t {
        Tree::Fail => RtTree::Fail,
        Tree::Leaf(l) => RtTree::Leaf(l.clone()),
        Tree::Guard { leaf, fallback } => RtTree::Guard {
            leaf: leaf.clone(),
            fallback: Box::new(translate(env, fallback)),
        },
        Tree::SwitchCon {
            slot,
            ty,
            cases,
            default,
        } => RtTree::SwitchCon {
            slot: *slot,
            ty: *ty,
            cases: cases
                .iter()
                .map(|c| RtCase {
                    handle: env.con(c.con).handle,
                    tag: c.tag,
                    arity: c.arity,
                    base: c.base,
                    tree: translate(env, &c.tree),
                })
                .collect(),
            default: default.as_ref().map(|d| Box::new(translate(env, d))),
        },
        Tree::SwitchLit {
            slot,
            kind,
            cases,
            default,
        } => RtTree::SwitchLit {
            slot: *slot,
            kind: *kind,
            cases: cases
                .iter()
                .map(|(l, t)| RtLitCase {
                    lit: l.clone(),
                    obj: literal_object(l),
                    tree: translate(env, t),
                })
                .collect(),
            default: Box::new(translate(env, default)),
        },
    }
}

#[inline]
unsafe fn ary_len(v: Value) -> i64 {
    array::rb_ary_len(v) as i64
}

#[inline]
fn is_array(v: Value) -> bool {
    v.ty() == ValueType::Array
}

#[inline]
fn is_string(v: Value) -> bool {
    v.ty() == ValueType::RString
}

#[inline]
fn truthy(v: Value) -> bool {
    !(v.is_nil() || v.is_false())
}

/// Number of elements of a list view: Arrays and Strings (`String = [Char]`).
#[inline]
unsafe fn list_len(slot: Slot) -> Option<i64> {
    if is_array(slot.v) {
        Some(ary_len(slot.v) - slot.off)
    } else if is_string(slot.v) && slot.kind != SlotKind::Char {
        Some(rutie::rubysys::string::rb_str_strlen(slot.v) as i64 - slot.off)
    } else {
        None
    }
}

/// Identify the constructor (by tag) of `slot` for type `ty`; `None` when the
/// value is not of that type at all.
#[inline]
unsafe fn identify(env: &TypeEnv, ty: TypeId, slot: Slot) -> Option<usize> {
    match env.ty(ty).kind {
        TypeKind::Adt => {
            if slot.kind != SlotKind::Plain {
                return None;
            }
            let klass = class::rb_obj_class(slot.v).value;
            let dt = env.ty(ty);
            for &c in &dt.cons {
                if env.con(c).handle == klass {
                    return Some(env.con(c).tag);
                }
            }
            None
        }
        TypeKind::Bool => {
            if slot.kind != SlotKind::Plain {
                None
            } else if slot.v.is_true() {
                Some(1)
            } else if slot.v.is_false() {
                Some(0)
            } else {
                None
            }
        }
        TypeKind::List => list_len(slot).map(|n| if n == 0 { 0 } else { 1 }),
        TypeKind::Tuple(n) => {
            if slot.kind != SlotKind::Char
                && is_array(slot.v)
                && (ary_len(slot.v) - slot.off) as usize == n
            {
                Some(0)
            } else {
                None
            }
        }
    }
}

/// Write the fields of the constructor at `slot` into `slots[base..]`.
#[inline]
unsafe fn fields_into(
    env: &TypeEnv,
    ty: TypeId,
    slot: Slot,
    arity: usize,
    base: usize,
    slots: &mut [Slot],
) {
    match env.ty(ty).kind {
        TypeKind::Adt => {
            for i in 0..arity {
                let idx = rutie::Integer::new(i as i64).value();
                slots[base + i] = Slot::plain(rstruct::rb_struct_aref(slot.v, idx));
            }
        }
        TypeKind::Bool => {}
        TypeKind::List => {
            if arity == 2 {
                slots[base] = if is_array(slot.v) {
                    Slot::plain(array::rb_ary_entry(slot.v, slot.off as _))
                } else {
                    Slot {
                        v: slot.v,
                        off: slot.off,
                        kind: SlotKind::Char,
                    }
                };
                slots[base + 1] = Slot {
                    v: slot.v,
                    off: slot.off + 1,
                    kind: SlotKind::View,
                };
            }
        }
        TypeKind::Tuple(_) => {
            for i in 0..arity {
                slots[base + i] =
                    Slot::plain(array::rb_ary_entry(slot.v, (slot.off + i as i64) as _));
            }
        }
    }
}

/// Turn a slot into the Ruby object a variable bound to it should see.
#[inline]
unsafe fn materialize(slot: Slot) -> Value {
    match slot.kind {
        SlotKind::Plain => slot.v,
        SlotKind::View => {
            if slot.off == 0 {
                slot.v
            } else if is_array(slot.v) {
                let len = ary_len(slot.v) - slot.off;
                array::rb_ary_subseq(slot.v, slot.off as _, len.max(0) as _)
            } else {
                let len = rutie::rubysys::string::rb_str_strlen(slot.v) as i64 - slot.off;
                rb_str_substr(slot.v, slot.off as _, len.max(0) as _)
            }
        }
        SlotKind::Char => rb_str_substr(slot.v, slot.off as _, 1),
    }
}

/// The character at `off` of an ASCII-only string, without allocating.
#[inline]
unsafe fn ascii_char_at(v: Value, off: i64) -> Option<u8> {
    if rutie::rubysys::string::rb_enc_str_asciionly_p(v) == 0 {
        return None;
    }
    let len = rutie::rubysys::string::rstring_len(v) as i64;
    if off < 0 || off >= len {
        return None;
    }
    let ptr = rutie::rubysys::string::rb_string_value_ptr(&v) as *const u8;
    Some(*ptr.add(off as usize))
}

unsafe fn lit_matches(kind: LitKind, case: &RtLitCase, slot: Slot) -> bool {
    match kind {
        LitKind::Num => {
            let v = slot.v;
            if v.is_fixnum() {
                let i = (v.value as i64) >> 1;
                return match case.lit {
                    Lit::Int(j) => i == j,
                    Lit::Float(f) => (i as f64) == f,
                    _ => false,
                };
            }
            if v.is_flonum() || v.ty() == ValueType::Float {
                let f = rutie::rubysys::float::rb_num2dbl(v);
                return match case.lit {
                    Lit::Int(j) => f == (j as f64),
                    Lit::Float(g) => f == g,
                    Lit::Big(_) => class::rb_equal(v, case.obj).is_true(),
                    _ => false,
                };
            }
            class::rb_equal(v, case.obj).is_true()
        }
        LitKind::Char => {
            let want = match case.lit {
                Lit::Char(c) => c,
                _ => return false,
            };
            if slot.kind == SlotKind::Char {
                if let Some(b) = ascii_char_at(slot.v, slot.off) {
                    return want.is_ascii() && b == want as u8;
                }
            }
            rutie::rubysys::string::rb_str_equal(materialize(slot), case.obj).is_true()
        }
        LitKind::Sym => slot.v.value == case.obj.value,
    }
}

unsafe fn lit_kind_ok(kind: LitKind, slot: Slot) -> bool {
    match kind {
        LitKind::Num => {
            let v = slot.v;
            slot.kind == SlotKind::Plain
                && (v.is_fixnum()
                    || v.is_flonum()
                    || matches!(v.ty(), ValueType::Float | ValueType::Bignum)
                    || class::rb_obj_is_kind_of(v, rutie::rubysys::builtins::rb_cNumeric).is_true())
        }
        LitKind::Char => {
            slot.kind == SlotKind::Char || (slot.kind == SlotKind::Plain && is_string(slot.v))
        }
        LitKind::Sym => {
            slot.kind == SlotKind::Plain && (slot.v.is_symbol() || slot.v.ty() == ValueType::Symbol)
        }
    }
}

/// Run `f` under `rb_protect`.  On a Ruby exception or non-local jump the
/// pending state is returned untouched (`$!` is left set) so the caller can
/// resume it with `rb_jump_tag` after cleaning up.
pub fn call_protected(f: impl FnOnce() -> Value) -> Result<Value, i32> {
    let mut f = Some(f);
    match VM::protect(|| AnyObject::from((f.take().expect("called once"))())) {
        Ok(v) => Ok(v.value()),
        Err(state) => Err(state),
    }
}

/// Where a leaf's bound values ended up.
pub enum Bound {
    /// `n` values in the caller-provided stack buffer.
    Stack(usize),
    /// Values in a Ruby array (used for clauses binding more variables than
    /// the stack buffer holds).
    Array(Value),
}

/// Capacity of the stack buffer callers hand to `select`.
pub const STACK_BINDS: usize = 64;

#[inline]
fn qnil() -> Value {
    Value::from(rutie::rubysys::value::RubySpecialConsts::Nil as usize)
}

/// Call `proc` with the bound values as positional arguments.
///
/// # Safety
/// `proc` must be a live Proc and `buf` the buffer `bound` refers to.  Not
/// protected: callers must hold no Rust values needing `Drop` when they call
/// this, since the proc may leave by exception or non-local jump.
#[inline]
pub unsafe fn invoke(proc: Value, bound: &Bound, buf: &[Value]) -> Value {
    match bound {
        Bound::Stack(n) => rproc::rb_proc_call_with_block(proc, *n as _, buf.as_ptr(), qnil()),
        Bound::Array(a) => rproc::rb_proc_call(proc, *a),
    }
}

/// The bound values as a fresh Ruby array.
///
/// # Safety
/// `buf` must be the buffer `bound` refers to, on a Ruby thread with the GVL.
pub unsafe fn bound_to_array(bound: &Bound, buf: &[Value]) -> Value {
    match bound {
        Bound::Stack(n) => array::rb_ary_new_from_values(*n as _, buf.as_ptr()),
        Bound::Array(a) => *a,
    }
}

impl Matcher {
    pub fn type_name(&self, ty: TypeId) -> String {
        self.env.ty(ty).name.clone()
    }

    /// Match `args`, returning the selected clause and where its bindings
    /// were written.  `guards` is a Ruby array (or nil) of procs indexed by
    /// clause; a missing or nil entry means "no guard".
    ///
    /// # Safety
    /// Must run on a Ruby thread holding the GVL; `args` must be live Ruby
    /// values (rooted by the caller) and `guards` a live `Array` or `nil`.
    pub unsafe fn select(
        &self,
        args: &[Value],
        guards: Value,
        buf: &mut [Value; STACK_BINDS],
    ) -> Result<(usize, Bound), RtError> {
        if args.len() != self.arity {
            return Err(RtError::Arity {
                expected: self.arity,
                got: args.len(),
            });
        }
        let mut stack = [Slot::empty(); 32];
        let mut heap: Vec<Slot> = Vec::new();
        let slots: &mut [Slot] = if self.n_slots <= 32 {
            &mut stack[..self.n_slots.max(1)]
        } else {
            heap.resize(self.n_slots, Slot::empty());
            &mut heap[..]
        };
        for (slot, arg) in slots.iter_mut().zip(args.iter()) {
            *slot = Slot::plain(*arg);
        }
        let mut node = &self.tree;
        loop {
            match node {
                RtTree::Fail => return Err(RtError::NoMatch),
                RtTree::Leaf(leaf) => {
                    let bound = self.bind(leaf, slots, buf)?;
                    return Ok((leaf.clause, bound));
                }
                RtTree::Guard { leaf, fallback } => {
                    let bound = self.bind(leaf, slots, buf)?;
                    let guard = if guards.is_nil() || ary_len(guards) <= leaf.clause as i64 {
                        qnil()
                    } else {
                        array::rb_ary_entry(guards, leaf.clause as _)
                    };
                    let ok = if guard.is_nil() {
                        true
                    } else {
                        let r =
                            call_protected(|| invoke(guard, &bound, buf)).map_err(RtError::Ruby)?;
                        truthy(r)
                    };
                    if ok {
                        return Ok((leaf.clause, bound));
                    }
                    node = fallback;
                }
                RtTree::SwitchCon {
                    slot,
                    ty,
                    cases,
                    default,
                } => {
                    let s = slots[*slot];
                    // A value that is not of this type matches no constructor
                    // pattern; wildcard rows (the default branch) still
                    // accept it, as `_` accepts anything in Haskell.  Without
                    // such rows no clause could ever match it: report the
                    // type error Haskell would have found statically.
                    match identify(&self.env, *ty, s) {
                        Some(tag) => match cases.iter().find(|c| c.tag == tag) {
                            Some(c) => {
                                fields_into(&self.env, *ty, s, c.arity, c.base, slots);
                                node = &c.tree;
                            }
                            None => match default {
                                Some(d) => node = d,
                                None => return Err(RtError::NoMatch),
                            },
                        },
                        None => match default {
                            Some(d) if !matches!(**d, RtTree::Fail) => node = d,
                            _ => {
                                return Err(RtError::TypeMismatch {
                                    expected: self.type_name(*ty),
                                    got: materialize(s),
                                })
                            }
                        },
                    }
                }
                RtTree::SwitchLit {
                    slot,
                    kind,
                    cases,
                    default,
                } => {
                    let s = slots[*slot];
                    if !lit_kind_ok(*kind, s) {
                        if matches!(**default, RtTree::Fail) {
                            return Err(RtError::TypeMismatch {
                                expected: kind.name().to_string(),
                                got: materialize(s),
                            });
                        }
                        node = default;
                        continue;
                    }
                    let mut next: &RtTree = default;
                    for c in cases {
                        if lit_matches(*kind, c, s) {
                            next = &c.tree;
                            break;
                        }
                    }
                    node = next;
                }
            }
        }
    }

    /// Write the bound values of a leaf into `buf` (or, for very wide
    /// clauses, into a fresh Ruby array).
    unsafe fn bind(
        &self,
        leaf: &Leaf,
        slots: &[Slot],
        buf: &mut [Value; STACK_BINDS],
    ) -> Result<Bound, RtError> {
        let n = leaf.binds.len();
        if n <= STACK_BINDS {
            for (i, b) in leaf.binds.iter().enumerate() {
                buf[i] = match b {
                    BindSrc::Slot(s) => materialize(slots[*s]),
                    BindSrc::Lazy(_) => qnil(),
                };
            }
            for lz in &leaf.lazies {
                let LazyPat { slot, pat } = lz;
                let mut store = |i: usize, v: Value| buf[i] = v;
                if !self.destructure(pat, slots[*slot], &mut store) {
                    return Err(RtError::LazyFailed {
                        clause: leaf.clause,
                        got: materialize(slots[*slot]),
                    });
                }
            }
            Ok(Bound::Stack(n))
        } else {
            let out = array::rb_ary_new_capa(n as _);
            for (i, b) in leaf.binds.iter().enumerate() {
                let v = match b {
                    BindSrc::Slot(s) => materialize(slots[*s]),
                    BindSrc::Lazy(_) => qnil(),
                };
                array::rb_ary_store(out, i as _, v);
            }
            for lz in &leaf.lazies {
                let LazyPat { slot, pat } = lz;
                let mut store = |i: usize, v: Value| array::rb_ary_store(out, i as _, v);
                if !self.destructure(pat, slots[*slot], &mut store) {
                    return Err(RtError::LazyFailed {
                        clause: leaf.clause,
                        got: materialize(slots[*slot]),
                    });
                }
            }
            Ok(Bound::Array(out))
        }
    }

    /// Interpretive match used for lazy patterns; variables go to `store`.
    unsafe fn destructure(
        &self,
        pat: &Pat,
        slot: Slot,
        store: &mut dyn FnMut(usize, Value),
    ) -> bool {
        match pat {
            Pat::Wild => true,
            Pat::Var(v) => {
                store(*v, materialize(slot));
                true
            }
            Pat::As(v, inner) => {
                store(*v, materialize(slot));
                self.destructure(inner, slot, store)
            }
            Pat::Lazy(inner) => self.destructure(inner, slot, store),
            Pat::Lit(l) => {
                let kind = l.kind();
                if !lit_kind_ok(kind, slot) {
                    return false;
                }
                let obj = match self.lazy_lits.get(&l.key()) {
                    Some(o) => *o,
                    None => literal_object(l),
                };
                let case = RtLitCase {
                    lit: l.clone(),
                    obj,
                    tree: RtTree::Fail,
                };
                lit_matches(kind, &case, slot)
            }
            Pat::Con(c, args) => {
                let con = self.env.con(*c);
                let ty = con.ty;
                match identify(&self.env, ty, slot) {
                    Some(tag) if tag == con.tag => {
                        let mut tmp = vec![Slot::empty(); con.arity];
                        fields_into(&self.env, ty, slot, con.arity, 0, &mut tmp);
                        args.iter()
                            .zip(tmp.iter())
                            .all(|(a, s)| self.destructure(a, *s, store))
                    }
                    _ => false,
                }
            }
        }
    }
}

/// Human-readable dump of a tree, for `Matcher#tree` and tests.
pub fn dump(env: &TypeEnv, t: &RtTree, names: &[Vec<String>], indent: usize) -> String {
    let pad = "  ".repeat(indent);
    match t {
        RtTree::Fail => format!("{}FAIL\n", pad),
        RtTree::Leaf(l) => format!(
            "{}=> clause {}{}\n",
            pad,
            l.clause + 1,
            dump_binds(l, names)
        ),
        RtTree::Guard { leaf, fallback } => format!(
            "{}guard clause {}{}\n{}",
            pad,
            leaf.clause + 1,
            dump_binds(leaf, names),
            dump(env, fallback, names, indent)
        ),
        RtTree::SwitchCon {
            slot,
            ty,
            cases,
            default,
        } => {
            let mut s = format!("{}switch slot {} ({})\n", pad, slot, env.ty(*ty).name);
            for c in cases {
                let con = env.con_by_tag(*ty, c.tag);
                let fields: Vec<String> = (0..c.arity).map(|i| format!("{}", c.base + i)).collect();
                s.push_str(&format!(
                    "{}  {}{}:\n",
                    pad,
                    env.con(con).name,
                    if fields.is_empty() {
                        String::new()
                    } else {
                        format!(" -> slots [{}]", fields.join(", "))
                    }
                ));
                s.push_str(&dump(env, &c.tree, names, indent + 2));
            }
            if let Some(d) = default {
                s.push_str(&format!("{}  _:\n", pad));
                s.push_str(&dump(env, d, names, indent + 2));
            }
            s
        }
        RtTree::SwitchLit {
            slot,
            kind,
            cases,
            default,
        } => {
            let mut s = format!("{}switch slot {} ({})\n", pad, slot, kind.name());
            for c in cases {
                s.push_str(&format!("{}  {}:\n", pad, c.lit));
                s.push_str(&dump(env, &c.tree, names, indent + 2));
            }
            s.push_str(&format!("{}  _:\n", pad));
            s.push_str(&dump(env, default, names, indent + 2));
            s
        }
    }
}

fn dump_binds(l: &Leaf, names: &[Vec<String>]) -> String {
    let ns = &names[l.clause];
    if ns.is_empty() {
        return String::new();
    }
    let parts: Vec<String> = l
        .binds
        .iter()
        .enumerate()
        .map(|(i, b)| match b {
            BindSrc::Slot(s) => format!("{}=slot {}", ns[i], s),
            BindSrc::Lazy(_) => format!("{}=lazy", ns[i]),
        })
        .collect();
    format!(" [{}]", parts.join(", "))
}
