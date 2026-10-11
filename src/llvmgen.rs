//! llvmgen: CDirac AST -> textual LLVM IR (N0 slice + native interop).
//!
//! Boxed values (`%Value*`, opaque `ptr`); every builtin is an n-ary
//! runtime call over an explicit arg array (`@cdi_add_n(ptr, i32)`).
//! Native calls (`extern-c`, `extern-cpp`, C++ classes/methods) lower
//! through generated `extern "C"` wrappers (`@cdi_wrap_N`, boxed
//! `Value*` in/out — see `native.rs` and `--emit-cpp-shim`); the IR
//! never names a C++-mangled symbol.
//! Static checks live here (no checker yet in this crate): unbound
//! identifiers, exact arity, set-before-def, set-to-const, one
//! namespace, return/break/continue placement, top-level-only procs.
//!
//! N0+N1 cover the full core: literals, var/set/const, all 62 builtins
//! (`+ - * /`, comparisons, integer trio, unary/small arithmetic,
//! predicates, containers, strings, I/O), short-circuit `and`/`or`,
//! value-position `if`, `if/else`, `while`, C-style `for`,
//! `break`/`continue`, `return`, `fn`/recursion, `machine.*` (all
//! builtins except `and`/`or`, plus machine-only `alloc`/`dealloc`).
//! Everything else errors as unbound/JVM-only with reference-style
//! messages.
//!
//! Output needs `clang` + `llvmrt/*.c` to assemble and run:
//! `clang out.ll llvmrt/value.c llvmrt/arith.c llvmrt/strings.c
//!   llvmrt/containers.c llvmrt/cinterop.c llvmrt/pred.c llvmrt/io.c
//!   llvmrt/mem.c -o prog`
//! (add `-lgc -DCDI_GC_BOEHM` with Boehm). With native directives,
//! also compile the `--emit-cpp-shim` output with `g++ -std=c++17`
//! and link it in (see the IR header comment).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::ast::{Node, NodeKind};
use crate::closures::{nest_key, TopItem};
use crate::native::{
    collect_native, is_native_directive, wrapper_arity, CallbackInfo, NativeDecls, NativeTy,
    Wrapper,
};

/// Builtins emitted by llvmgen (mirrors the runtime linked in).
/// Full core parity: every builtin in `docs/builtins.md` §1-11.
const BUILTINS: &[&str] = &[
    "+",
    "-",
    "*",
    "/",
    "=",
    "<",
    ">",
    "<=",
    ">=",
    "not",
    "display",
    "displayln",
    "array",
    "len",
    "get",
    "quotient",
    "remainder",
    "modulo",
    "add1",
    "sub1",
    "abs",
    "max",
    "min",
    "expt",
    "sqrt",
    "zero?",
    "positive?",
    "negative?",
    "even?",
    "odd?",
    "equal?",
    "number?",
    "real?",
    "integer?",
    "boolean?",
    "string?",
    "array?",
    "queue?",
    "empty?",
    "void?",
    "put",
    "push",
    "pop",
    "queue",
    "qpush",
    "qpush-front",
    "qpop",
    "qpop-back",
    "qpeek",
    "qpeek-back",
    "string-append",
    "string-length",
    "substring",
    "string-upcase",
    "string-downcase",
    "number->string",
    "string->number",
    "print",
    "newline",
    "read-line",
    "error",
    "void",
    "and",
    "or",
];

/// `machine`-only ops (explicit memory; bare `alloc` stays unbound).
const MACHINE_ONLY: &[&str] = &["alloc", "dealloc"];

fn rt_name(op: &str) -> &'static str {
    match op {
        "+" => "cdi_add_n",
        "-" => "cdi_sub_n",
        "*" => "cdi_mul_n",
        "/" => "cdi_div_n",
        "=" => "cdi_eq_n",
        "<" => "cdi_lt_n",
        ">" => "cdi_gt_n",
        "<=" => "cdi_le_n",
        ">=" => "cdi_ge_n",
        "not" => "cdi_not",
        "display" => "cdi_display",
        "displayln" => "cdi_displayln",
        "array" => "cdi_array_n",
        "len" => "cdi_len",
        "get" => "cdi_get",
        "quotient" => "cdi_quotient",
        "remainder" => "cdi_remainder",
        "modulo" => "cdi_modulo",
        "add1" => "cdi_add1",
        "sub1" => "cdi_sub1",
        "abs" => "cdi_abs",
        "max" => "cdi_max_n",
        "min" => "cdi_min_n",
        "expt" => "cdi_expt",
        "sqrt" => "cdi_sqrt",
        "zero?" => "cdi_zero_p",
        "positive?" => "cdi_positive_p",
        "negative?" => "cdi_negative_p",
        "even?" => "cdi_even_p",
        "odd?" => "cdi_odd_p",
        "equal?" => "cdi_equal_p",
        "number?" => "cdi_number_p",
        "real?" => "cdi_real_p",
        "integer?" => "cdi_integer_p",
        "boolean?" => "cdi_boolean_p",
        "string?" => "cdi_string_p",
        "array?" => "cdi_array_p",
        "queue?" => "cdi_queue_p",
        "empty?" => "cdi_empty_p",
        "void?" => "cdi_void_p",
        "put" => "cdi_put",
        "push" => "cdi_push",
        "pop" => "cdi_pop",
        "queue" => "cdi_queue_n",
        "qpush" => "cdi_qpush",
        "qpush-front" => "cdi_qpush_front",
        "qpop" => "cdi_qpop",
        "qpop-back" => "cdi_qpop_back",
        "qpeek" => "cdi_qpeek",
        "qpeek-back" => "cdi_qpeek_back",
        "string-append" => "cdi_string_append_n",
        "string-length" => "cdi_string_length",
        "substring" => "cdi_substring",
        "string-upcase" => "cdi_string_upcase",
        "string-downcase" => "cdi_string_downcase",
        "number->string" => "cdi_number_to_string",
        "string->number" => "cdi_string_to_number",
        "print" => "cdi_print",
        "newline" => "cdi_newline",
        "read-line" => "cdi_read_line",
        "error" => "cdi_error_n",
        "void" => "cdi_void",
        "alloc" => "cdi_machine_alloc",
        "dealloc" => "cdi_machine_dealloc",
        _ => unreachable!("rt_name covers exactly the builtin + machine tables"),
    }
}

#[derive(Clone)]
struct ProcSig {
    ll_name: String,
    arity: usize,
}

/// A lexically nested procedure lifted to a top-level function.
/// Captures are hidden leading `ptr` parameters holding caller slot
/// addresses (by reference: `set` writes through).
#[derive(Clone)]
struct NestedSig {
    ll_name: String,
    declared_arity: usize,
    /// Ordered `(name, is_const)` captures (positions are implicit).
    caps: Vec<(String, bool)>,
}

/// One lexical procedure frame (top-level proc, `@main` flow, or nested
/// body under generation). Nested defs register here sequentially.
struct ProcFrame {
    dirac_name: String,
    nested: Vec<(String, NestedSig)>,
}

struct LoopCtx {
    break_lbl: String,
    cont_lbl: String,
}

struct Scope {
    vars: HashMap<String, String>,
    consts: HashSet<String>,
}

struct Gen {
    out: String,
    tmp: usize,
    lbl: usize,
    uniq: usize,
    procs: HashMap<String, ProcSig>,
    globals: HashMap<String, String>,
    defined: HashSet<String>,
    consts_top: HashSet<String>,
    scopes: Vec<Scope>,
    loops: Vec<LoopCtx>,
    in_proc: bool,
    str_consts: usize,
    strtab: String,
    native: Rc<NativeDecls>,
    /// Lexical procedure frames (parallel to scopes regions).
    proc_stack: Vec<ProcFrame>,
    /// scopes.len() at each frame entry; lookups stay within the
    /// current frame (outer frames are different LLVM functions).
    frame_bases: Vec<usize>,
    /// Hoisted lifted bodies, flushed at end of module.
    lifted: Rc<RefCell<Vec<String>>>,
    /// Callback alias -> trampoline LLVM symbol (resolved in
    /// `compile_llvm` before any codegen; bare names box it).
    callback_tramps: HashMap<String, String>,
    /// Static capture sets from `closures::analyze` (nest key -> caps).
    nest_caps: HashMap<String, Vec<(String, bool)>>,
}

impl Gen {
    fn new() -> Self {
        Gen {
            out: String::new(),
            tmp: 0,
            lbl: 0,
            uniq: 0,
            procs: HashMap::new(),
            globals: HashMap::new(),
            defined: HashSet::new(),
            consts_top: HashSet::new(),
            scopes: Vec::new(),
            loops: Vec::new(),
            in_proc: false,
            str_consts: 0,
            strtab: String::new(),
            native: Rc::new(NativeDecls::default()),
            proc_stack: Vec::new(),
            frame_bases: Vec::new(),
            lifted: Rc::new(RefCell::new(Vec::new())),
            callback_tramps: HashMap::new(),
            nest_caps: HashMap::new(),
        }
    }

    fn emit(&mut self, line: &str) {
        self.out.push_str("  ");
        self.out.push_str(line);
        self.out.push('\n');
    }

    fn label(&mut self, name: String) {
        self.out.push_str(&name);
        self.out.push_str(":\n");
    }

    fn fresh_tmp(&mut self) -> String {
        self.tmp += 1;
        format!("%t{}", self.tmp)
    }

    fn fresh_lbl(&mut self, tag: &str) -> String {
        self.lbl += 1;
        format!("L{tag}{}", self.lbl)
    }

    fn push_scope(&mut self) {
        self.scopes.push(Scope {
            vars: HashMap::new(),
            consts: HashSet::new(),
        });
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn lookup_local(&self, name: &str) -> Option<String> {
        // Bounded by the current procedure frame: outer frames live in
        // different LLVM functions (their SSA values are unreachable).
        let base = self.frame_bases.last().copied().unwrap_or(0);
        let depth = self.scopes.len().saturating_sub(base);
        for sc in self.scopes.iter().rev().take(depth) {
            if let Some(a) = sc.vars.get(name) {
                return Some(a.clone());
            }
        }
        None
    }

    fn is_const(&self, name: &str) -> bool {
        if self.consts_top.contains(name) {
            return true;
        }
        for sc in self.scopes.iter().rev() {
            if sc.consts.contains(name) {
                return true;
            }
            if sc.vars.contains_key(name) {
                return false;
            }
        }
        false
    }

    fn define_local(&mut self, name: &str, is_const: bool) -> String {
        self.tmp += 1;
        let a = format!("%v{}", self.tmp);
        if let Some(sc) = self.scopes.last_mut() {
            sc.vars.insert(name.to_string(), a.clone());
            if is_const {
                sc.consts.insert(name.to_string());
            }
        }
        a
    }

    fn err_at(&self, node: &Node, msg: impl Into<String>) -> String {
        format!("line {}: {}", node.span.line, msg.into())
    }
}

fn head_of(node: &Node) -> &str {
    match &node.kind {
        NodeKind::List(elems, _) => match elems.first() {
            Some(first) => match &first.kind {
                NodeKind::Sym(s) => s.as_str(),
                _ => "",
            },
            None => "",
        },
        _ => "",
    }
}

fn is_builtin(name: &str) -> bool {
    BUILTINS.contains(&name)
}

/// Crate-visible builtin test for the closure prepass (`closures.rs`).
pub(crate) fn is_builtin_name(name: &str) -> bool {
    is_builtin(name)
}

/// `machine` members: every builtin except `and`/`or` (the reference
/// rejects those: write `&&` / `||` instead), plus the machine-only
/// memory ops.
fn is_machine_op(name: &str) -> bool {
    (is_builtin(name) && name != "and" && name != "or") || MACHINE_ONLY.contains(&name)
}

fn escape_ir_bytes(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'\\' => out.push_str("\\5C"),
            b'"' => out.push_str("\\22"),
            0x20..=0x7E => out.push(b as char),
            _ => out.push_str(&format!("\\{b:02X}")),
        }
    }
    out
}

/// Emit a string literal: global constant + `cdi_mk_str` call.
fn gen_str_lit(g: &mut Gen, s: &str) -> Result<String, String> {
    let n = s.len();
    g.str_consts += 1;
    let gname = format!("@.str.{}", g.str_consts);
    // String bytes live in globals flushed before any `define`.
    g.strtab.push_str(&format!(
        "{gname} = private unnamed_addr constant [{n} x i8] c\"{}\"\n",
        escape_ir_bytes(s)
    ));
    let p = g.fresh_tmp();
    g.emit(&format!(
        "{p} = getelementptr [{n} x i8], ptr {gname}, i32 0, i32 0"
    ));
    let v = g.fresh_tmp();
    g.emit(&format!("{v} = call ptr @cdi_mk_str(ptr {p}, i32 {n})"));
    Ok(v)
}

// ------------------------------------------------------------ values

/// Emit an n-ary builtin call over an explicit arg array.
fn gen_nary(g: &mut Gen, rt: &str, args: &[Node]) -> Result<String, String> {
    let n = args.len();
    let arr = if n == 0 {
        "null".to_string()
    } else {
        let a = g.fresh_tmp();
        g.emit(&format!("{a} = alloca [{n} x ptr]"));
        for (i, arg) in args.iter().enumerate() {
            let v = gen_value(g, arg)?;
            let slot = g.fresh_tmp();
            g.emit(&format!(
                "{slot} = getelementptr [{n} x ptr], ptr {a}, i32 0, i32 {i}"
            ));
            g.emit(&format!("store ptr {v}, ptr {slot}"));
        }
        a
    };
    let v = g.fresh_tmp();
    g.emit(&format!("{v} = call ptr @{rt}(ptr {arr}, i32 {n})"));
    Ok(v)
}

fn gen_value(g: &mut Gen, node: &Node) -> Result<String, String> {
    match &node.kind {
        NodeKind::Int(n) => {
            let v = g.fresh_tmp();
            g.emit(&format!("{v} = call ptr @cdi_mk_int(i32 {n})"));
            Ok(v)
        }
        NodeKind::Float(x) => {
            let v = g.fresh_tmp();
            g.emit(&format!(
                "{v} = call ptr @cdi_mk_float(double 0x{:016X})",
                x.to_bits()
            ));
            Ok(v)
        }
        NodeKind::Bool(b) => {
            let v = g.fresh_tmp();
            g.emit(&format!(
                "{v} = call ptr @cdi_mk_bool(i1 {})",
                if *b { "true" } else { "false" }
            ));
            Ok(v)
        }
        NodeKind::Str(s) => gen_str_lit(g, s),
        NodeKind::Nil => {
            let v = g.fresh_tmp();
            g.emit(&format!("{v} = call ptr @cdi_nil()"));
            Ok(v)
        }
        NodeKind::Sym(name) => {
            // Callback-anchored procs read as trampoline addresses
            // (boxed handles) — the one way a proc name is a value.
            if let Some(cb_ll) = g.callback_tramps.get(name.as_str()).cloned() {
                let v = g.fresh_tmp();
                g.emit(&format!("{v} = call ptr @cdi_from_ptr(ptr @{cb_ll})"));
                return Ok(v);
            }
            if g.procs.contains_key(name.as_str()) || find_nested(g, name.as_str()).is_some() {
                return Err(g.err_at(node, format!("'{name}' is a procedure, not a value")));
            }
            if g.native.classes.contains_key(name.as_str()) {
                return Err(g.err_at(
                    node,
                    format!("'{name}' is a cpp-class, not a value (use {name}(...) to construct)"),
                ));
            }
            if g.native.structs.contains_key(name.as_str()) {
                return Err(g.err_at(
                    node,
                    format!("'{name}' is a cpp-struct, not a value (pass an array of its fields)"),
                ));
            }
            if let Some(a) = g.lookup_local(name) {
                let v = g.fresh_tmp();
                g.emit(&format!("{v} = load ptr, ptr {a}"));
                return Ok(v);
            }
            if let Some(f) = g.globals.get(name.as_str()).cloned() {
                if !g.in_proc && !g.defined.contains(name.as_str()) {
                    return Err(g.err_at(node, format!("unbound identifier: {name}")));
                }
                let v = g.fresh_tmp();
                g.emit(&format!("{v} = load ptr, ptr {f}"));
                return Ok(v);
            }
            // Named C++ constants read as values (lookup-first: procs,
            // classes, locals, and globals above all win).
            if let Some(w) = g.native.clone().find_const(name.as_str()).cloned() {
                return Ok(gen_wrap_call(g, &w, Vec::new()));
            }
            if name.contains('.') {
                // C++ field sugar (`obj.field`, `Class.field`) resolves
                // before the JVM-only fallback below.
                if let Some((recv, field)) = split_one_dot(name) {
                    let nat = g.native.clone();
                    if nat.classes.contains_key(recv) {
                        match nat.find_static_field(recv, field).cloned() {
                            Some(w) => return Ok(gen_wrap_call(g, &w, Vec::new())),
                            None => {
                                if nat.find_field(recv, field).is_some() {
                                    return Err(g.err_at(
                                        node,
                                        format!("'{field}' of class '{recv}' is an instance field (read it on a handle, or use {recv}_{field}_get)"),
                                    ));
                                }
                                return Err(g.err_at(
                                    node,
                                    format!("unknown cpp static '{field}' of class '{recv}' (declare [cpp-static-field {recv} {field} ...])"),
                                ));
                            }
                        }
                    }
                    if is_value_bound(g, recv) {
                        match nat.find_field_by_name(field) {
                            Some((class, _)) => {
                                let w = nat
                                    .find_field(&class, field)
                                    .cloned()
                                    .expect("field getter registered");
                                let r = load_bound(g, node, recv)?;
                                let v = gen_wrap_call(g, &w, vec![r]);
                                return Ok(v);
                            }
                            None => {
                                return Err(g.err_at(
                                    node,
                                    format!(
                                        "unknown cpp field '{field}' (no cpp-field declares it)"
                                    ),
                                ))
                            }
                        }
                    }
                }
                return Err(g.err_at(
                    node,
                    format!("{name} is JVM-only (native has no Java fields)"),
                ));
            }
            Err(g.err_at(node, format!("unbound identifier: {name}")))
        }
        NodeKind::Var { .. } | NodeKind::Const { .. } | NodeKind::Set { .. } => {
            Err(g.err_at(node, "var/const/set in expression position"))
        }
        NodeKind::Proc { .. } => Err(g.err_at(node, "procedures must be top-level")),
        NodeKind::List(elems, tail) => {
            if elems.is_empty() {
                return Err(g.err_at(node, "nil literal [] in expression position"));
            }
            if tail.is_some() {
                return Err(g.err_at(node, format!("cannot call a dotted form: {}", node.print())));
            }
            let h = match &elems[0].kind {
                NodeKind::Sym(s) => s.clone(),
                _ => return Err(g.err_at(node, "computed call in head position")),
            };
            gen_call(g, node, &h, &elems[1..])
        }
    }
}

/// Call by head name: procs (exact arity), N0 builtins, `machine.*`.
fn gen_call(g: &mut Gen, node: &Node, h: &str, args: &[Node]) -> Result<String, String> {
    // `machine` namespace: member must be a machine op (same table as
    // bare calls minus `and`/`or`, plus the machine-only memory ops).
    let (h, args, was_machine) = match h {
        "machine" => {
            let member = match args.first() {
                Some(m) => match &m.kind {
                    NodeKind::Sym(s) => s.clone(),
                    _ => {
                        return Err(g.err_at(
                            node,
                            "machine: expects an operation name, e.g. machine.displayln(x)",
                        ))
                    }
                },
                None => {
                    return Err(g.err_at(
                        node,
                        "machine: expects an operation name, e.g. machine.displayln(x)",
                    ))
                }
            };
            if !is_machine_op(&member) {
                return Err(g.err_at(node, format!("unknown machine operation '{member}'")));
            }
            (member, &args[1..], true)
        }
        _ => (h.to_string(), args, false),
    };
    if !was_machine {
        // Lexically nested procedures shadow top-level ones; captures
        // (hidden leading slot addresses) resolve in the caller's frame.
        if let Some(sig) = find_nested(g, h.as_str()) {
            if sig.declared_arity != args.len() {
                return Err(g.err_at(
                    node,
                    format!(
                        "arity mismatch in call to {h}: expects {}, got {}",
                        sig.declared_arity,
                        args.len()
                    ),
                ));
            }
            let mut rendered = Vec::new();
            for (cn, _) in &sig.caps.clone() {
                match g.lookup_local(cn) {
                    // By reference: pass the slot address itself (the
                    // callee's capture param doubles as the slot).
                    Some(a) => rendered.push(a),
                    None => {
                        return Err(
                            g.err_at(node, format!("cannot capture '{cn}' here (out of scope)"))
                        )
                    }
                }
            }
            for a in args {
                rendered.push(gen_value(g, a)?);
            }
            let v = g.fresh_tmp();
            let params = rendered
                .iter()
                .map(|r| format!("ptr {r}"))
                .collect::<Vec<_>>()
                .join(", ");
            g.emit(&format!("{v} = call ptr @{}({params})", sig.ll_name));
            return Ok(v);
        }
        if let Some(p) = g.procs.get(h.as_str()).cloned() {
            if p.arity != args.len() {
                return Err(g.err_at(
                    node,
                    format!(
                        "arity mismatch in call to {h}: expects {}, got {}",
                        p.arity,
                        args.len()
                    ),
                ));
            }
            let mut rendered = Vec::new();
            for a in args {
                rendered.push(gen_value(g, a)?);
            }
            let v = g.fresh_tmp();
            let params = rendered
                .iter()
                .map(|r| format!("ptr {r}"))
                .collect::<Vec<_>>()
                .join(", ");
            g.emit(&format!("{v} = call ptr @{}({params})", p.ll_name));
            return Ok(v);
        }
    }
    // Native interop (C++-first): class statics, extern aliases +
    // constructor overloads (alias == class name), and `recv.method`
    // sugar over bound handles. Dirac procs already won above.
    if !was_machine {
        let nat = g.native.clone();
        // `Class.method(...)` static sugar (C-surface `Class.method(...)`
        // desugars to `[Class method ...]`). Only fires when `member` is
        // actually declared on the class; otherwise this is an ordinary
        // call (e.g. a constructor taking a variable first argument).
        if nat.classes.contains_key(h.as_str()) {
            if let Some(mnode) = args.first() {
                if let NodeKind::Sym(member) = &mnode.kind {
                    let member = member.clone();
                    let named: Vec<&Wrapper> = nat
                        .wrappers
                        .iter()
                        .filter(|w| match &w.kind {
                            crate::native::WrapperKind::Method {
                                class: c,
                                method: m,
                                ..
                            } => c.as_str() == h.as_str() && m == &member,
                            _ => false,
                        })
                        .collect();
                    if !named.is_empty() {
                        match nat.find_static(h.as_str(), &member, args.len() - 1) {
                            Some(w) => {
                                let w = w.clone();
                                let mut rendered = Vec::new();
                                for a in &args[1..] {
                                    rendered.push(gen_value(g, a)?);
                                }
                                return Ok(gen_wrap_call(g, &w, rendered));
                            }
                            None => {
                                // Qualified instance call: `Class.method(handle,
                                // ...)` reuses the instance wrapper with an
                                // explicit handle (always unambiguous).
                                if args.len() >= 2 {
                                    if let Some(w) = nat
                                        .find_instance(h.as_str(), &member, args.len() - 2)
                                        .cloned()
                                    {
                                        let mut rendered = Vec::new();
                                        for a in &args[1..] {
                                            rendered.push(gen_value(g, a)?);
                                        }
                                        return Ok(gen_wrap_call(g, &w, rendered));
                                    }
                                }
                                if named.iter().any(|w| {
                                    !matches!(
                                        &w.kind,
                                        crate::native::WrapperKind::Method {
                                            is_static: true,
                                            ..
                                        }
                                    )
                                }) {
                                    return Err(g.err_at(
                                        node,
                                        format!("method '{member}' of class '{h}' is an instance method (write {h}.{member}(handle, ...) or {h}_{member}(handle, ...))"),
                                    ));
                                }
                                let mut arities: Vec<usize> =
                                    named.iter().map(|w| wrapper_arity(w) - 1).collect();
                                arities.sort_unstable();
                                arities.dedup();
                                let want: Vec<String> =
                                    arities.iter().map(|a| a.to_string()).collect();
                                return Err(g.err_at(
                                    node,
                                    format!(
                                        "arity mismatch in call to {h}.{member}: expects {} (got {})",
                                        want.join(" or "),
                                        args.len() - 1
                                    ),
                                ));
                            }
                        }
                    }
                }
            }
        }
        // Extern aliases + constructors (`Class(...)` registers the
        // class name as its own alias), with arity overload dispatch.
        // Vector builds (`Alias(v0, ...)`) match any arity, after fixed
        // overloads (so `Alias()` still constructs empty). Varargs
        // (`extern-c f … varargs`) match fixed..=fixed+MAX next.
        if !nat.alias_overloads(h.as_str()).is_empty() {
            match nat.find_alias_arity(h.as_str(), args.len()) {
                Some(w) => {
                    let w = w.clone();
                    let mut rendered = Vec::new();
                    for a in args {
                        rendered.push(gen_value(g, a)?);
                    }
                    return Ok(gen_wrap_call(g, &w, rendered));
                }
                None => {
                    if let Some(w) = nat.find_varargs(h.as_str(), args.len()).cloned() {
                        return gen_native_nary(g, &w, args);
                    }
                    if let Some(w) = nat.find_vec_build(h.as_str()).cloned() {
                        return gen_native_nary(g, &w, args);
                    }
                    let mut arities: Vec<usize> = nat
                        .alias_overloads(h.as_str())
                        .into_iter()
                        .filter(|w| {
                            !matches!(&w.kind, crate::native::WrapperKind::VecBuild { .. })
                                && !crate::native::is_varargs(&w.kind)
                        })
                        .map(wrapper_arity)
                        .collect();
                    let mut mins: Vec<usize> = nat
                        .alias_overloads(h.as_str())
                        .into_iter()
                        .filter(|w| crate::native::is_varargs(&w.kind))
                        .map(|w| w.params.len())
                        .collect();
                    arities.sort_unstable();
                    arities.dedup();
                    mins.sort_unstable();
                    mins.dedup();
                    let mut want: Vec<String> = arities.iter().map(|a| a.to_string()).collect();
                    for m in &mins {
                        want.push(format!("{m} or more"));
                    }
                    let want = if want.is_empty() {
                        "unknown".to_string()
                    } else {
                        want.join(" or ")
                    };
                    return Err(g.err_at(
                        node,
                        format!(
                            "arity mismatch in call to {h}: expects {want} (got {})",
                            args.len()
                        ),
                    ));
                }
            }
        }
        // `recv.method(...)` instance sugar: the head reads as a value
        // and the member is a declared instance method. Only fires when
        // the program declares instance methods at all; anything else
        // falls through to the builtin/unbound errors below (typos stay
        // typos in pure-Dirac programs).
        if let Some(mnode) = args.first() {
            if let NodeKind::Sym(member) = &mnode.kind {
                let member = member.clone();
                let any_instance = nat.wrappers.iter().any(|w| {
                    matches!(
                        &w.kind,
                        crate::native::WrapperKind::Method {
                            is_static: false,
                            ..
                        }
                    )
                });
                if any_instance && is_value_bound(g, h.as_str()) {
                    let any_named = nat.wrappers.iter().any(|w| match &w.kind {
                        crate::native::WrapperKind::Method {
                            method: m,
                            is_static: false,
                            ..
                        } => m == &member,
                        _ => false,
                    });
                    if !any_named {
                        return Err(g.err_at(
                            node,
                            format!("unknown cpp method '{member}' (no cpp-method declares it)"),
                        ));
                    }
                    let cands = nat.method_candidates(&member, args.len());
                    if cands.len() == 1 {
                        let w = cands[0].clone();
                        let recv = load_bound(g, node, h.as_str())?;
                        let mut rendered = vec![recv];
                        for a in &args[1..] {
                            rendered.push(gen_value(g, a)?);
                        }
                        return Ok(gen_wrap_call(g, &w, rendered));
                    }
                    if cands.len() > 1 {
                        let owners: Vec<String> = cands
                            .iter()
                            .filter_map(|w| match &w.kind {
                                crate::native::WrapperKind::Method { class, .. } => {
                                    Some(class.clone())
                                }
                                _ => None,
                            })
                            .collect();
                        return Err(g.err_at(
                            node,
                            format!(
                                "ambiguous cpp method '{member}' ({} define it; call Class_{member}(recv, ...) explicitly)",
                                owners.join(", ")
                            ),
                        ));
                    }
                    let mut arities: Vec<usize> = nat
                        .wrappers
                        .iter()
                        .filter_map(|w| match &w.kind {
                            crate::native::WrapperKind::Method {
                                method: m,
                                is_static: false,
                                ..
                            } if m == &member => Some(wrapper_arity(w) - 1),
                            _ => None,
                        })
                        .collect();
                    arities.sort_unstable();
                    arities.dedup();
                    let want: Vec<String> = arities.iter().map(|a| a.to_string()).collect();
                    return Err(g.err_at(
                        node,
                        format!(
                            "arity mismatch in call to {h}.{member}: expects {} (got {})",
                            want.join(" or "),
                            args.len() - 1
                        ),
                    ));
                }
            }
        }
    }
    // Statement-only / removed / future heads.
    match h.as_str() {
        "var" | "set" | "const" | "while" | "for" => {
            Err(g.err_at(node, format!("{h} in expression position")))
        }
        "break" => Err(g.err_at(node, "break outside a loop")),
        "continue" => Err(g.err_at(node, "continue outside a loop")),
        "return" => Err(g.err_at(
            node,
            if g.in_proc {
                "return in expression position"
            } else {
                "return outside a procedure"
            }
            .to_string(),
        )),
        "proc" => Err(g.err_at(node, "procedures must be top-level")),
        "if" => {
            // Rebuild the head-inclusive form for branch splitting.
            let mut full = vec![Node::new(NodeKind::Sym("if".to_string()), node.span)];
            full.extend(args.iter().cloned());
            gen_value_if(g, node, &full)
        }
        "and" | "or" => gen_and_or(g, h.as_str() == "and", args),
        "new" | "import-java" | "requires-java" => Err(g.err_at(
            node,
            format!("{h} is JVM-only (native has no Java classes)"),
        )),
        "elif" | "else" => Err(g.err_at(node, format!("'{h}' is an if-separator, not a form"))),
        "alloc" | "dealloc" => {
            // Machine-only memory ops: bare `alloc` stays unbound.
            if !was_machine {
                return Err(g.err_at(node, format!("unbound identifier: {h}")));
            }
            let a = one_arg(g, node, h.as_str(), args)?;
            let v = g.fresh_tmp();
            g.emit(&format!("{v} = call ptr @{}(ptr {a})", rt_name(h.as_str())));
            Ok(v)
        }
        op if is_builtin(op) => {
            // Fixed single-arg runtime calls.
            match op {
                "not" => {
                    let a = one_arg(g, node, op, args)?;
                    let v = g.fresh_tmp();
                    g.emit(&format!("{v} = call ptr @cdi_not(ptr {a})"));
                    Ok(v)
                }
                "display" => {
                    let a = one_arg(g, node, op, args)?;
                    let v = g.fresh_tmp();
                    g.emit(&format!("{v} = call ptr @cdi_display(ptr {a})"));
                    Ok(v)
                }
                "displayln" => {
                    let a = one_arg(g, node, op, args)?;
                    let v = g.fresh_tmp();
                    g.emit(&format!("{v} = call ptr @cdi_displayln(ptr {a})"));
                    Ok(v)
                }
                "print" => {
                    let a = one_arg(g, node, op, args)?;
                    let v = g.fresh_tmp();
                    g.emit(&format!("{v} = call ptr @cdi_print(ptr {a})"));
                    Ok(v)
                }
                "len" => {
                    let a = one_arg(g, node, op, args)?;
                    let v = g.fresh_tmp();
                    g.emit(&format!("{v} = call ptr @cdi_len(ptr {a})"));
                    Ok(v)
                }
                "add1" | "sub1" | "abs" | "sqrt" | "zero?" | "positive?" | "negative?"
                | "even?" | "odd?" | "number?" | "real?" | "integer?" | "boolean?" | "string?"
                | "array?" | "queue?" | "empty?" | "void?" | "string-length" | "string-upcase"
                | "string-downcase" | "number->string" | "string->number" | "pop" | "qpop"
                | "qpop-back" | "qpeek" | "qpeek-back" => {
                    let a = one_arg(g, node, op, args)?;
                    let v = g.fresh_tmp();
                    g.emit(&format!("{v} = call ptr @{}(ptr {a})", rt_name(op)));
                    Ok(v)
                }
                "get" | "quotient" | "remainder" | "modulo" | "expt" | "equal?" | "push"
                | "qpush" | "qpush-front" => {
                    let (a, b) = two_args(g, node, op, args)?;
                    let v = g.fresh_tmp();
                    g.emit(&format!(
                        "{v} = call ptr @{}(ptr {a}, ptr {b})",
                        rt_name(op)
                    ));
                    Ok(v)
                }
                "put" => {
                    let (a, b, c) = three_args(g, node, op, args)?;
                    let v = g.fresh_tmp();
                    g.emit(&format!(
                        "{v} = call ptr @cdi_put(ptr {a}, ptr {b}, ptr {c})"
                    ));
                    Ok(v)
                }
                "substring" => {
                    if args.len() != 2 && args.len() != 3 {
                        return Err(g.err_at(
                            node,
                            format!("substring: expects 2..3 argument(s), got {}", args.len()),
                        ));
                    }
                    let s = gen_value(g, &args[0])?;
                    let i = gen_value(g, &args[1])?;
                    let v = g.fresh_tmp();
                    if args.len() == 3 {
                        let j = gen_value(g, &args[2])?;
                        g.emit(&format!(
                            "{v} = call ptr @cdi_substring(ptr {s}, ptr {i}, ptr {j}, i1 true)"
                        ));
                    } else {
                        g.emit(&format!(
                            "{v} = call ptr @cdi_substring(ptr {s}, ptr {i}, ptr null, i1 false)"
                        ));
                    }
                    Ok(v)
                }
                "void" | "newline" | "read-line" => {
                    if !args.is_empty() {
                        return Err(g.err_at(
                            node,
                            format!("{op}: expects 0 argument(s), got {}", args.len()),
                        ));
                    }
                    let v = g.fresh_tmp();
                    g.emit(&format!("{v} = call ptr @{}()", rt_name(op)));
                    Ok(v)
                }
                "error" => gen_nary(g, rt_name(op), args),
                _ => gen_nary(g, rt_name(op), args),
            }
        }
        _ => Err(g.err_at(node, format!("unbound identifier: {h}"))),
    }
}

/// Short-circuit `and`/`or` over boxed values: each operand evaluates
/// in its own block; the first decisive test jumps to the join carrying
/// its value (`and` stops at the first falsy, `or` at the first truthy;
/// only `false` is falsy, so a decisive edge carries exactly the
/// reference yield). `[and]` is true, `[or]` is false; single operands
/// need no branch.
fn gen_and_or(g: &mut Gen, is_and: bool, args: &[Node]) -> Result<String, String> {
    if args.is_empty() {
        let v = g.fresh_tmp();
        g.emit(&format!(
            "{v} = call ptr @cdi_mk_bool(i1 {})",
            if is_and { "true" } else { "false" }
        ));
        return Ok(v);
    }
    if args.len() == 1 {
        return gen_value(g, &args[0]);
    }
    let done = g.fresh_lbl("logicend");
    let mut incoming: Vec<(String, String)> = Vec::new();
    for arg in &args[..args.len() - 1] {
        let t = gen_value(g, arg)?;
        let c = g.fresh_tmp();
        g.emit(&format!("{c} = call i1 @cdi_truthy(ptr {t})"));
        let short = g.fresh_lbl("logicshort");
        let more = g.fresh_lbl("logicmore");
        // `and` continues on truthy, `or` on falsy; the other edge
        // carries this operand's value straight to the join.
        let (t_lbl, f_lbl) = if is_and {
            (more.clone(), short.clone())
        } else {
            (short.clone(), more.clone())
        };
        g.emit(&format!("br i1 {c}, label %{t_lbl}, label %{f_lbl}"));
        g.label(short.clone());
        g.emit(&format!("br label %{done}"));
        incoming.push((t, short));
        g.label(more);
    }
    // Fallthrough block: evaluates the last operand (the `and` yield /
    // `or` tail) with a known predecessor name for the phi.
    let fall = g.fresh_lbl("logicfall");
    g.emit(&format!("br label %{fall}"));
    g.label(fall.clone());
    let last = gen_value(g, &args[args.len() - 1])?;
    g.emit(&format!("br label %{done}"));
    incoming.push((last, fall));
    g.label(done.clone());
    let r = g.fresh_tmp();
    let arms = incoming
        .iter()
        .map(|(v, l)| format!("[{v}, %{l}]"))
        .collect::<Vec<_>>()
        .join(", ");
    g.emit(&format!("{r} = phi ptr {arms}"));
    Ok(r)
}

fn one_arg(g: &mut Gen, node: &Node, op: &str, args: &[Node]) -> Result<String, String> {
    if args.len() != 1 {
        return Err(g.err_at(
            node,
            format!("{op}: expects 1 argument, got {}", args.len()),
        ));
    }
    gen_value(g, &args[0])
}

fn two_args(g: &mut Gen, node: &Node, op: &str, args: &[Node]) -> Result<(String, String), String> {
    if args.len() != 2 {
        return Err(g.err_at(
            node,
            format!("{op}: expects 2 argument(s), got {}", args.len()),
        ));
    }
    Ok((gen_value(g, &args[0])?, gen_value(g, &args[1])?))
}

fn three_args(
    g: &mut Gen,
    node: &Node,
    op: &str,
    args: &[Node],
) -> Result<(String, String, String), String> {
    if args.len() != 3 {
        return Err(g.err_at(
            node,
            format!("{op}: expects 3 argument(s), got {}", args.len()),
        ));
    }
    Ok((
        gen_value(g, &args[0])?,
        gen_value(g, &args[1])?,
        gen_value(g, &args[2])?,
    ))
}

/// Value-position `if`: branch chain with a phi join, mirroring the
/// reference's ternary shape (single-expression value branches only;
/// a missing `else` yields void).
fn gen_value_if(g: &mut Gen, node: &Node, elems: &[Node]) -> Result<String, String> {
    let branches = split_branches(elems).map_err(|e| g.err_at(node, e))?;
    for br in &branches {
        if br.body.len() != 1 || !is_value_form(&br.body[0]) {
            return Err(g.err_at(
                node,
                "value-if with statement prefix (only single-expression branches)",
            ));
        }
    }
    let done = g.fresh_lbl("vifend");
    let mut incoming: Vec<(String, String)> = Vec::new();
    let mut last_false = String::new();
    let mut saw_test = false;
    for br in &branches {
        let test = match &br.test {
            Some(t) => t,
            None => {
                // Else arm evaluates in the trailing false block.
                let v = gen_value(g, &br.body[0])?;
                g.emit(&format!("br label %{done}"));
                incoming.push((v, last_false.clone()));
                last_false.clear();
                continue;
            }
        };
        saw_test = true;
        let c = gen_test(g, test)?;
        let then_lbl = g.fresh_lbl("vifthen");
        let false_lbl = g.fresh_lbl("vifnext");
        g.emit(&format!("br i1 {c}, label %{then_lbl}, label %{false_lbl}"));
        g.label(then_lbl.clone());
        let v = gen_value(g, &br.body[0])?;
        g.emit(&format!("br label %{done}"));
        incoming.push((v, then_lbl));
        g.label(false_lbl.clone());
        last_false = false_lbl;
    }
    if !saw_test {
        return Err(g.err_at(node, "if: expects a test expression"));
    }
    if !last_false.is_empty() {
        // No else: void default in the trailing false block.
        let v = g.fresh_tmp();
        g.emit(&format!("{v} = call ptr @cdi_void()"));
        g.emit(&format!("br label %{done}"));
        incoming.push((v, last_false));
    }
    g.label(done.clone());
    let r = g.fresh_tmp();
    let arms = incoming
        .iter()
        .map(|(v, l)| format!("[{v}, %{l}]"))
        .collect::<Vec<_>>()
        .join(", ");
    g.emit(&format!("{r} = phi ptr {arms}"));
    Ok(r)
}

/// Forms that read as values (mirrors the reference gate: statement
/// heads never do, so value-if branches stay single-expression).
fn is_value_form(node: &Node) -> bool {
    match &node.kind {
        NodeKind::Int(_)
        | NodeKind::Float(_)
        | NodeKind::Bool(_)
        | NodeKind::Str(_)
        | NodeKind::Sym(_)
        | NodeKind::Nil => true,
        NodeKind::List(elems, _) => match elems.first() {
            Some(first) => match &first.kind {
                NodeKind::Sym(h) => !matches!(
                    h.as_str(),
                    "var"
                        | "set"
                        | "const"
                        | "while"
                        | "for"
                        | "if"
                        | "break"
                        | "continue"
                        | "return"
                        | "proc"
                        | "elif"
                        | "else"
                        | "new"
                        | "import-java"
                        | "requires-java"
                        | "include-c"
                        | "include-cpp"
                        | "link-native"
                        | "link-c"
                        | "link-cpp"
                        | "extern-c"
                        | "extern-cpp"
                        | "cpp-class"
                        | "cpp-ctor"
                        | "cpp-method"
                        | "cpp-static"
                        | "cpp-field"
                ),
                _ => true,
            },
            None => false,
        },
        _ => false,
    }
}

// ------------------------------------------------------------ native interop

/// A Dirac `proc` name wins over everything (lookup-first); otherwise a
/// name reads as a value when it is a local or a declared global.
fn is_value_bound(g: &Gen, name: &str) -> bool {
    if find_nested(g, name).is_some() {
        return false;
    }
    if g.procs.contains_key(name) {
        return false;
    }
    if g.lookup_local(name).is_some() {
        return true;
    }
    g.globals.contains_key(name)
}

/// Innermost-visible nested procedure (lexical frames, latest def wins).
fn find_nested(g: &Gen, name: &str) -> Option<NestedSig> {
    for f in g.proc_stack.iter().rev() {
        for (n, s) in f.nested.iter().rev() {
            if n == name {
                return Some(s.clone());
            }
        }
    }
    None
}

/// Ancestor path for nest keys (frame names outward-in). Must match the
/// prepass exactly (`@main` for the top-level flow).
fn frame_path(g: &Gen) -> Vec<String> {
    g.proc_stack.iter().map(|f| f.dirac_name.clone()).collect()
}

/// Pretty path for lifted LLVM names (`@main` reads as `main`).
fn ll_path(g: &Gen) -> Vec<String> {
    g.proc_stack
        .iter()
        .map(|f| {
            if f.dirac_name == "@main" {
                String::from("main")
            } else {
                f.dirac_name.clone()
            }
        })
        .collect()
}

/// True when `name` is bound in the current frame region or any ancestor
/// frame region (used by the nested-def filter: only genuinely visible
/// names become captures; anything else takes the natural error path).
fn lookup_in_frames(g: &Gen, name: &str) -> bool {
    let first_base = g.frame_bases.first().copied().unwrap_or(0);
    for sc in &g.scopes[..first_base.min(g.scopes.len())] {
        if sc.vars.contains_key(name) {
            return true;
        }
    }
    for (i, base) in g.frame_bases.iter().enumerate() {
        let end = g.frame_bases.get(i + 1).copied().unwrap_or(g.scopes.len());
        for sc in &g.scopes[(*base).min(g.scopes.len())..end.min(g.scopes.len())] {
            if sc.vars.contains_key(name) {
                return true;
            }
        }
    }
    false
}

/// Load a bound value, enforcing top-level set-before-def like `Sym`.
fn load_bound(g: &mut Gen, node: &Node, name: &str) -> Result<String, String> {
    if let Some(a) = g.lookup_local(name) {
        let v = g.fresh_tmp();
        g.emit(&format!("{v} = load ptr, ptr {a}"));
        return Ok(v);
    }
    if let Some(f) = g.globals.get(name).cloned() {
        if !g.in_proc && !g.defined.contains(name) {
            return Err(g.err_at(node, format!("unbound identifier: {name}")));
        }
        let v = g.fresh_tmp();
        g.emit(&format!("{v} = load ptr, ptr {f}"));
        return Ok(v);
    }
    Err(g.err_at(node, format!("unbound identifier: {name}")))
}

/// Emit a native wrapper call over already-evaluated boxed args.
fn gen_wrap_call(g: &mut Gen, w: &Wrapper, rendered: Vec<String>) -> String {
    let v = g.fresh_tmp();
    let params = rendered
        .iter()
        .map(|r| format!("ptr {r}"))
        .collect::<Vec<_>>()
        .join(", ");
    g.emit(&format!("{v} = call ptr @cdi_wrap_{}({params})", w.idx));
    v
}

/// Emit an n-ary native call (vector builds): boxed args packed into an
/// explicit array, mirroring `gen_nary`'s `(array, count)` shape.
fn gen_native_nary(g: &mut Gen, w: &Wrapper, args: &[Node]) -> Result<String, String> {
    let n = args.len();
    let arr = if n == 0 {
        "null".to_string()
    } else {
        let a = g.fresh_tmp();
        g.emit(&format!("{a} = alloca [{n} x ptr]"));
        for (i, arg) in args.iter().enumerate() {
            let v = gen_value(g, arg)?;
            let slot = g.fresh_tmp();
            g.emit(&format!(
                "{slot} = getelementptr [{n} x ptr], ptr {a}, i32 0, i32 {i}"
            ));
            g.emit(&format!("store ptr {v}, ptr {slot}"));
        }
        a
    };
    let v = g.fresh_tmp();
    g.emit(&format!(
        "{v} = call ptr @cdi_wrap_{}(ptr {arr}, i32 {n})",
        w.idx
    ));
    Ok(v)
}

fn split_one_dot(name: &str) -> Option<(&str, &str)> {
    let mut it = name.splitn(2, '.');
    match (it.next(), it.next()) {
        (Some(a), Some(b)) if !a.is_empty() && !b.is_empty() => Some((a, b)),
        _ => None,
    }
}

// ------------------------------------------------------------ statements
//
// gen_stmt returns true when the statement diverges (terminated the
// block: return/break/continue or a fully-diverging if). Callers stop
// emitting afterwards (dead code is never valid IR to fall into).

fn gen_stmt(g: &mut Gen, node: &Node) -> Result<bool, String> {
    match &node.kind {
        NodeKind::Var { name, val, .. } | NodeKind::Const { name, val, .. } => {
            let is_const = matches!(&node.kind, NodeKind::Const { .. });
            if g.procs.contains_key(name.as_str()) {
                return Err(g.err_at(
                    node,
                    format!("cannot redefine procedure '{name}' as a variable"),
                ));
            }
            if g.proc_stack
                .last()
                .map(|f| f.nested.iter().any(|(n, _)| n == name))
                .unwrap_or(false)
            {
                return Err(g.err_at(
                    node,
                    format!("cannot redefine procedure '{name}' as a variable"),
                ));
            }
            let e = gen_value(g, val)?;
            if !g.in_proc && g.scopes.len() == 1 {
                let f = g
                    .globals
                    .get(name.as_str())
                    .cloned()
                    .ok_or_else(|| g.err_at(node, format!("unbound identifier: {name}")))?;
                g.defined.insert(name.clone());
                if is_const {
                    g.consts_top.insert(name.clone());
                }
                g.emit(&format!("store ptr {e}, ptr {f}"));
            } else {
                let a = g.define_local(name, is_const);
                g.emit(&format!("{a} = alloca ptr"));
                g.emit(&format!("store ptr {e}, ptr {a}"));
            }
            Ok(false)
        }
        NodeKind::Set { name, val } => {
            if name.contains('.') {
                // C++ field write (`set obj.field = v;`) resolves before
                // the JVM-only fallback below.
                if let Some((recv, field)) = split_one_dot(name) {
                    let nat = g.native.clone();
                    if nat.classes.contains_key(recv) {
                        match nat.find_static_field_set(recv, field).cloned() {
                            Some(w) => {
                                let e = gen_value(g, val)?;
                                gen_wrap_call(g, &w, vec![e]);
                                return Ok(false);
                            }
                            None => {
                                if nat.find_static_field(recv, field).is_some() {
                                    return Err(g.err_at(
                                        node,
                                        format!("cannot assign to constant '{recv}.{field}' (declared const)"),
                                    ));
                                }
                                if nat.find_field_set(recv, field).is_some() {
                                    return Err(g.err_at(
                                        node,
                                        format!("'{field}' of class '{recv}' is an instance field (write it on a handle, or use {recv}_{field}_set)"),
                                    ));
                                }
                                return Err(g.err_at(
                                    node,
                                    format!("unknown cpp static '{field}' of class '{recv}' (declare [cpp-static-field {recv} {field} ...])"),
                                ));
                            }
                        }
                    }
                    if is_value_bound(g, recv) {
                        match nat.find_field_by_name(field) {
                            Some((class, _)) => {
                                match nat.find_field_set(&class, field).cloned() {
                                    Some(w) => {
                                        let r = load_bound(g, node, recv)?;
                                        let e = gen_value(g, val)?;
                                        gen_wrap_call(g, &w, vec![r, e]);
                                        return Ok(false);
                                    }
                                    None => {
                                        return Err(g.err_at(
                                            node,
                                            format!("cannot assign to constant '{recv}.{field}' (declared const)"),
                                        ))
                                    }
                                }
                            }
                            None => {
                                return Err(g.err_at(
                                    node,
                                    format!("unknown cpp field '{field}' (no cpp-field declares it)"),
                                ))
                            }
                        }
                    }
                }
                return Err(g.err_at(
                    node,
                    format!("{name} is JVM-only (native has no Java fields)"),
                ));
            }
            if g.procs.contains_key(name.as_str()) || find_nested(g, name.as_str()).is_some() {
                return Err(g.err_at(node, format!("cannot assign to procedure '{name}'")));
            }
            if g.is_const(name) {
                return Err(g.err_at(node, format!("cannot assign to constant '{name}'")));
            }
            if g.native.clone().find_const(name.as_str()).is_some() {
                return Err(g.err_at(node, format!("cannot assign to constant '{name}'")));
            }
            let e = gen_value(g, val)?;
            if let Some(a) = g.lookup_local(name) {
                g.emit(&format!("store ptr {e}, ptr {a}"));
                return Ok(false);
            }
            if g.globals.contains_key(name.as_str()) {
                if !g.in_proc && !g.defined.contains(name.as_str()) {
                    return Err(g.err_at(node, format!("unbound identifier: {name}")));
                }
                let f = g.globals.get(name.as_str()).cloned().unwrap();
                g.emit(&format!("store ptr {e}, ptr {f}"));
                return Ok(false);
            }
            Err(g.err_at(node, format!("unbound identifier: {name}")))
        }
        NodeKind::Proc {
            name, params, body, ..
        } => {
            // Nested procedure: lift to a top-level function with hidden
            // leading capture params (by reference). The def statement
            // emits no code; calls resolve lexically (see gen_call).
            if name == "machine" {
                return Err("machine is a reserved namespace (io/system home)".to_string());
            }
            if g.proc_stack
                .last()
                .map(|f| f.nested.iter().any(|(n, _)| n == name))
                .unwrap_or(false)
            {
                return Err(g.err_at(
                    node,
                    format!("duplicate procedure '{name}' (redefinition not supported)"),
                ));
            }
            if g.lookup_local(name).is_some() {
                return Err(g.err_at(node, format!("cannot redefine '{name}' as a procedure")));
            }
            let path = frame_path(g);
            let key = nest_key(&path, name, node.span.line, node.span.col);
            let mut caps = g.nest_caps.get(&key).cloned().unwrap_or_default();
            // Def-time filter: only genuinely frame-visible names become
            // captures; anything else (typos, error-path names) takes
            // the natural error path inside the body.
            caps.retain(|(n, _)| lookup_in_frames(g, n));
            g.uniq += 1;
            let mut llpath = ll_path(g);
            llpath.push(name.clone());
            let ll = format!("cdi_proc_{}_{}", sanitize(&llpath.join("_")), g.uniq);
            let pnames: Vec<String> = params.iter().map(|(n, _)| n.clone()).collect();
            let sig = NestedSig {
                ll_name: ll.clone(),
                declared_arity: pnames.len(),
                caps: caps.clone(),
            };
            let text = gen_fn_body(
                g,
                &ll,
                name,
                &pnames,
                &caps,
                Some((name.clone(), sig.clone())),
                body,
            )?;
            g.lifted.borrow_mut().push(text);
            // Sequential visibility: later statements in this frame see it.
            if let Some(f) = g.proc_stack.last_mut() {
                f.nested.push((name.clone(), sig));
            }
            g.emit(&format!("; nested procedure '{name}' lifted to @{ll}"));
            Ok(false)
        }
        NodeKind::Int(_)
        | NodeKind::Float(_)
        | NodeKind::Bool(_)
        | NodeKind::Str(_)
        | NodeKind::Sym(_)
        | NodeKind::Nil => {
            if let NodeKind::Sym(name) = &node.kind {
                if (g.procs.contains_key(name.as_str()) || find_nested(g, name.as_str()).is_some())
                    && !g.callback_tramps.contains_key(name.as_str())
                {
                    return Err(g.err_at(node, format!("'{name}' is a procedure, not a value")));
                }
            }
            let v = gen_value(g, node)?;
            let d = g.fresh_tmp();
            g.emit(&format!("{d} = call ptr @cdi_echo(ptr {v})"));
            Ok(false)
        }
        NodeKind::List(elems, tail) => {
            if elems.is_empty() {
                return Err(g.err_at(node, "nil literal [] in statement position"));
            }
            if tail.is_some() {
                return Err(g.err_at(node, format!("cannot call a dotted form: {}", node.print())));
            }
            let h = match &elems[0].kind {
                NodeKind::Sym(s) => s.clone(),
                _ => {
                    let v = gen_value(g, node)?;
                    let d = g.fresh_tmp();
                    g.emit(&format!("{d} = call ptr @cdi_echo(ptr {v})"));
                    return Ok(false);
                }
            };
            match h.as_str() {
                "if" => gen_if_stmt(g, node, elems),
                "while" => gen_while_stmt(g, node, elems),
                "for" => gen_for_stmt(g, node, elems),
                "break" => {
                    if !elems[1..].is_empty() {
                        return Err(g.err_at(node, "break takes no arguments"));
                    }
                    match g.loops.last() {
                        Some(l) => {
                            let t = l.break_lbl.clone();
                            g.emit(&format!("br label %{t}"));
                            Ok(true)
                        }
                        None => Err(g.err_at(node, "break outside a loop")),
                    }
                }
                "continue" => {
                    if !elems[1..].is_empty() {
                        return Err(g.err_at(node, "continue takes no arguments"));
                    }
                    match g.loops.last() {
                        Some(l) => {
                            let t = l.cont_lbl.clone();
                            g.emit(&format!("br label %{t}"));
                            Ok(true)
                        }
                        None => Err(g.err_at(node, "continue outside a loop")),
                    }
                }
                "return" => {
                    if !g.in_proc {
                        return Err(g.err_at(node, "return outside a procedure"));
                    }
                    if elems.len() > 2 {
                        return Err(g.err_at(
                            node,
                            format!("return: expects 0..1 argument(s), got {}", elems.len() - 1),
                        ));
                    }
                    if elems.len() == 2 {
                        let v = gen_value(g, &elems[1])?;
                        g.emit(&format!("ret ptr {v}"));
                    } else {
                        let v = g.fresh_tmp();
                        g.emit(&format!("{v} = call ptr @cdi_void()"));
                        g.emit(&format!("ret ptr {v}"));
                    }
                    Ok(true)
                }
                // Directives and removed forms fail with their guidance.
                "import-java" | "requires-java" => Err(g.err_at(
                    node,
                    format!("{h} is JVM-only (native has no Java classes)"),
                )),
                "new" => Err(g.err_at(node, "new is JVM-only (write Class(...) without new)")),
                h if is_native_directive(h) => {
                    if g.in_proc {
                        Err(g.err_at(node, format!("{h} must be top-level")))
                    } else {
                        Ok(false) // collected up front; emits no code
                    }
                }
                "elif" | "else" => {
                    Err(g.err_at(node, format!("'{h}' is an if-separator, not a form")))
                }
                _ => {
                    let v = gen_call(g, node, &h, &elems[1..])?;
                    // Muting void builtins reads better, but echo skips
                    // void at runtime — one shape for all value calls.
                    let d = g.fresh_tmp();
                    g.emit(&format!("{d} = call ptr @cdi_echo(ptr {v})"));
                    Ok(false)
                }
            }
        }
    }
}

fn gen_seq(g: &mut Gen, body: &[Node]) -> Result<bool, String> {
    let mut diverged = false;
    for b in body {
        if diverged {
            break; // dead tail truncated (javac/LLVM both reject fallthrough)
        }
        diverged = gen_stmt(g, b)?;
    }
    Ok(diverged)
}

fn gen_test(g: &mut Gen, test: &Node) -> Result<String, String> {
    let t = gen_value(g, test)?;
    let c = g.fresh_tmp();
    g.emit(&format!("{c} = call i1 @cdi_truthy(ptr {t})"));
    Ok(c)
}

fn gen_if_stmt(g: &mut Gen, node: &Node, elems: &[Node]) -> Result<bool, String> {
    let branches = split_branches(elems)?;
    let _ = node;
    let has_else = branches.iter().any(|b| b.test.is_none());
    let end = g.fresh_lbl("ifend");
    // Diverging branches already end in a terminator (ret/br), so only
    // fallthrough edges target `end`. A test with no successor test and
    // no else branches false straight to `end` (no empty block — every
    // block needs a terminator). When every path diverges (needs `else`),
    // no edge reaches `end`: skip the label, report divergence so
    // callers truncate dead tails.
    let mut all_diverge = true;
    for (i, br) in branches.iter().enumerate() {
        if let Some(test) = &br.test {
            let c = gen_test(g, test)?;
            let then_lbl = g.fresh_lbl("ifthen");
            let more = i + 1 < branches.len();
            let false_lbl = if more {
                g.fresh_lbl("ifnext")
            } else {
                end.clone()
            };
            g.emit(&format!("br i1 {c}, label %{then_lbl}, label %{false_lbl}"));
            g.label(then_lbl);
            g.push_scope();
            let d = gen_seq(g, &br.body)?;
            g.pop_scope();
            if !d {
                g.emit(&format!("br label %{end}"));
            }
            all_diverge = all_diverge && d;
            if more {
                g.label(false_lbl);
            }
        } else {
            g.push_scope();
            let d = gen_seq(g, &br.body)?;
            g.pop_scope();
            if !d {
                g.emit(&format!("br label %{end}"));
            }
            all_diverge = all_diverge && d;
        }
    }
    if all_diverge && has_else {
        return Ok(true);
    }
    g.label(end);
    // Without else, control always falls through.
    Ok(false)
}

struct Branch {
    test: Option<Node>,
    body: Vec<Node>,
}

fn split_branches(elems: &[Node]) -> Result<Vec<Branch>, String> {
    if elems.len() < 2 {
        return Err("if: expects a test expression".to_string());
    }
    let mut branches = vec![Branch {
        test: Some(elems[1].clone()),
        body: Vec::new(),
    }];
    let mut saw_else = false;
    let mut k = 2;
    while k < elems.len() {
        match &elems[k].kind {
            NodeKind::Sym(s) if s == "elif" || s == "else" => {
                if saw_else {
                    return Err("if: else must be the last branch".to_string());
                }
                if s == "else" {
                    saw_else = true;
                    branches.push(Branch {
                        test: None,
                        body: Vec::new(),
                    });
                } else {
                    k += 1;
                    if k >= elems.len() {
                        return Err("elif needs a test expression".to_string());
                    }
                    branches.push(Branch {
                        test: Some(elems[k].clone()),
                        body: Vec::new(),
                    });
                }
            }
            _ => branches.last_mut().unwrap().body.push(elems[k].clone()),
        }
        k += 1;
    }
    Ok(branches)
}

fn gen_while_stmt(g: &mut Gen, node: &Node, elems: &[Node]) -> Result<bool, String> {
    if elems.len() < 2 {
        return Err(g.err_at(node, "while: expects a test expression"));
    }
    let cond = g.fresh_lbl("wcond");
    let body_lbl = g.fresh_lbl("wbody");
    let exit = g.fresh_lbl("wexit");
    g.emit(&format!("br label %{cond}"));
    g.label(cond.clone());
    let c = gen_test(g, &elems[1])?;
    g.emit(&format!("br i1 {c}, label %{body_lbl}, label %{exit}"));
    g.label(body_lbl);
    g.push_scope();
    g.loops.push(LoopCtx {
        break_lbl: exit.clone(),
        cont_lbl: cond.clone(),
    });
    let d = gen_seq(g, &elems[2..])?;
    g.loops.pop();
    g.pop_scope();
    if !d {
        g.emit(&format!("br label %{cond}"));
    }
    g.label(exit);
    Ok(false)
}

fn gen_for_stmt(g: &mut Gen, node: &Node, elems: &[Node]) -> Result<bool, String> {
    if elems.len() < 4 {
        return Err(g.err_at(node, "for: expects an init, a test, an update and a body"));
    }
    // init runs in a for-scoped child scope (like the reference
    // loop_env): init vars are visible to test/update/body, never outside.
    g.push_scope();
    let d = gen_stmt(g, &elems[1])?;
    if d {
        return Err(g.err_at(node, "for init must not diverge"));
    }
    let cond = g.fresh_lbl("fcond");
    let body_lbl = g.fresh_lbl("fbody");
    let upd = g.fresh_lbl("fupd");
    let exit = g.fresh_lbl("fexit");
    g.emit(&format!("br label %{cond}"));
    g.label(cond.clone());
    let c = gen_test(g, &elems[2])?;
    g.emit(&format!("br i1 {c}, label %{body_lbl}, label %{exit}"));
    g.label(body_lbl);
    g.push_scope();
    g.loops.push(LoopCtx {
        break_lbl: exit.clone(),
        cont_lbl: upd.clone(),
    });
    // Body fallthrough re-enters via the update; diverged edges
    // (break/continue/return) already target exit/update directly.
    let db = gen_seq(g, &elems[4..])?;
    g.loops.pop();
    g.pop_scope();
    if !db {
        g.emit(&format!("br label %{upd}"));
    }
    g.label(upd);
    // Update evaluates for effects (set/expr never diverge in practice).
    let du = gen_stmt(g, &elems[3])?;
    if !du {
        g.emit(&format!("br label %{cond}"));
    }
    g.label(exit);
    g.pop_scope();
    Ok(false)
}

// ------------------------------------------------------------ procedures

/// Top-level procedure table entry: Dirac name, LLVM name, arity, params.
type ProcDef = (String, String, usize, Vec<String>);

fn collect_procs(prog: &[Node]) -> Result<Vec<ProcDef>, String> {
    // Returns (dirac name, llvm name, arity, param names); rejects dupes,
    // nesting (single pass over top level suffices: bodies hold no defs
    // that escape), and `machine` reservation.
    let mut defs = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut uniq = 0usize;
    for node in prog {
        if let NodeKind::Proc { name, params, .. } = &node.kind {
            if name == "machine" {
                return Err("machine is a reserved namespace (io/system home)".to_string());
            }
            if !seen.insert(name.clone()) {
                return Err(format!(
                    "duplicate procedure '{name}' (redefinition not supported)"
                ));
            }
            uniq += 1;
            let ll = format!("cdi_proc_{}_{}", sanitize(name), uniq);
            defs.push((
                name.clone(),
                ll,
                params.len(),
                params.iter().map(|(n, _)| n.clone()).collect(),
            ));
        }
    }
    Ok(defs)
}

fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// C ABI type for a callback signature scalar (`bool` is one byte on
/// our targets, matching C `_Bool` / C++ `bool`).
fn cb_c_ty(t: &NativeTy) -> &'static str {
    match t {
        NativeTy::Int => "i32",
        NativeTy::Double => "double",
        NativeTy::Bool => "i8",
        NativeTy::Str | NativeTy::Ptr => "ptr",
        _ => unreachable!("callback sigs are scalar-only (checked at collect)"),
    }
}

/// Emit one callback trampoline into the lifted sink: box the C args,
/// call the Dirac proc (boxed convention), unbox (or drop) the result.
/// Returns the trampoline symbol (callers box its address).
fn gen_callback_trampoline(g: &mut Gen, alias: &str, info: &CallbackInfo, proc_ll: &str) -> String {
    g.uniq += 1;
    let ll = format!("cdi_cb_{}_{}", sanitize(alias), g.uniq);
    let ret_ty: String = if info.ret == NativeTy::Void {
        "void".to_string()
    } else {
        cb_c_ty(&info.ret).to_string()
    };
    let mut body = String::new();
    let params: Vec<String> = info
        .params
        .iter()
        .enumerate()
        .map(|(i, t)| format!("{} %a{i}", cb_c_ty(t)))
        .collect();
    body.push_str(&format!(
        "define {ret_ty} @{ll}({}) {{\nentry:\n",
        params.join(", ")
    ));
    let mut boxed: Vec<String> = Vec::new();
    for (i, t) in info.params.iter().enumerate() {
        let v = g.fresh_tmp();
        match t {
            NativeTy::Int => body.push_str(&format!("  {v} = call ptr @cdi_mk_int(i32 %a{i})\n")),
            NativeTy::Double => {
                body.push_str(&format!("  {v} = call ptr @cdi_mk_float(double %a{i})\n"))
            }
            NativeTy::Bool => {
                let w = g.fresh_tmp();
                body.push_str(&format!("  {w} = zext i8 %a{i} to i32\n"));
                body.push_str(&format!("  {v} = call ptr @cdi_from_bool(i32 {w})\n"));
            }
            NativeTy::Str => {
                body.push_str(&format!("  {v} = call ptr @cdi_from_cstring(ptr %a{i})\n"))
            }
            NativeTy::Ptr => body.push_str(&format!("  {v} = call ptr @cdi_from_ptr(ptr %a{i})\n")),
            _ => unreachable!("callback sigs are scalar-only (checked at collect)"),
        }
        boxed.push(format!("ptr {v}"));
    }
    let r = g.fresh_tmp();
    body.push_str(&format!(
        "  {r} = call ptr @{proc_ll}({})\n",
        boxed.join(", ")
    ));
    match &info.ret {
        NativeTy::Void => body.push_str("  ret void\n"),
        NativeTy::Int => {
            let v = g.fresh_tmp();
            body.push_str(&format!("  {v} = call i32 @cdi_to_int(ptr {r})\n"));
            body.push_str(&format!("  ret i32 {v}\n"));
        }
        NativeTy::Double => {
            let v = g.fresh_tmp();
            body.push_str(&format!("  {v} = call double @cdi_to_double(ptr {r})\n"));
            body.push_str(&format!("  ret double {v}\n"));
        }
        NativeTy::Bool => {
            let w = g.fresh_tmp();
            let v = g.fresh_tmp();
            body.push_str(&format!("  {w} = call i32 @cdi_to_bool(ptr {r})\n"));
            body.push_str(&format!("  {v} = trunc i32 {w} to i8\n"));
            body.push_str(&format!("  ret i8 {v}\n"));
        }
        NativeTy::Str => {
            let v = g.fresh_tmp();
            body.push_str(&format!("  {v} = call ptr @cdi_to_cstring(ptr {r})\n"));
            body.push_str(&format!("  ret ptr {v}\n"));
        }
        NativeTy::Ptr => {
            let v = g.fresh_tmp();
            body.push_str(&format!("  {v} = call ptr @cdi_to_ptr(ptr {r})\n"));
            body.push_str(&format!("  ret ptr {v}\n"));
        }
        _ => unreachable!("callback sigs are scalar-only (checked at collect)"),
    }
    body.push_str("}\n");
    g.lifted.borrow_mut().push(body);
    ll
}

/// True when any `Proc` node named `name` nests below top level (used
/// to tailor the missing-callback-target diagnostic).
fn has_nested_proc(prog: &[Node], name: &str) -> bool {
    fn walk(nodes: &[Node], name: &str, depth: usize) -> bool {
        for n in nodes {
            match &n.kind {
                NodeKind::Proc { name: pn, body, .. } => {
                    if depth > 0 && pn == name {
                        return true;
                    }
                    if walk(body, name, depth + 1) {
                        return true;
                    }
                }
                NodeKind::List(elems, tail) => {
                    if walk(elems, name, depth) {
                        return true;
                    }
                    if let Some(t) = tail {
                        if walk(std::slice::from_ref(t.as_ref()), name, depth) {
                            return true;
                        }
                    }
                }
                NodeKind::Var { val, .. }
                | NodeKind::Const { val, .. }
                | NodeKind::Set { val, .. }
                    if walk(std::slice::from_ref(val), name, depth) =>
                {
                    return true;
                }
                _ => {}
            }
        }
        false
    }
    walk(prog, name, 0)
}

fn gen_proc_body(
    g: &mut Gen,
    proc_name: &str,
    ll_name: &str,
    param_names: &[String],
    body: &[Node],
) -> Result<(), String> {
    // Top-level procedures take no captures; the frame still exists so
    // nested definitions inside resolve their binder chain.
    let text = gen_fn_body(g, ll_name, proc_name, param_names, &[], None, body)?;
    g.out.push_str(&text);
    Ok(())
}

/// Generate one function body (top-level or lifted-nested) into a fresh
/// buffer and return the complete `define` text. Captures become hidden
/// leading `ptr %capN` params holding caller slot addresses; they bind
/// as ordinary locals (loads/stores work uniformly, writes propagate).
fn gen_fn_body(
    g: &mut Gen,
    ll_name: &str,
    func_name: &str,
    param_names: &[String],
    caps: &[(String, bool)],
    self_nested: Option<(String, NestedSig)>,
    body: &[Node],
) -> Result<String, String> {
    let saved_out = std::mem::take(&mut g.out);
    let r = gen_fn_body_inner(g, ll_name, func_name, param_names, caps, self_nested, body);
    let text = std::mem::replace(&mut g.out, saved_out);
    r.map(|_| text)
}

fn gen_fn_body_inner(
    g: &mut Gen,
    ll_name: &str,
    func_name: &str,
    param_names: &[String],
    caps: &[(String, bool)],
    self_nested: Option<(String, NestedSig)>,
    body: &[Node],
) -> Result<(), String> {
    let mut sig_params = Vec::new();
    for i in 0..caps.len() {
        sig_params.push(format!("ptr %cap{i}"));
    }
    for i in 0..param_names.len() {
        sig_params.push(format!("ptr %p{i}"));
    }
    g.out.push_str(&format!(
        "define ptr @{ll_name}({}) {{\nentry:\n",
        sig_params.join(", ")
    ));
    g.push_scope();
    g.proc_stack.push(ProcFrame {
        dirac_name: func_name.to_string(),
        nested: Vec::new(),
    });
    g.frame_bases.push(g.scopes.len() - 1);
    if let Some((nm, s)) = self_nested {
        if let Some(f) = g.proc_stack.last_mut() {
            f.nested.push((nm, s));
        }
    }
    let saved_in_proc = g.in_proc;
    g.in_proc = true;
    // Nested bodies must not see enclosing loops: `break` inside a
    // nested proc is outside any loop (it returns a function).
    let saved_loops = std::mem::take(&mut g.loops);
    for (i, (n, c)) in caps.iter().enumerate() {
        if let Some(sc) = g.scopes.last_mut() {
            sc.vars.insert(n.clone(), format!("%cap{i}"));
            if *c {
                sc.consts.insert(n.clone());
            }
        }
    }
    for (i, pname) in param_names.iter().enumerate() {
        if pname == "machine" {
            return Err("machine is a reserved namespace (io/system home)".to_string());
        }
        let a = g.define_local(pname, false);
        g.emit(&format!("{a} = alloca ptr"));
        g.emit(&format!("store ptr %p{i}, ptr {a}"));
    }
    let d = gen_seq(g, body)?;
    if !d {
        let v = g.fresh_tmp();
        g.emit(&format!("{v} = call ptr @cdi_void()"));
        g.emit(&format!("ret ptr {v}"));
    }
    g.loops = saved_loops;
    g.in_proc = saved_in_proc;
    g.frame_bases.pop();
    g.proc_stack.pop();
    g.pop_scope();
    g.out.push_str("}\n");
    Ok(())
}

// ------------------------------------------------------------ driver

fn gen_toplevel(g: &mut Gen, node: &Node) -> Result<bool, String> {
    if let NodeKind::Proc { name, .. } = &node.kind {
        if !g.procs.contains_key(name.as_str()) {
            return Err(g.err_at(node, format!("unbound identifier: {name}")));
        }
        return Ok(false); // bodies emitted separately with the full table
    }
    match &node.kind {
        NodeKind::Var { .. } | NodeKind::Set { .. } | NodeKind::Const { .. } => {
            Ok(gen_stmt(g, node)?)
        }
        _ if head_of(node) == "import-java" || head_of(node) == "requires-java" => Err(g.err_at(
            node,
            "requires-java is JVM-only (native has no Java classes)",
        )),
        _ if is_native_directive(head_of(node)) => Ok(false), // collected up front
        _ => {
            let h = head_of(node).to_string();
            if h == "elif" || h == "else" {
                return Err(g.err_at(node, format!("'{h}' is an if-separator, not a form")));
            }
            Ok(gen_stmt(g, node)?)
        }
    }
}

/// Compile parsed top-level forms to LLVM IR text (needs llvmrt/*.c).
pub fn compile_llvm(prog: &[Node]) -> Result<String, String> {
    // Pass 0: native directives (headers, externs, C++ classes). One
    // namespace with procs/vars: collisions are diagnosed here.
    let native = Rc::new(collect_native(prog)?);
    for alias in native
        .by_alias
        .keys()
        .chain(native.classes.keys())
        .chain(native.structs.keys())
        .chain(native.callbacks.keys())
    {
        if is_builtin(alias) {
            return Err(format!(
                "native declaration '{alias}' collides with builtin '{alias}'"
            ));
        }
        if alias == "machine" {
            return Err("machine is a reserved namespace (io/system home)".to_string());
        }
    }
    // Pass 1: procs + top-level var/const fields (one namespace).
    let defs = collect_procs(prog)?;
    let mut g = Gen::new();
    g.native = native.clone();
    for (name, ll, arity, _) in &defs {
        if native.by_alias.contains_key(name)
            || native.classes.contains_key(name)
            || native.structs.contains_key(name)
        {
            return Err(format!("cannot redefine native '{name}' as a procedure"));
        }
        // Callback aliases are exempt here: each one REQUIRES a
        // same-named top-level procedure (resolved below).
        g.procs.insert(
            name.clone(),
            ProcSig {
                ll_name: ll.clone(),
                arity: *arity,
            },
        );
    }
    // Pass 1b: resolve callbacks to top-level procs and emit one LLVM
    // trampoline per decl into the lifted sink (flushed end of module,
    // like nested bodies; forward references are fine).
    {
        let mut aliases: Vec<String> = native.callbacks.keys().cloned().collect();
        aliases.sort();
        for alias in aliases {
            let info: CallbackInfo = native
                .callbacks
                .get(&alias)
                .cloned()
                .expect("callback registered");
            match defs.iter().find(|(n, _, _, _)| n == &alias) {
                Some((_, ll, arity, _)) if *arity == info.params.len() => {
                    let tramp = gen_callback_trampoline(&mut g, &alias, &info, ll);
                    g.callback_tramps.insert(alias, tramp);
                }
                Some((_, _, arity, _)) => {
                    return Err(format!(
                        "callback '{alias}' arity mismatch: signature takes {}, procedure '{alias}' takes {arity}",
                        info.params.len()
                    ));
                }
                None if has_nested_proc(prog, &alias) => {
                    return Err(format!(
                        "callback '{alias}' must name a top-level procedure (nested '{alias}' doesn't qualify)"
                    ));
                }
                None => {
                    return Err(format!(
                        "callback '{alias}' has no matching top-level procedure '{alias}'"
                    ));
                }
            }
        }
    }
    let mut fields: Vec<(String, String)> = Vec::new();
    let mut seen_fields: HashSet<String> = HashSet::new();
    for node in prog {
        let name = match &node.kind {
            NodeKind::Var { name, .. } | NodeKind::Const { name, .. } => name,
            _ => continue,
        };
        if head_of(node) == "import-java" || head_of(node) == "requires-java" {
            continue;
        }
        if g.procs.contains_key(name.as_str()) {
            return Err(format!("cannot redefine procedure '{name}' as a variable"));
        }
        if native.by_alias.contains_key(name.as_str())
            || native.classes.contains_key(name.as_str())
            || native.structs.contains_key(name.as_str())
            || native.callbacks.contains_key(name.as_str())
        {
            return Err(format!("cannot redefine native '{name}' as a variable"));
        }
        if name == "machine" {
            return Err("machine is a reserved namespace (io/system home)".to_string());
        }
        if seen_fields.contains(name) {
            continue;
        }
        seen_fields.insert(name.clone());
        // Sanitized collisions are disambiguated (procs/keywords like
        // `main` can also force a suffix via sanitize collisions below).
        let mut n = 0usize;
        let mut jf = format!("cdi_g_{}", sanitize(name));
        while fields.iter().any(|(_, f)| f == &jf) {
            n += 1;
            jf = format!("cdi_g_{}_{}", sanitize(name), n);
        }
        fields.push((name.clone(), jf));
    }
    for (name, jf) in &fields {
        g.globals.insert(name.clone(), format!("@{jf}"));
    }

    // Pass 1b: static capture analysis for nested procedures
    // (`closures::analyze`, total — codegen reports any real errors).
    {
        let mut items: Vec<TopItem> = Vec::new();
        for node in prog {
            if let NodeKind::Proc {
                name, params, body, ..
            } = &node.kind
            {
                items.push((
                    name.clone(),
                    params.iter().map(|(n, _)| (n.clone(), false)).collect(),
                    body.clone(),
                ));
            }
        }
        let main_body: Vec<Node> = prog
            .iter()
            .filter(|n| !matches!(&n.kind, NodeKind::Proc { .. }))
            .cloned()
            .collect();
        items.push((String::from("@main"), Vec::new(), main_body));
        let top_names: HashSet<String> = defs.iter().map(|(n, _, _, _)| n.clone()).collect();
        let global_names: HashSet<String> = fields.iter().map(|(n, _)| n.clone()).collect();
        g.nest_caps = crate::closures::analyze(&items, &top_names, &global_names, &native).caps;
    }

    // Emit: procs first (full table for recursion), then main.
    let rt_srcs = "llvmrt/value.c llvmrt/arith.c llvmrt/strings.c llvmrt/containers.c llvmrt/cinterop.c llvmrt/pred.c llvmrt/io.c llvmrt/mem.c";
    let mut out = if native.has_native() {
        let link: String = if native.link_flags.is_empty() {
            String::new()
        } else {
            format!(" {}", native.link_flags.join(" "))
        };
        format!(
            "; cdirac-llvm output (native interop): assemble + link with e.g.\n;   clang++ out.ll {rt_srcs} cdi_shim.cpp -o prog{link}\n; (cdi_shim.cpp comes from `cdirac --emit-cpp-shim FILE.cdc`)\n\n",
            rt_srcs = rt_srcs,
            link = link,
        )
    } else {
        format!(
            "; cdirac-llvm output: assemble with clang + llvmrt/*.c\n; clang out.ll {rt_srcs} -o prog\n\n",
            rt_srcs = rt_srcs,
        )
    };
    out.push_str(
        "declare ptr @cdi_mk_int(i32)\n\
         declare ptr @cdi_mk_float(double)\n\
         declare ptr @cdi_mk_bool(i1)\n\
         declare ptr @cdi_mk_str(ptr, i32)\n\
         declare ptr @cdi_nil()\n\
         declare ptr @cdi_void()\n\
         declare ptr @cdi_mk_ptr(ptr)\n\
         declare i1 @cdi_truthy(ptr)\n\
         declare ptr @cdi_add_n(ptr, i32)\n\
         declare ptr @cdi_sub_n(ptr, i32)\n\
         declare ptr @cdi_mul_n(ptr, i32)\n\
         declare ptr @cdi_div_n(ptr, i32)\n\
         declare ptr @cdi_eq_n(ptr, i32)\n\
         declare ptr @cdi_lt_n(ptr, i32)\n\
         declare ptr @cdi_gt_n(ptr, i32)\n\
         declare ptr @cdi_le_n(ptr, i32)\n\
         declare ptr @cdi_ge_n(ptr, i32)\n\
         declare ptr @cdi_not(ptr)\n\
         declare ptr @cdi_display(ptr)\n\
         declare ptr @cdi_displayln(ptr)\n\
         declare ptr @cdi_print(ptr)\n\
         declare ptr @cdi_echo(ptr)\n\
         declare ptr @cdi_array_n(ptr, i32)\n\
         declare ptr @cdi_len(ptr)\n\
         declare ptr @cdi_get(ptr, ptr)\n\
         declare ptr @cdi_quotient(ptr, ptr)\n\
         declare ptr @cdi_remainder(ptr, ptr)\n\
         declare ptr @cdi_modulo(ptr, ptr)\n\
         declare ptr @cdi_add1(ptr)\n\
         declare ptr @cdi_sub1(ptr)\n\
         declare ptr @cdi_abs(ptr)\n\
         declare ptr @cdi_max_n(ptr, i32)\n\
         declare ptr @cdi_min_n(ptr, i32)\n\
         declare ptr @cdi_expt(ptr, ptr)\n\
         declare ptr @cdi_sqrt(ptr)\n\
         declare ptr @cdi_zero_p(ptr)\n\
         declare ptr @cdi_positive_p(ptr)\n\
         declare ptr @cdi_negative_p(ptr)\n\
         declare ptr @cdi_even_p(ptr)\n\
         declare ptr @cdi_odd_p(ptr)\n\
         declare ptr @cdi_equal_p(ptr, ptr)\n\
         declare ptr @cdi_number_p(ptr)\n\
         declare ptr @cdi_real_p(ptr)\n\
         declare ptr @cdi_integer_p(ptr)\n\
         declare ptr @cdi_boolean_p(ptr)\n\
         declare ptr @cdi_string_p(ptr)\n\
         declare ptr @cdi_array_p(ptr)\n\
         declare ptr @cdi_queue_p(ptr)\n\
         declare ptr @cdi_empty_p(ptr)\n\
         declare ptr @cdi_void_p(ptr)\n\
         declare ptr @cdi_put(ptr, ptr, ptr)\n\
         declare ptr @cdi_push(ptr, ptr)\n\
         declare ptr @cdi_pop(ptr)\n\
         declare ptr @cdi_queue_n(ptr, i32)\n\
         declare ptr @cdi_qpush(ptr, ptr)\n\
         declare ptr @cdi_qpush_front(ptr, ptr)\n\
         declare ptr @cdi_qpop(ptr)\n\
         declare ptr @cdi_qpop_back(ptr)\n\
         declare ptr @cdi_qpeek(ptr)\n\
         declare ptr @cdi_qpeek_back(ptr)\n\
         declare ptr @cdi_string_append_n(ptr, i32)\n\
         declare ptr @cdi_string_length(ptr)\n\
         declare ptr @cdi_substring(ptr, ptr, ptr, i1)\n\
         declare ptr @cdi_string_upcase(ptr)\n\
         declare ptr @cdi_string_downcase(ptr)\n\
         declare ptr @cdi_number_to_string(ptr)\n\
         declare ptr @cdi_string_to_number(ptr)\n\
         declare ptr @cdi_newline()\n\
         declare ptr @cdi_read_line()\n\
         declare ptr @cdi_error_n(ptr, i32)\n\
         declare ptr @cdi_machine_alloc(ptr)\n\
         declare ptr @cdi_machine_dealloc(ptr)\n\n",
    );
    // Scalar conversions for callback trampolines only (kept out of
    // the table otherwise so callback-free IR stays byte-identical).
    if !native.callbacks.is_empty() {
        out.push_str(
            "declare i32 @cdi_to_int(ptr)\n\
             declare double @cdi_to_double(ptr)\n\
             declare i32 @cdi_to_bool(ptr)\n\
             declare ptr @cdi_to_cstring(ptr)\n\
             declare ptr @cdi_to_ptr(ptr)\n\
             declare ptr @cdi_from_int(i32)\n\
             declare ptr @cdi_from_double(double)\n\
             declare ptr @cdi_from_bool(i32)\n\
             declare ptr @cdi_from_cstring(ptr)\n\
             declare ptr @cdi_from_ptr(ptr)\n\n",
        );
    }
    out.push_str(&crate::native::wrapper_declares(&native));
    if native.has_native() {
        out.push('\n');
    }
    // Proc bodies with the full table (recursion + mutual recursion).
    for node in prog {
        if let NodeKind::Proc {
            name, params, body, ..
        } = &node.kind
        {
            let sig = g
                .procs
                .get(name.as_str())
                .cloned()
                .ok_or_else(|| format!("unbound identifier: {name}"))?;
            // Fresh generator state per proc, sharing tables and the
            // global temp/label space (every name stays unique).
            let mut pg = Gen::new();
            pg.procs = g.procs.clone();
            pg.globals = g.globals.clone();
            pg.callback_tramps = g.callback_tramps.clone();
            pg.native = g.native.clone();
            pg.lifted = g.lifted.clone();
            pg.nest_caps = g.nest_caps.clone();
            pg.tmp = g.tmp;
            pg.lbl = g.lbl;
            pg.uniq = g.uniq;
            pg.str_consts = g.str_consts;
            let pnames: Vec<String> = params.iter().map(|(n, _)| n.clone()).collect();
            gen_proc_body(&mut pg, name, &sig.ll_name, &pnames, body)?;
            out.push_str(&pg.strtab);
            out.push_str(&pg.out);
            g.tmp = pg.tmp;
            g.lbl = pg.lbl;
            g.uniq = pg.uniq;
            g.str_consts = pg.str_consts;
            // Proc string globals must precede their `define`s: they were
            // just pushed to `out`, ahead of main. Main's own strings
            // flush from `g.strtab` below.
        }
    }
    // Globals + main. Main resolves procs incrementally (a top-level
    // call before its definition fails, like the tree-walker).
    // NOTE: the body generates first so main-level string constants
    // (`g.strtab`) can flush with the other module-level items, ahead
    // of the function (proc bodies flush their own tables).
    g.procs.clear();
    g.push_scope();
    // Main-flow frame: nested definitions directly in top-level blocks
    // lift with `@main` as their binder (main-flow locals capturable).
    g.proc_stack.push(ProcFrame {
        dirac_name: String::from("@main"),
        nested: Vec::new(),
    });
    g.frame_bases.push(g.scopes.len() - 1);
    for node in prog {
        if let NodeKind::Proc { name, .. } = &node.kind {
            if let Some(def) =
                defs.iter()
                    .find(|(n, _, _, _)| n == name)
                    .map(|(_, ll, arity, _)| ProcSig {
                        ll_name: ll.clone(),
                        arity: *arity,
                    })
            {
                g.procs.insert(name.clone(), def);
            }
            continue;
        }
        if gen_toplevel(&mut g, node)? {
            break; // dead tail truncated
        }
    }
    g.out.push_str("  ret i32 0\n}\n");
    for (_, jf) in &fields {
        out.push_str(&format!("@{jf} = global ptr null\n"));
    }
    out.push_str(&g.strtab);
    out.push_str("define i32 @main() {\nentry:\n");
    out.push_str(&g.out);
    // Hoisted nested procedures (forward references are fine in LLVM IR;
    // their string globals flushed above, ahead of every `define`).
    for lifted_body in g.lifted.borrow().iter() {
        out.push_str(lifted_body);
    }
    Ok(out)
}

/// Compile parsed top-level forms to the C++ shim source (compile with
/// `g++ -std=c++17 -c`, then link with the `--emit-llvm` output).
pub fn compile_cpp_shim(prog: &[Node]) -> Result<String, String> {
    let native = collect_native(prog)?;
    if !native.has_native() {
        return Err(
            "no native directives (include-c/include-cpp/extern-c/extern-cpp/cpp-class/...)"
                .to_string(),
        );
    }
    Ok(crate::native::gen_cpp_shim(&native))
}
