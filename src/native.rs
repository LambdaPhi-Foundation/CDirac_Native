//! native: C/C++ header interop for the LLVM backend (C++-first).
//!
//! Thin C support is just headers + free functions; C++ gets classes,
//! construction, methods, statics, and fields on top. Everything lowers
//! through a generated C++ shim (see `gen_cpp_shim`): the shim includes
//! the real headers and exposes stable `extern "C"` wrappers
//! (`cdi_wrap_N`) over boxed `Value*`. The LLVM IR only ever calls those
//! wrappers, so `llvmgen` never deals with C++ name mangling, overload
//! resolution, or templates — the host C++ compiler does.
//!
//! Surface (bracket-form directives, pragma-like, top-level only):
//! ```text
//! [include-c "stdio.h" "string.h"]   C headers
//! [include-cpp "<vector>" "widget.h"] C++ headers (<...> or "..." as written)
//! [link-native "-lm" ...]            link flags ([link-c]/[link-cpp] aliases)
//! [extern-c puts [string] int]       C free function (alias == symbol)
//! [extern-c dirac-alias "real_sym" [int] int]
//! [extern-c printf "printf" [string] int varargs]  C varargs (fixed + extras)
//! [extern-cpp doubles "ns::doubles" [int] int]
//! [callback Cmp [int int] int]                     Dirac proc as C function pointer (see §10)
//! [cpp-class Widget "widget.h"]      opaque class (header optional)
//! [cpp-ctor Widget [int]]            constructor (overloads: one per arity)
//! [cpp-method Widget push [int] void]
//! [cpp-static Widget make [int] Widget]
//! [cpp-field Widget size int]
//! [cpp-enum RED "Color::RED" GREEN "Color::GREEN"]  enumerators as ints
//! [cpp-const PI "M_PI" double]                     named constants
//! [cpp-static-field Widget VERSION int]            static data members
//! [cpp-vector IntVec int]                        typed STL vector (see §7)
//! [cpp-struct Point "ns::Point" "p.h" [x int] [y int]]  by-value struct (see §9)
//! ```
//! Calls stay first-class Dirac: `puts("hi")`, `Widget(5)` constructs,
//! `w.push(5)` / `Widget.make(5)` methods, `w.size` / `set w.size = 3`
//! fields, `Widget_delete(w)` frees, `RED` reads a constant,
//! `Widget.VERSION` reads a static, `IntVec(1, 2)` builds a vector.
//! A Dirac `proc`/`var` literally
//! named like an extern alias or class wins (lookup-first, like the
//! Java model); only on lookup failure does native dispatch run.
//!
//! Raw-type mapping (v1): `int`->`int32_t`/`int`, `double`->`double`,
//! `bool`->`bool`, `string`->`const char*` (C) / `std::string` (C++),
//! `void`->`void`, `ptr`->`void*`, `ClassName`->`ClassName*` (opaque
//! handle, `CDI_PTR`). C++ class results must be pointers; value
//! returns need a tiny header-side factory returning a pointer.

use crate::ast::{Node, NodeKind};

/// Param/return type inside native declarations.
#[derive(Clone, Debug, PartialEq)]
pub enum NativeTy {
    Int,
    Double,
    Bool,
    Str,
    Void,
    Ptr,
    Class(String),
    /// By-value struct (`[cpp-struct ...]`): the Dirac value is an
    /// array of the fields in order; the shim converts field by
    /// field, so C++ padding never matters (aggregate init required).
    Struct(String),
    /// C function pointer by callback alias (`[callback ...]`): the
    /// Dirac value is the trampoline address boxed as a handle. The
    /// alias must name a `[callback]` decl (see `CallbackInfo`).
    Func(String),
}

impl NativeTy {
    pub fn name(&self) -> String {
        match self {
            NativeTy::Int => "int".to_string(),
            NativeTy::Double => "double".to_string(),
            NativeTy::Bool => "bool".to_string(),
            NativeTy::Str => "string".to_string(),
            NativeTy::Void => "void".to_string(),
            NativeTy::Ptr => "ptr".to_string(),
            NativeTy::Class(c) => c.clone(),
            NativeTy::Struct(s) => s.clone(),
            NativeTy::Func(f) => f.clone(),
        }
    }
}

/// One generated wrapper (stable `extern "C"` symbol `cdi_wrap_{idx}`).
#[derive(Clone, Debug)]
pub enum WrapperKind {
    /// C free function: `ret c_sym(params...)`. With `varargs`, the Dirac
    /// side takes `fixed..=fixed+MAX_VARARGS` args over the n-ary
    /// `(Value**, i32)` shape and the shim forwards extras with runtime
    /// tag dispatch (`printf`-style functions).
    ExternC { c_sym: String, varargs: bool },
    /// C++ free function: `ret qualified(params...)` (same varargs story).
    ExternCpp { qualified: String, varargs: bool },
    /// `new Class(params...)`.
    Ctor { class: String },
    /// `self->method(params...)` (instance) or `Class::method(...)` (static).
    Method {
        class: String,
        method: String,
        is_static: bool,
    },
    /// `self->field` read.
    FieldGet { class: String, field: String },
    /// `self->field = v` write.
    FieldSet { class: String, field: String },
    /// Named constant: typed snapshot of a qualified enumerator,
    /// `#define`, `constexpr`, or const variable (`ty` is Int, Double,
    /// Bool, or Str). Reads as a bare Dirac value (`RED`, also `RED()`).
    Const { qualified: String, ty: NativeTy },
    /// `Class::field` read (static data member).
    StaticGet { class: String, field: String },
    /// `Class::field = v` write (static data member).
    StaticSet { class: String, field: String },
    /// `delete self`.
    Delete { class: String },
    /// N-ary vector build: `Alias(v0, v1, ...)` snapshots elements
    /// into a fresh `std::vector<E>`. Any arity; call shape is
    /// `(Value**, i32)` like the Dirac n-ary builtins.
    VecBuild { alias: String },
}

#[derive(Clone, Debug)]
pub struct Wrapper {
    pub idx: usize,
    /// Dirac-visible alias (`puts`, `Widget`, `Widget_push`, ...).
    /// Overloads share one alias; calls dispatch on arity.
    pub alias: String,
    pub kind: WrapperKind,
    pub params: Vec<NativeTy>,
    pub ret: NativeTy,
}

/// Cap on forwarded extras for a varargs wrapper (fixed params + up to
/// this many). The shim generates a flat tag-combination switch
/// (4^k cases per arity k), so this must stay small; larger calls fail
/// with `at most K varargs` (a runtime `Error:`, like other aborts).
pub const MAX_VARARGS: usize = 4;

/// True for `extern-c/cpp … varargs` wrappers (range arity).
pub fn is_varargs(kind: &WrapperKind) -> bool {
    matches!(
        kind,
        WrapperKind::ExternC { varargs: true, .. } | WrapperKind::ExternCpp { varargs: true, .. }
    )
}

/// A `[cpp-struct Alias ...]` bridge: an aggregate C++ struct passed
/// by value, spelled in Dirac as an array of its fields in order.
/// Registered with qualified name + header + ordered `(field, type)`
/// pairs; conversions are field-by-field in the shim.
#[derive(Clone, Debug)]
pub struct StructInfo {
    pub qualified: String,
    pub header: String,
    pub fields: Vec<(String, NativeTy)>,
}

/// A `[callback Alias ...]` decl: a top-level Dirac proc exposed as a
/// C function pointer. The alias doubles as a function-pointer *type*
/// in extern decls; calls lower through an LLVM trampoline (emitted by
/// `llvmgen`, which alone knows the proc's symbol) that boxes C args,
/// calls the proc, and unboxes the result. Signatures stay scalar
/// (`int double bool string ptr`, `void` returns only).
#[derive(Clone, Debug)]
pub struct CallbackInfo {
    pub params: Vec<NativeTy>,
    pub ret: NativeTy,
}

/// A registered C++ class: header + constructor signatures.
#[derive(Clone, Debug, Default)]
pub struct ClassInfo {
    pub header: String,
    /// Fully qualified C++ spelling (`dirac_demo::Accum`); the Dirac
    /// alias stays short (`Accum`). Defaults to the alias.
    pub qualified: String,
    pub ctors: Vec<Vec<NativeTy>>,
}

/// A `[cpp-vector Alias elem]` bridge: typed `std::vector<E>` owned
/// by an opaque handle, desugared at collect time into a class entry
/// (qualified `std::vector<E>`), a 0-arg ctor, an n-ary build wrapper,
/// `len/get/put/push/pop/str` methods, and the usual delete.
#[derive(Clone, Debug)]
pub struct VecInfo {
    pub elem: NativeTy,
}

/// All native declarations collected from top-level directives.
#[derive(Clone, Debug, Default)]
pub struct NativeDecls {
    pub c_headers: Vec<String>,
    pub cpp_headers: Vec<String>,
    pub link_flags: Vec<String>,
    pub wrappers: Vec<Wrapper>,
    /// alias -> wrapper idxs (overloads share an alias; arity picks).
    pub by_alias: std::collections::HashMap<String, Vec<usize>>,
    /// class name -> info (header + constructor signatures).
    pub classes: std::collections::HashMap<String, ClassInfo>,
    /// struct alias -> info (qualified name + header + fields).
    pub structs: std::collections::HashMap<String, StructInfo>,
    /// callback alias -> C signature (see `CallbackInfo`).
    pub callbacks: std::collections::HashMap<String, CallbackInfo>,
    /// vector alias -> element bridge (see VecInfo).
    pub vecs: std::collections::HashMap<String, VecInfo>,
}

impl NativeDecls {
    pub fn has_native(&self) -> bool {
        !self.c_headers.is_empty()
            || !self.cpp_headers.is_empty()
            || !self.link_flags.is_empty()
            || !self.wrappers.is_empty()
            || !self.classes.is_empty()
            || !self.structs.is_empty()
            || !self.callbacks.is_empty()
            || !self.vecs.is_empty()
    }

    pub fn find_alias(&self, alias: &str) -> Option<&Wrapper> {
        self.by_alias
            .get(alias)
            .and_then(|idxs| idxs.first().map(|&i| &self.wrappers[i]))
    }

    /// Overload candidates for a Dirac alias.
    pub fn alias_overloads(&self, alias: &str) -> Vec<&Wrapper> {
        match self.by_alias.get(alias) {
            Some(idxs) => idxs.iter().map(|&i| &self.wrappers[i]).collect(),
            None => Vec::new(),
        }
    }

    /// Single overload of `alias` with Dirac arity `arity`.
    /// VecBuild wrappers never match here (any arity); use
    /// `find_vec_build` as the fallback after fixed overloads.
    /// Varargs wrappers never match here either (range arity); use
    /// `find_varargs` after fixed overloads.
    pub fn find_alias_arity(&self, alias: &str, arity: usize) -> Option<&Wrapper> {
        self.alias_overloads(alias).into_iter().find(|w| {
            !matches!(&w.kind, WrapperKind::VecBuild { .. })
                && !is_varargs(&w.kind)
                && wrapper_arity(w) == arity
        })
    }

    /// Varargs overload of `alias` accepting Dirac arity `arity`
    /// (`arity >= fixed params`). Checked after fixed overloads.
    pub fn find_varargs(&self, alias: &str, arity: usize) -> Option<&Wrapper> {
        self.alias_overloads(alias).into_iter().find(|w| {
            is_varargs(&w.kind) && arity >= w.params.len() && arity <= w.params.len() + MAX_VARARGS
        })
    }

    /// N-ary vector build for `alias`, if `alias` bridges a vector.
    pub fn find_vec_build(&self, alias: &str) -> Option<&Wrapper> {
        self.wrappers.iter().find(|w| match &w.kind {
            WrapperKind::VecBuild { alias: a } => a == alias,
            _ => false,
        })
    }

    /// Instance-method candidates for dot sugar (`w.push(...)`):
    /// every class registering `method` with matching Dirac arity
    /// (params plus the implicit handle).
    pub fn method_candidates(&self, method: &str, arity_with_handle: usize) -> Vec<&Wrapper> {
        self.wrappers
            .iter()
            .filter(|w| match &w.kind {
                WrapperKind::Method {
                    method: m,
                    is_static: false,
                    ..
                } => m == method && wrapper_arity(w) == arity_with_handle,
                _ => false,
            })
            .collect()
    }

    pub fn find_static(&self, class: &str, method: &str, arity: usize) -> Option<&Wrapper> {
        self.wrappers.iter().find(|w| match &w.kind {
            WrapperKind::Method {
                class: c,
                method: m,
                is_static: true,
            } => c == class && m == method && wrapper_arity(w) == arity,
            _ => false,
        })
    }

    /// Instance method of `class` taking `n_params` Dirac params (handle
    /// implicit). Backs qualified calls `Class.method(handle, ...)` with
    /// an explicit handle — always unambiguous.
    pub fn find_instance(&self, class: &str, method: &str, n_params: usize) -> Option<&Wrapper> {
        self.wrappers.iter().find(|w| match &w.kind {
            WrapperKind::Method {
                class: c,
                method: m,
                is_static: false,
            } => c == class && m == method && w.params.len() == n_params,
            _ => false,
        })
    }

    /// Constructor overload of `class` with Dirac arity `arity`.
    pub fn find_ctor(&self, class: &str, arity: usize) -> Option<&Wrapper> {
        self.wrappers.iter().find(|w| match &w.kind {
            WrapperKind::Ctor { class: c } => c == class && wrapper_arity(w) == arity,
            _ => false,
        })
    }

    pub fn find_field(&self, class: &str, field: &str) -> Option<&Wrapper> {
        self.wrappers.iter().find(|w| match &w.kind {
            WrapperKind::FieldGet { class: c, field: f } => c == class && f == field,
            _ => false,
        })
    }

    pub fn find_field_set(&self, class: &str, field: &str) -> Option<&Wrapper> {
        self.wrappers.iter().find(|w| match &w.kind {
            WrapperKind::FieldSet { class: c, field: f } => c == class && f == field,
            _ => false,
        })
    }

    /// Static data member getter for `class.field`.
    pub fn find_static_field(&self, class: &str, field: &str) -> Option<&Wrapper> {
        self.wrappers.iter().find(|w| match &w.kind {
            WrapperKind::StaticGet { class: c, field: f } => c == class && f == field,
            _ => false,
        })
    }

    /// Static data member setter for `set class.field = v`.
    pub fn find_static_field_set(&self, class: &str, field: &str) -> Option<&Wrapper> {
        self.wrappers.iter().find(|w| match &w.kind {
            WrapperKind::StaticSet { class: c, field: f } => c == class && f == field,
            _ => false,
        })
    }

    /// Named constant by bare Dirac alias (`RED` reads as a value).
    /// Lookup-first still applies: callers check procs/vars first.
    pub fn find_const(&self, alias: &str) -> Option<&Wrapper> {
        self.by_alias.get(alias).and_then(|idxs| {
            idxs.iter().find_map(|&i| match &self.wrappers[i].kind {
                WrapperKind::Const { .. } => Some(&self.wrappers[i]),
                _ => None,
            })
        })
    }

    /// Field registration for `obj.field` sugar: exactly one class may
    /// register `field` in v1 (else use explicit wrappers).
    pub fn find_field_by_name(&self, field: &str) -> Option<(String, &Wrapper)> {
        let mut hit: Option<(String, &Wrapper)> = None;
        for w in &self.wrappers {
            if let WrapperKind::FieldGet { class, field: f } = &w.kind {
                if f == field {
                    if hit.is_some() {
                        return None;
                    }
                    hit = Some((class.clone(), w));
                }
            }
        }
        hit
    }
}

pub fn is_native_directive(head: &str) -> bool {
    matches!(
        head,
        "include-c"
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
            | "cpp-enum"
            | "cpp-const"
            | "cpp-static-field"
            | "cpp-struct"
            | "cpp-vector"
            | "callback"
    )
}

fn err_at(node: &Node, msg: impl Into<String>) -> String {
    format!("line {}: {}", node.span.line, msg.into())
}

fn expect_sym(node: &Node, what: &str) -> Result<String, String> {
    match &node.kind {
        NodeKind::Sym(s) => Ok(s.clone()),
        _ => Err(err_at(
            node,
            format!("expected {what} (found '{}')", node.print()),
        )),
    }
}

fn expect_str(node: &Node, what: &str) -> Result<String, String> {
    match &node.kind {
        NodeKind::Str(s) => Ok(s.clone()),
        _ => Err(err_at(
            node,
            format!("expected {what} as a string (found '{}')", node.print()),
        )),
    }
}

fn sym_or_str(node: &Node, what: &str) -> Result<String, String> {
    match &node.kind {
        NodeKind::Sym(s) | NodeKind::Str(s) => Ok(s.clone()),
        _ => Err(err_at(
            node,
            format!("expected {what} (found '{}')", node.print()),
        )),
    }
}

/// Parse a native type item: `int double bool string void ptr`, a
/// registered class or struct name, or a registered callback alias
/// (as a function-pointer type). Classes/structs/callbacks must be
/// declared before use (order-dependent, like classes).
fn parse_ty(
    node: &Node,
    classes: &std::collections::HashMap<String, ClassInfo>,
    structs: &std::collections::HashMap<String, StructInfo>,
    callbacks: &std::collections::HashMap<String, CallbackInfo>,
) -> Result<NativeTy, String> {
    let s = expect_sym(
        node,
        "a type (int, double, bool, string, void, ptr, a cpp-class, a cpp-struct, or a callback)",
    )?;
    match s.as_str() {
        "int" => Ok(NativeTy::Int),
        "double" => Ok(NativeTy::Double),
        "bool" => Ok(NativeTy::Bool),
        "string" => Ok(NativeTy::Str),
        "void" => Ok(NativeTy::Void),
        "ptr" => Ok(NativeTy::Ptr),
        c if classes.contains_key(c) => Ok(NativeTy::Class(c.to_string())),
        c if structs.contains_key(c) => Ok(NativeTy::Struct(c.to_string())),
        c if callbacks.contains_key(c) => Ok(NativeTy::Func(c.to_string())),
        other => Err(err_at(node, format!("unknown native type: {other}"))),
    }
}

fn parse_params(
    node: &Node,
    classes: &std::collections::HashMap<String, ClassInfo>,
    structs: &std::collections::HashMap<String, StructInfo>,
    callbacks: &std::collections::HashMap<String, CallbackInfo>,
) -> Result<Vec<NativeTy>, String> {
    // `[]` (Nil) is the empty parameter list.
    if matches!(&node.kind, NodeKind::Nil) {
        return Ok(Vec::new());
    }
    match &node.kind {
        NodeKind::List(elems, tail) => {
            if tail.is_some() {
                return Err(err_at(node, "parameter list takes no dotted tail"));
            }
            let mut out = Vec::new();
            for e in elems {
                out.push(parse_ty(e, classes, structs, callbacks)?);
            }
            Ok(out)
        }
        _ => Err(err_at(node, "expected a parameter list [...]")),
    }
}

fn list_elems(node: &Node) -> &[Node] {
    match &node.kind {
        NodeKind::List(elems, _) => elems,
        _ => &[],
    }
}

/// Collect top-level native directives. Must run before proc/field
/// collection so collisions are diagnosed early.
pub fn collect_native(prog: &[Node]) -> Result<NativeDecls, String> {
    let mut d = NativeDecls::default();
    // Overloads share one alias (C++ overloads, ctors); identical
    // (alias, arity) twice is a duplicate. C free functions must be
    // unique per alias (C has no overloading).
    let push_wrapper = |d: &mut NativeDecls,
                        node: &Node,
                        alias: String,
                        kind: WrapperKind,
                        params: Vec<NativeTy>,
                        ret: NativeTy|
     -> Result<(), String> {
        let arity = wrapper_call_arity(&kind, params.len());
        let is_c = matches!(&kind, WrapperKind::ExternC { .. });
        if d.callbacks.contains_key(&alias) {
            return Err(err_at(
                node,
                format!("native alias '{alias}' collides with callback '{alias}'"),
            ));
        }
        if d.structs.contains_key(&alias) {
            return Err(err_at(
                node,
                format!("native alias '{alias}' collides with cpp-struct '{alias}'"),
            ));
        }
        if let Some(idxs) = d.by_alias.get(&alias) {
            for &i in idxs {
                let w = &d.wrappers[i];
                if wrapper_call_arity(&w.kind, w.params.len()) == arity {
                    return Err(err_at(
                        node,
                        format!("duplicate native declaration '{alias}'"),
                    ));
                }
                // A varargs decl covers a range, so any same-alias decl
                // beside it is a duplicate (C keeps no-overload rule;
                // C++ keeps one varargs per alias).
                if is_varargs(&kind) || is_varargs(&w.kind) {
                    return Err(err_at(
                        node,
                        format!(
                            "duplicate native declaration '{alias}' (varargs covers all arities)"
                        ),
                    ));
                }
            }
            if is_c {
                return Err(err_at(
                    node,
                    format!("duplicate native declaration '{alias}' (C has no overloads)"),
                ));
            }
        }
        let idx = d.wrappers.len();
        d.wrappers.push(Wrapper {
            idx,
            alias: alias.clone(),
            kind,
            params,
            ret,
        });
        d.by_alias.entry(alias).or_default().push(idx);
        Ok(())
    };

    for node in prog {
        let elems = list_elems(node);
        if elems.is_empty() {
            continue;
        }
        let head = match &elems[0].kind {
            NodeKind::Sym(s) => s.clone(),
            _ => continue,
        };
        if !is_native_directive(&head) {
            continue;
        }
        let args = &elems[1..];
        match head.as_str() {
            "include-c" => {
                for a in args {
                    d.c_headers.push(expect_str(a, "a header")?);
                }
            }
            "include-cpp" => {
                for a in args {
                    d.cpp_headers.push(expect_str(a, "a header")?);
                }
            }
            "link-native" | "link-c" | "link-cpp" => {
                for a in args {
                    d.link_flags.push(expect_str(a, "a link flag")?);
                }
            }
            "extern-c" | "extern-cpp" => {
                // [extern-c alias [params] ret] or
                // [extern-c alias "target" [params] ret]
                // plus trailing `varargs` for printf-style functions:
                // [extern-c printf "printf" [string] int varargs]
                // (fixed params, then up to MAX_VARARGS extras).
                let raw: &[Node] = args;
                let varargs = raw
                    .last()
                    .map(|n| matches!(&n.kind, NodeKind::Sym(s) if s == "varargs"))
                    .unwrap_or(false);
                let owned: Vec<Node>;
                let args: &[Node] = if varargs {
                    owned = raw[..raw.len() - 1].to_vec();
                    &owned
                } else {
                    raw
                };
                if args.len() != 3 && args.len() != 4 {
                    return Err(err_at(node, format!("{head}: expects alias [params] ret (or alias target [params] ret), optional trailing varargs")));
                }
                let alias = expect_sym(&args[0], "an alias")?;
                if d.classes.contains_key(&alias) {
                    return Err(err_at(
                        &args[0],
                        format!("native alias '{alias}' collides with cpp-class '{alias}'"),
                    ));
                }
                let (target, params_node, ret_node) = if args.len() == 4 {
                    (sym_or_str(&args[1], "a target symbol")?, &args[2], &args[3])
                } else {
                    (alias.clone(), &args[1], &args[2])
                };
                let params = parse_params(params_node, &d.classes, &d.structs, &d.callbacks)?;
                let ret = parse_ty(ret_node, &d.classes, &d.structs, &d.callbacks)?;
                if ret == NativeTy::Void && params.contains(&NativeTy::Void) {
                    return Err(err_at(
                        node,
                        format!("{head}: void is only valid as the return type"),
                    ));
                }
                if params.contains(&NativeTy::Void) {
                    return Err(err_at(
                        params_node,
                        format!("{head}: void is only valid as the return type"),
                    ));
                }
                let kind = if head == "extern-c" {
                    WrapperKind::ExternC {
                        c_sym: target,
                        varargs,
                    }
                } else {
                    WrapperKind::ExternCpp {
                        qualified: target,
                        varargs,
                    }
                };
                push_wrapper(&mut d, node, alias, kind, params, ret)?;
            }
            "cpp-class" => {
                // [cpp-class Alias] or [cpp-class Alias header] or
                // [cpp-class Alias "Qualified::Name" header]
                if args.is_empty() || args.len() > 3 {
                    return Err(err_at(
                        node,
                        "cpp-class: expects Alias (or Alias qualified header)",
                    ));
                }
                let name = expect_sym(&args[0], "a class name")?;
                if d.classes.contains_key(&name) {
                    return Err(err_at(
                        node,
                        format!("duplicate native declaration '{name}'"),
                    ));
                }
                if d.by_alias.contains_key(&name) {
                    return Err(err_at(
                        &args[0],
                        format!("cpp-class '{name}' collides with native alias '{name}'"),
                    ));
                }
                if d.structs.contains_key(&name) {
                    return Err(err_at(
                        &args[0],
                        format!("cpp-class '{name}' collides with cpp-struct '{name}'"),
                    ));
                }
                if d.callbacks.contains_key(&name) {
                    return Err(err_at(
                        &args[0],
                        format!("cpp-class '{name}' collides with callback '{name}'"),
                    ));
                }
                let mut qualified = name.clone();
                let mut header = String::new();
                if args.len() == 2 {
                    // Ambiguous slot: a Str holding `::` is the qualified
                    // name, any other Str is a header; a Sym is the
                    // qualified name (simple rename, no `::` possible).
                    match &args[1].kind {
                        NodeKind::Str(h) if h.contains("::") => qualified = h.clone(),
                        NodeKind::Str(h) => header = h.clone(),
                        NodeKind::Sym(q) => qualified = q.clone(),
                        _ => {
                            return Err(err_at(
                                &args[1],
                                "cpp-class: expects a header string or qualified name",
                            ))
                        }
                    }
                } else if args.len() == 3 {
                    qualified = sym_or_str(&args[1], "a qualified C++ name")?;
                    header = expect_str(&args[2], "a header")?;
                }
                if !header.is_empty() && !d.cpp_headers.contains(&header) {
                    d.cpp_headers.push(header.clone());
                }
                d.classes.insert(
                    name.clone(),
                    ClassInfo {
                        header,
                        qualified,
                        ctors: Vec::new(),
                    },
                );
                // Per-class delete wrapper: `Class_delete(handle)`.
                // Params stay empty: the handle is the implicit a0.
                let alias = format!("{name}_delete");
                push_wrapper(
                    &mut d,
                    node,
                    alias,
                    WrapperKind::Delete { class: name },
                    Vec::new(),
                    NativeTy::Void,
                )?;
            }
            "cpp-struct" => {
                // [cpp-struct Alias Qualified Header [field type] ...]:
                // an aggregate C++ struct passed by value. Dirac spells
                // values as arrays of the fields in order; the shim
                // converts field by field (aggregate init in, array
                // build out), so C++ padding never matters. Structs must
                // be declared before use (like classes), which also
                // rules out by-value cycles.
                if args.len() < 4 {
                    return Err(err_at(
                        node,
                        "cpp-struct: expects Alias Qualified Header [field type] ...",
                    ));
                }
                let name = expect_sym(&args[0], "a struct name")?;
                if d.structs.contains_key(&name) {
                    return Err(err_at(
                        node,
                        format!("duplicate native declaration '{name}'"),
                    ));
                }
                if d.classes.contains_key(&name) {
                    return Err(err_at(
                        &args[0],
                        format!("cpp-struct '{name}' collides with cpp-class '{name}'"),
                    ));
                }
                if d.by_alias.contains_key(&name) {
                    return Err(err_at(
                        &args[0],
                        format!("cpp-struct '{name}' collides with native alias '{name}'"),
                    ));
                }
                if d.vecs.contains_key(&name) {
                    return Err(err_at(
                        &args[0],
                        format!("cpp-struct '{name}' collides with cpp-vector '{name}'"),
                    ));
                }
                let qualified = sym_or_str(&args[1], "a qualified C++ name")?;
                let header = expect_str(&args[2], "a header")?;
                let mut fields = Vec::new();
                let mut seen_fields = std::collections::HashSet::new();
                for f in &args[3..] {
                    let elems = list_elems(f);
                    if elems.len() != 2 {
                        return Err(err_at(f, "cpp-struct: expects [field type] pairs"));
                    }
                    let fname = expect_sym(&elems[0], "a field name")?;
                    if !seen_fields.insert(fname.clone()) {
                        return Err(err_at(
                            &elems[0],
                            format!("cpp-struct '{name}' has duplicate field '{fname}'"),
                        ));
                    }
                    let fty = parse_ty(&elems[1], &d.classes, &d.structs, &d.callbacks)?;
                    if fty == NativeTy::Void {
                        return Err(err_at(&elems[1], "cpp-struct: void is not a field type"));
                    }
                    fields.push((fname, fty));
                }
                if !d.cpp_headers.contains(&header) {
                    d.cpp_headers.push(header.clone());
                }
                d.structs.insert(
                    name,
                    StructInfo {
                        qualified,
                        header,
                        fields,
                    },
                );
            }
            "callback" => {
                // [callback Alias [params] ret]: expose top-level Dirac
                // proc `Alias` as a C function pointer. The alias doubles
                // as a function-pointer type in extern decls; calls lower
                // through an LLVM trampoline (see `llvmgen`), so the shim
                // never names it. Signatures stay scalar (`int double
                // bool string ptr`, `void` returns only); anything else
                // is a follow-up (like struct/class returns).
                if args.len() != 3 {
                    return Err(err_at(node, "callback: expects Alias [params] ret"));
                }
                let name = expect_sym(&args[0], "a callback name")?;
                if d.callbacks.contains_key(&name) {
                    return Err(err_at(
                        node,
                        format!("duplicate native declaration '{name}'"),
                    ));
                }
                if d.classes.contains_key(&name) {
                    return Err(err_at(
                        &args[0],
                        format!("callback '{name}' collides with cpp-class '{name}'"),
                    ));
                }
                if d.structs.contains_key(&name) {
                    return Err(err_at(
                        &args[0],
                        format!("callback '{name}' collides with cpp-struct '{name}'"),
                    ));
                }
                if d.by_alias.contains_key(&name) {
                    return Err(err_at(
                        &args[0],
                        format!("callback '{name}' collides with native alias '{name}'"),
                    ));
                }
                if d.vecs.contains_key(&name) {
                    return Err(err_at(
                        &args[0],
                        format!("callback '{name}' collides with cpp-vector '{name}'"),
                    ));
                }
                let params = parse_params(&args[1], &d.classes, &d.structs, &d.callbacks)?;
                let ret = parse_ty(&args[2], &d.classes, &d.structs, &d.callbacks)?;
                for t in params.iter().chain(std::iter::once(&ret)) {
                    match t {
                        NativeTy::Int
                        | NativeTy::Double
                        | NativeTy::Bool
                        | NativeTy::Str
                        | NativeTy::Ptr
                        | NativeTy::Void => {}
                        other => {
                            return Err(err_at(
                                node,
                                format!(
                                    "callback '{name}': signatures support int, double, bool, string, ptr (and void returns), found {}",
                                    other.name()
                                ),
                            ))
                        }
                    }
                }
                if params.contains(&NativeTy::Void) {
                    return Err(err_at(node, "callback: void is not a parameter type"));
                }
                d.callbacks.insert(name, CallbackInfo { params, ret });
            }
            "cpp-ctor" => {
                // [cpp-ctor Class [params]] (overloads: one per arity).
                if args.len() != 2 {
                    return Err(err_at(node, "cpp-ctor: expects Class [params]"));
                }
                let class = expect_sym(&args[0], "a class name")?;
                if !d.classes.contains_key(&class) {
                    return Err(err_at(
                        node,
                        format!("cpp-ctor: unknown cpp-class '{class}'"),
                    ));
                }
                let params = parse_params(&args[1], &d.classes, &d.structs, &d.callbacks)?;
                if params.contains(&NativeTy::Void) {
                    return Err(err_at(&args[1], "cpp-ctor: void is not a parameter type"));
                }
                if let Some(info) = d.classes.get_mut(&class) {
                    info.ctors.push(params.clone());
                }
                let ret = NativeTy::Class(class.clone());
                push_wrapper(
                    &mut d,
                    node,
                    class.clone(),
                    WrapperKind::Ctor { class },
                    params,
                    ret,
                )?;
            }
            "cpp-method" | "cpp-static" => {
                if args.len() != 4 {
                    return Err(err_at(
                        node,
                        format!("{head}: expects Class method [params] ret"),
                    ));
                }
                let class = expect_sym(&args[0], "a class name")?;
                let method = expect_sym(&args[1], "a method name")?;
                if !d.classes.contains_key(&class) {
                    return Err(err_at(node, format!("{head}: unknown cpp-class '{class}'")));
                }
                let params = parse_params(&args[2], &d.classes, &d.structs, &d.callbacks)?;
                let ret = parse_ty(&args[3], &d.classes, &d.structs, &d.callbacks)?;
                if params.contains(&NativeTy::Void) {
                    return Err(err_at(
                        &args[2],
                        format!("{head}: void is only valid as the return type"),
                    ));
                }
                let is_static = head == "cpp-static";
                let alias = format!("{class}_{method}");
                push_wrapper(
                    &mut d,
                    node,
                    alias,
                    WrapperKind::Method {
                        class,
                        method,
                        is_static,
                    },
                    params,
                    ret,
                )?;
            }
            "cpp-field" => {
                // [cpp-field Class field type] (+ optional `const`).
                if args.len() != 3 && args.len() != 4 {
                    return Err(err_at(
                        node,
                        "cpp-field: expects Class field type (or Class field type const)",
                    ));
                }
                let class = expect_sym(&args[0], "a class name")?;
                let field = expect_sym(&args[1], "a field name")?;
                if !d.classes.contains_key(&class) {
                    return Err(err_at(
                        node,
                        format!("cpp-field: unknown cpp-class '{class}'"),
                    ));
                }
                let ty = parse_ty(&args[2], &d.classes, &d.structs, &d.callbacks)?;
                if ty == NativeTy::Void {
                    return Err(err_at(&args[2], "cpp-field: void is not a field type"));
                }
                // Trailing `const` marks read-only members (LIMIT-style):
                // no setter is generated and Dirac writes fail naming it.
                let readonly = if args.len() == 4 {
                    match &args[3].kind {
                        NodeKind::Sym(s) if s == "const" => true,
                        _ => {
                            return Err(err_at(
                                &args[3],
                                "cpp-field: fourth item must be const (or nothing)",
                            ))
                        }
                    }
                } else {
                    false
                };
                // One getter + (unless const) one setter wrapper; sugar
                // resolves by name.
                push_wrapper(
                    &mut d,
                    node,
                    format!("{class}_{field}_get"),
                    WrapperKind::FieldGet {
                        class: class.clone(),
                        field: field.clone(),
                    },
                    vec![],
                    ty.clone(),
                )?;
                if !readonly {
                    push_wrapper(
                        &mut d,
                        node,
                        format!("{class}_{field}_set"),
                        WrapperKind::FieldSet { class, field },
                        vec![ty],
                        NativeTy::Void,
                    )?;
                }
            }
            "cpp-enum" => {
                // [cpp-enum Alias "Qualified::Value" ...]: enumerators
                // (plain or scoped) snapshot as Dirac ints. Pairs keep
                // one directive per enum natural.
                if args.is_empty() || !args.len().is_multiple_of(2) {
                    return Err(err_at(
                        node,
                        "cpp-enum: expects Alias \"Qualified::Value\" pairs",
                    ));
                }
                let mut i = 0;
                while i < args.len() {
                    let alias = expect_sym(&args[i], "an alias")?;
                    if d.classes.contains_key(&alias) {
                        return Err(err_at(
                            &args[i],
                            format!("native alias '{alias}' collides with cpp-class '{alias}'"),
                        ));
                    }
                    let qualified = sym_or_str(&args[i + 1], "a qualified enumerator")?;
                    push_wrapper(
                        &mut d,
                        &args[i],
                        alias,
                        WrapperKind::Const {
                            qualified,
                            ty: NativeTy::Int,
                        },
                        Vec::new(),
                        NativeTy::Int,
                    )?;
                    i += 2;
                }
            }
            "cpp-const" => {
                // [cpp-const Alias "Qualified::Name" type]: `#define`,
                // `constexpr`, or const variable snapshot. Type is int,
                // double, bool, or string (never void/ptr/class: consts
                // are plain values).
                if args.len() != 3 {
                    return Err(err_at(
                        node,
                        "cpp-const: expects Alias \"Qualified::Name\" type",
                    ));
                }
                let alias = expect_sym(&args[0], "an alias")?;
                if d.classes.contains_key(&alias) {
                    return Err(err_at(
                        &args[0],
                        format!("native alias '{alias}' collides with cpp-class '{alias}'"),
                    ));
                }
                let qualified = sym_or_str(&args[1], "a qualified constant")?;
                let ty = match expect_sym(&args[2], "a type (int, double, bool, string)")?.as_str() {
                    "int" => NativeTy::Int,
                    "double" => NativeTy::Double,
                    "bool" => NativeTy::Bool,
                    "string" => NativeTy::Str,
                    other => {
                        return Err(err_at(
                            &args[2],
                            format!("cpp-const: type must be int, double, bool, or string (found '{other}')"),
                        ))
                    }
                };
                push_wrapper(
                    &mut d,
                    node,
                    alias,
                    WrapperKind::Const {
                        qualified,
                        ty: ty.clone(),
                    },
                    Vec::new(),
                    ty,
                )?;
            }
            "cpp-static-field" => {
                // [cpp-static-field Class field type] (+ optional `const`).
                // Static data member with `Class.field` read sugar,
                // `set Class.field = v` writes, and explicit
                // `Class_field_sget/sset` aliases.
                if args.len() != 3 && args.len() != 4 {
                    return Err(err_at(
                        node,
                        "cpp-static-field: expects Class field type (or Class field type const)",
                    ));
                }
                let class = expect_sym(&args[0], "a class name")?;
                let field = expect_sym(&args[1], "a field name")?;
                if !d.classes.contains_key(&class) {
                    return Err(err_at(
                        node,
                        format!("cpp-static-field: unknown cpp-class '{class}'"),
                    ));
                }
                let ty = parse_ty(&args[2], &d.classes, &d.structs, &d.callbacks)?;
                if ty == NativeTy::Void {
                    return Err(err_at(
                        &args[2],
                        "cpp-static-field: void is not a field type",
                    ));
                }
                // Trailing `const` marks read-only members: no setter is
                // generated and Dirac writes fail naming it.
                let readonly = if args.len() == 4 {
                    match &args[3].kind {
                        NodeKind::Sym(s) if s == "const" => true,
                        _ => {
                            return Err(err_at(
                                &args[3],
                                "cpp-static-field: fourth item must be const (or nothing)",
                            ))
                        }
                    }
                } else {
                    false
                };
                push_wrapper(
                    &mut d,
                    node,
                    format!("{class}_{field}_sget"),
                    WrapperKind::StaticGet {
                        class: class.clone(),
                        field: field.clone(),
                    },
                    vec![],
                    ty.clone(),
                )?;
                if !readonly {
                    push_wrapper(
                        &mut d,
                        node,
                        format!("{class}_{field}_sset"),
                        WrapperKind::StaticSet { class, field },
                        vec![ty],
                        NativeTy::Void,
                    )?;
                }
            }
            "cpp-vector" => {
                // [cpp-vector Alias elem]: typed `std::vector<E>` owned by
                // an opaque handle. Desugars into a class entry plus the
                // Dirac-array-shaped API: 0-arg ctor, n-ary build
                // (`Alias(v0, ...)` snapshots elements; note `Alias(5)`
                // is the one-element vector [5], never a size),
                // `len/get/put/push/pop/str` methods (dot sugar when
                // unambiguous, `Alias_op` aliases always), and delete.
                // `for x in v` does not apply (it lowers to the array
                // builtins); use `vlen` loops or retire to these ops.
                if args.len() != 2 {
                    return Err(err_at(node, "cpp-vector: expects Alias elem"));
                }
                let alias = expect_sym(&args[0], "a vector name")?;
                if d.classes.contains_key(&alias) {
                    return Err(err_at(
                        &args[0],
                        format!("cpp-vector '{alias}' collides with cpp-class '{alias}'"),
                    ));
                }
                if d.by_alias.contains_key(&alias) {
                    return Err(err_at(
                        &args[0],
                        format!("cpp-vector '{alias}' collides with native alias '{alias}'"),
                    ));
                }
                if d.structs.contains_key(&alias) {
                    return Err(err_at(
                        &args[0],
                        format!("cpp-vector '{alias}' collides with cpp-struct '{alias}'"),
                    ));
                }
                if d.callbacks.contains_key(&alias) {
                    return Err(err_at(
                        &args[0],
                        format!("cpp-vector '{alias}' collides with callback '{alias}'"),
                    ));
                }
                let elem = match expect_sym(&args[1], "an element type (int, double, string, ptr, or a cpp-class)")?.as_str() {
                    "int" => NativeTy::Int,
                    "double" => NativeTy::Double,
                    "string" => NativeTy::Str,
                    "ptr" => NativeTy::Ptr,
                    "bool" => {
                        return Err(err_at(
                            &args[1],
                            "cpp-vector: bool elements are bit-packed in std::vector<bool>; use int 0/1",
                        ))
                    }
                    "void" => {
                        return Err(err_at(&args[1], "cpp-vector: void is not an element type"))
                    }
                    c if d.structs.contains_key(c) => {
                        return Err(err_at(
                            &args[1],
                            "cpp-vector: struct elements are not supported",
                        ))
                    }
                    c if d.callbacks.contains_key(c) => {
                        return Err(err_at(
                            &args[1],
                            "cpp-vector: function-pointer elements are not supported",
                        ))
                    }
                    c if d.classes.contains_key(c) => {
                        if d.vecs.contains_key(c) {
                            return Err(err_at(
                                &args[1],
                                "cpp-vector: nested vectors are not supported",
                            ));
                        }
                        NativeTy::Class(c.to_string())
                    }
                    other => {
                        return Err(err_at(&args[1], format!("unknown native type: {other}")))
                    }
                };
                let qualified = format!("std::vector<{}>", vec_elem_cpp(&d, &elem));
                d.classes.insert(
                    alias.clone(),
                    ClassInfo {
                        header: String::new(),
                        qualified,
                        ctors: vec![Vec::new()],
                    },
                );
                d.vecs.insert(alias.clone(), VecInfo { elem: elem.clone() });
                // 0-arg ctor: `Alias()`.
                push_wrapper(
                    &mut d,
                    node,
                    alias.clone(),
                    WrapperKind::Ctor {
                        class: alias.clone(),
                    },
                    Vec::new(),
                    NativeTy::Class(alias.clone()),
                )?;
                // N-ary build: `Alias(v0, ...)`.
                push_wrapper(
                    &mut d,
                    node,
                    alias.clone(),
                    WrapperKind::VecBuild {
                        alias: alias.clone(),
                    },
                    Vec::new(),
                    NativeTy::Class(alias.clone()),
                )?;
                // Element operations (dot sugar + `Alias_op` aliases).
                let ops: &[(&str, Vec<NativeTy>, NativeTy)] = &[
                    ("len", vec![], NativeTy::Int),
                    ("get", vec![NativeTy::Int], elem.clone()),
                    ("put", vec![NativeTy::Int, elem.clone()], NativeTy::Void),
                    ("push", vec![elem.clone()], NativeTy::Void),
                    ("pop", vec![], elem.clone()),
                    ("str", vec![], NativeTy::Str),
                ];
                for (method, params, ret) in ops {
                    push_wrapper(
                        &mut d,
                        node,
                        format!("{alias}_{method}"),
                        WrapperKind::Method {
                            class: alias.clone(),
                            method: method.to_string(),
                            is_static: false,
                        },
                        params.clone(),
                        ret.clone(),
                    )?;
                }
                // Delete the handle.
                push_wrapper(
                    &mut d,
                    node,
                    format!("{alias}_delete"),
                    WrapperKind::Delete { class: alias },
                    Vec::new(),
                    NativeTy::Void,
                )?;
            }
            _ => unreachable!(),
        }
    }
    Ok(d)
}

// ------------------------------------------------------------ shim gen

fn include_line(h: &str) -> String {
    let h = h.trim();
    if h.starts_with('<') || (h.starts_with('"') && h.ends_with('"')) {
        format!("#include {h}")
    } else {
        format!("#include \"{h}\"")
    }
}

/// C++ element type for a vector bridge (`std::vector<E>`).
fn vec_elem_cpp(d: &NativeDecls, elem: &NativeTy) -> String {
    match elem {
        NativeTy::Int => "int32_t".to_string(),
        NativeTy::Double => "double".to_string(),
        NativeTy::Str => "std::string".to_string(),
        NativeTy::Ptr => "void*".to_string(),
        NativeTy::Class(c) => format!("{}*", qual(d, c)),
        NativeTy::Bool => unreachable!("bool vectors are rejected at collect"),
        NativeTy::Void => unreachable!("void vectors are rejected at collect"),
        NativeTy::Struct(_) => unreachable!("struct vectors are rejected at collect"),
        NativeTy::Func(_) => unreachable!("function-pointer vectors are rejected at collect"),
    }
}

/// C spelling of a callback's function type, e.g.
/// `int32_t(*)(int32_t, int32_t)`. Used for `Func` params (the
/// `reinterpret_cast` target) and returns. `is_c` selects the string
/// mapping, matching the trampoline's box/unbox pair.
fn fn_sig_spelling(d: &NativeDecls, alias: &str, is_c: bool) -> String {
    let (ret, params) = fn_sig_parts(d, alias, is_c);
    format!("{ret}(*)({})", params.join(", "))
}

/// Declarator form with a name, e.g. `int32_t (*p0)(int32_t)`: C
/// declarator syntax needs the name inside the parens.
fn fn_sig_decl(d: &NativeDecls, alias: &str, name: &str, is_c: bool) -> String {
    let (ret, params) = fn_sig_parts(d, alias, is_c);
    format!("{ret} (*{name})({})", params.join(", "))
}

fn fn_sig_parts(d: &NativeDecls, alias: &str, is_c: bool) -> (String, Vec<String>) {
    let info = d.callbacks.get(alias).expect("callback registered");
    let str_ty = if is_c { "char*" } else { "std::string" };
    let ret = match &info.ret {
        NativeTy::Int => "int32_t".to_string(),
        NativeTy::Double => "double".to_string(),
        NativeTy::Bool => "bool".to_string(),
        NativeTy::Str => str_ty.to_string(),
        NativeTy::Ptr => "void*".to_string(),
        NativeTy::Void => "void".to_string(),
        other => unreachable!("callback sigs are scalar-only, found {}", other.name()),
    };
    let params: Vec<String> = info
        .params
        .iter()
        .map(|t| match t {
            NativeTy::Int => "int32_t".to_string(),
            NativeTy::Double => "double".to_string(),
            NativeTy::Bool => "bool".to_string(),
            NativeTy::Str => str_ty.to_string(),
            NativeTy::Ptr => "void*".to_string(),
            other => unreachable!("callback sigs are scalar-only, found {}", other.name()),
        })
        .collect();
    (ret, params)
}

/// Fully qualified C++ spelling of a class or struct alias (identity
/// default). Namespaces are disjoint (collisions rejected at collect).
fn qual<'a>(d: &'a NativeDecls, alias: &'a str) -> &'a str {
    d.classes
        .get(alias)
        .map(|c| c.qualified.as_str())
        .or_else(|| d.structs.get(alias).map(|s| s.qualified.as_str()))
        .unwrap_or(alias)
}
fn cpp_param_ty(d: &NativeDecls, t: &NativeTy, is_c: bool) -> String {
    match t {
        NativeTy::Int => "int32_t".to_string(),
        NativeTy::Double => "double".to_string(),
        NativeTy::Bool => "bool".to_string(),
        NativeTy::Str if is_c => "char*".to_string(),
        NativeTy::Str => "std::string".to_string(),
        NativeTy::Void => "void".to_string(),
        NativeTy::Ptr => "void*".to_string(),
        NativeTy::Class(c) => format!("{}*", qual(d, c)),
        NativeTy::Struct(s) => qual(d, s).to_string(),
        NativeTy::Func(f) => fn_sig_spelling(d, f, is_c),
    }
}

fn cpp_ret_ty(d: &NativeDecls, t: &NativeTy, is_c: bool) -> String {
    match t {
        NativeTy::Str if is_c => "char*".to_string(),
        NativeTy::Str => "std::string".to_string(),
        other => cpp_param_ty(d, other, is_c),
    }
}

/// Unbox `a{i}` (a boxed `Value*`) into a shim local `p{i}`.
fn unbox_lines(d: &NativeDecls, out: &mut String, i: usize, t: &NativeTy, is_c: bool) {
    unbox_expr(d, out, &format!("a{i}"), &format!("p{i}"), t, is_c);
}

/// Unbox `a[i]` (n-ary array element) into a shim local `p{i}`.
/// Same conversions as [`unbox_lines`], indexed for `(Value**, i32)`.
fn unbox_nary_lines(d: &NativeDecls, out: &mut String, i: usize, t: &NativeTy, is_c: bool) {
    unbox_expr(d, out, &format!("a[{i}]"), &format!("p{i}"), t, is_c);
}

/// Unbox expression `src` (a `Value*`) into C++ local `dst` per `t`.
/// Structs convert field by field from the Dirac array (fields in
/// order); anything else aborts with a `cannot pass ...` diagnostic
/// in the usual conversion-failure shape.
fn unbox_expr(d: &NativeDecls, out: &mut String, src: &str, dst: &str, t: &NativeTy, is_c: bool) {
    match t {
        NativeTy::Int => out.push_str(&format!("  int32_t {dst} = cdi_to_int({src});\n")),
        NativeTy::Double => out.push_str(&format!("  double {dst} = cdi_to_double({src});\n")),
        NativeTy::Bool => out.push_str(&format!("  bool {dst} = cdi_to_bool({src}) != 0;\n")),
        NativeTy::Str if is_c => out.push_str(&format!("  char* {dst} = cdi_to_cstring({src});\n")),
        NativeTy::Str => out.push_str(&format!(
            // NOTE: sequenced statements, not one expression: argument
            // evaluation order would otherwise read pN_n before the
            // call that sets it (empty-string bug).
            "  int32_t {dst}_n = 0;\n  const char* {dst}_b = cdi_str_bytes({src}, &{dst}_n);\n  std::string {dst}({dst}_b, (size_t){dst}_n);\n"
        )),
        NativeTy::Ptr => out.push_str(&format!("  void* {dst} = cdi_expect_handle({src}, \"arg\");\n")),
        NativeTy::Class(c) => {
            let q = qual(d, c);
            out.push_str(&format!("  {q}* {dst} = ({q}*)cdi_expect_handle({src}, \"arg\");\n"))
        }
        NativeTy::Func(f) => {
            let sig = fn_sig_spelling(d, f, is_c);
            let decl = fn_sig_decl(d, f, dst, is_c);
            out.push_str(&format!("  {decl} = reinterpret_cast<{sig}>(cdi_expect_handle({src}, \"arg\"));\n"))
        }
        NativeTy::Struct(s) => unbox_struct(d, out, src, dst, s, is_c),
        NativeTy::Void => {}
    }
}

/// Unbox a by-value struct from its Dirac array spelling.
fn unbox_struct(d: &NativeDecls, out: &mut String, src: &str, dst: &str, s: &str, is_c: bool) {
    let info = d.structs.get(s).expect("struct registered");
    let q = info.qualified.clone();
    let fields: Vec<(String, NativeTy)> = info.fields.clone();
    let k = fields.len();
    out.push_str(&format!("  if ({src}->tag != CDI_ARRAY) cdi_error(\"cannot pass value as {s} (expects array of {k} fields)\");\n"));
    out.push_str(&format!("  if (cdi_to_int(cdi_len({src})) != {k}) cdi_error(\"cannot pass array as {s} (expects {k} fields)\");\n"));
    for (j, (_, fty)) in fields.iter().enumerate() {
        out.push_str(&format!(
            "  Value* {dst}_v{j} = cdi_get({src}, cdi_mk_int({j}));\n"
        ));
        unbox_expr(
            d,
            out,
            &format!("{dst}_v{j}"),
            &format!("{dst}_f{j}"),
            fty,
            is_c,
        );
    }
    let inits: Vec<String> = (0..k).map(|j| format!("{dst}_f{j}")).collect();
    // Brace init: paren-init of aggregates without ctors is C++20;
    // the shim floor is C++17.
    out.push_str(&format!("  {q} {dst}{{{}}};\n", inits.join(", ")));
}
/// Box C++ expression `src` of type `t` into fresh `Value* dst`.
/// Structs build the Dirac array spelling field by field (inverse of
/// [`unbox_struct`]); nested structs recurse.
fn box_into(d: &NativeDecls, out: &mut String, dst: &str, src: &str, t: &NativeTy, is_c: bool) {
    match t {
        NativeTy::Int => out.push_str(&format!("  Value* {dst} = cdi_from_int({src});\n")),
        NativeTy::Double => out.push_str(&format!("  Value* {dst} = cdi_from_double({src});\n")),
        NativeTy::Bool => {
            out.push_str(&format!("  Value* {dst} = cdi_from_bool({src} ? 1 : 0);\n"))
        }
        NativeTy::Str if is_c => {
            out.push_str(&format!("  Value* {dst} = cdi_from_cstring({src});\n"))
        }
        NativeTy::Str => out.push_str(&format!(
            "  Value* {dst} = cdi_mk_str({src}.data(), (int32_t){src}.size());\n"
        )),
        NativeTy::Ptr => out.push_str(&format!("  Value* {dst} = cdi_from_ptr((void*){src});\n")),
        NativeTy::Class(_) => {
            out.push_str(&format!("  Value* {dst} = cdi_from_ptr((void*){src});\n"))
        }
        NativeTy::Func(_) => {
            out.push_str(&format!("  Value* {dst} = cdi_from_ptr((void*){src});\n"))
        }
        NativeTy::Struct(s) => {
            let info = d.structs.get(s).expect("struct registered");
            let fields: Vec<(String, NativeTy)> = info.fields.clone();
            out.push_str(&format!("  Value* {dst} = cdi_array0();\n"));
            for (j, (fname, fty)) in fields.iter().enumerate() {
                box_into(
                    d,
                    out,
                    &format!("{dst}_f{j}"),
                    &format!("({src}).{fname}"),
                    fty,
                    is_c,
                );
                out.push_str(&format!("  cdi_arr_push({dst}, {dst}_f{j});\n"));
            }
        }
        NativeTy::Void => unreachable!("boxed temps are never void"),
    }
}

/// Box a shim-local `r` of the natural C++ type for `ret`.
/// (Always the C++ spelling: field getters are class-side.)
fn box_temp(d: &NativeDecls, out: &mut String, ret: &NativeTy) {
    if let NativeTy::Struct(_) = ret {
        box_into(d, out, "r_boxed", "r", ret, false);
        out.push_str("  return r_boxed;\n");
        return;
    }
    // Scalar arms below are byte-pinned by tests; keep verbatim.
    match ret {
        NativeTy::Int => out.push_str("  return cdi_from_int(r);\n"),
        NativeTy::Double => out.push_str("  return cdi_from_double(r);\n"),
        NativeTy::Bool => out.push_str("  return cdi_from_bool(r ? 1 : 0);\n"),
        NativeTy::Str => out.push_str("  return cdi_mk_str(r.data(), (int32_t)r.size());\n"),
        NativeTy::Ptr => out.push_str("  return cdi_from_ptr((void*)r);\n"),
        NativeTy::Class(_) => out.push_str("  return cdi_from_ptr((void*)r);\n"),
        NativeTy::Func(_) => out.push_str("  return cdi_from_ptr((void*)r);\n"),
        NativeTy::Struct(_) | NativeTy::Void => unreachable!("handled above / never void"),
    }
}

/// Box a call result into the wrapper return.
fn box_result(d: &NativeDecls, out: &mut String, call: &str, ret: &NativeTy, is_c: bool) {
    if let NativeTy::Struct(_) = ret {
        let q = cpp_param_ty(d, ret, is_c);
        out.push_str(&format!("  {q} r = {call};\n"));
        box_into(d, out, "r_boxed", "r", ret, is_c);
        out.push_str("  return r_boxed;\n");
        return;
    }
    match ret {
        NativeTy::Void => {
            out.push_str(&format!("  {call};\n  return cdi_void();\n"));
        }
        NativeTy::Int => out.push_str(&format!(
            "  int32_t r = {call};\n  return cdi_from_int(r);\n"
        )),
        NativeTy::Double => out.push_str(&format!(
            "  double r = {call};\n  return cdi_from_double(r);\n"
        )),
        NativeTy::Bool => out.push_str(&format!(
            "  bool r = {call};\n  return cdi_from_bool(r ? 1 : 0);\n"
        )),
        NativeTy::Str if is_c => out.push_str(&format!(
            "  char* r = {call};\n  return cdi_from_cstring(r);\n"
        )),
        NativeTy::Str => out.push_str(&format!(
            "  std::string r = {call};\n  return cdi_mk_str(r.data(), (int32_t)r.size());\n"
        )),
        NativeTy::Ptr => out.push_str(&format!(
            "  void* r = (void*)({call});\n  return cdi_from_ptr(r);\n"
        )),
        NativeTy::Class(_) => out.push_str(&format!(
            "  void* r = (void*)({call});\n  return cdi_from_ptr(r);\n"
        )),
        NativeTy::Func(_) => out.push_str(&format!(
            "  void* r = (void*)({call});\n  return cdi_from_ptr(r);\n"
        )),
        NativeTy::Struct(_) => unreachable!("handled above"),
    }
}

fn wrapper_sig(w: &Wrapper) -> String {
    // Instance methods, field accessors, and delete take the handle as
    // the implicit first argument (see wrapper_arity). N-ary builds and
    // varargs take the Dirac n-ary shape (array + count).
    if matches!(&w.kind, WrapperKind::VecBuild { .. }) || is_varargs(&w.kind) {
        return format!("Value* cdi_wrap_{}(Value** a, int32_t n)", w.idx);
    }
    let n = wrapper_arity(w);
    let args: Vec<String> = (0..n).map(|i| format!("Value* a{i}")).collect();
    format!("Value* cdi_wrap_{}({})", w.idx, args.join(", "))
}

/// Vector element operation (`len/get/put/push/pop/str` on a
/// `[cpp-vector Alias elem]` bridge). Bounds failures use the wrapper
/// alias as head (`IntVec_get: index ...`), mirroring Dirac shapes.
fn vec_method_body(d: &NativeDecls, out: &mut String, w: &Wrapper, class: &str, method: &str) {
    let vec = d.vecs.get(class).expect("vec bridge registered");
    let q = qual(d, class);
    let e = vec_elem_cpp(d, &vec.elem);
    out.push_str(&format!(
        "  {q}* self = ({q}*)cdi_expect_handle(a0, \"self\");\n"
    ));
    match method {
        "len" => {
            out.push_str("  int32_t r = (int32_t)self->size();\n  return cdi_from_int(r);\n");
        }
        "get" | "put" => {
            out.push_str("  int32_t i = cdi_to_int(a1);\n");
            out.push_str(&format!(
                "  if (i < 0 || (size_t)i >= self->size()) {{\n    char __m[160];\n    snprintf(__m, sizeof(__m), \"{}: index %d out of range for length %d\", (int)i, (int)self->size());\n    cdi_error(__m);\n  }}\n",
                w.alias
            ));
            if method == "get" {
                out.push_str(&format!("  {e} r = (*self)[(size_t)i];\n"));
                // Box through the element type (get's ret IS the elem).
                box_temp(d, out, &vec.elem);
            } else {
                unbox_lines(d, out, 2, &vec.elem, false);
                out.push_str("  (*self)[(size_t)i] = p2;\n  return cdi_void();\n");
            }
        }
        "push" => {
            unbox_lines(d, out, 1, &vec.elem, false);
            out.push_str("  self->push_back(p1);\n  return cdi_void();\n");
        }
        "pop" => {
            out.push_str(&format!(
                "  if (self->empty()) {{ cdi_error(\"{}: empty vector\"); }}\n",
                w.alias
            ));
            out.push_str(&format!("  {e} r = self->back();\n  self->pop_back();\n"));
            box_temp(d, out, &vec.elem);
        }
        "str" => {
            // Debug rendering in Dirac write spellings (quoted strings,
            // Java-shaped floats) via temp Values; leak-by-design like
            // the rest of the runtime.
            out.push_str("  std::string __s = \"[\";\n");
            out.push_str("  for (size_t __k = 0; __k < self->size(); __k++) {\n");
            out.push_str("    if (__k) __s += \", \";\n");
            match &vec.elem {
                NativeTy::Int => out.push_str("    Value* __v = cdi_from_int((*self)[__k]);\n"),
                NativeTy::Double => out.push_str("    Value* __v = cdi_from_double((*self)[__k]);\n"),
                NativeTy::Str => out.push_str("    Value* __v = cdi_mk_str((*self)[__k].data(), (int32_t)(*self)[__k].size());\n"),
                NativeTy::Ptr => out.push_str("    Value* __v = cdi_from_ptr((*self)[__k]);\n"),
                NativeTy::Class(c) => {
                    let qc = qual(d, c);
                    out.push_str(&format!("    Value* __v = cdi_from_ptr((void*)(({qc}*)(*self)[__k]));\n"))
                }
                NativeTy::Bool | NativeTy::Void => unreachable!("vec elements are never bool/void"),
                NativeTy::Struct(_) => unreachable!("vec elements are never structs"),
                NativeTy::Func(_) => unreachable!("vec elements are never function pointers"),
            }
            out.push_str(
                "    char* __w = cdi_write_cstr(__v);\n    __s += __w;\n  }\n  __s += \"]\";\n",
            );
            out.push_str("  return cdi_mk_str(__s.data(), (int32_t)__s.size());\n");
        }
        _ => unreachable!("vec bridges only register len/get/put/push/pop/str"),
    }
}

/// N-ary vector build body: snapshot `a[0..n]` into a fresh vector.
fn vec_build_body(d: &NativeDecls, out: &mut String, alias: &str) {
    let vec = d.vecs.get(alias).expect("vec bridge registered");
    let q = qual(d, alias);
    out.push_str(&format!("  {q}* v = new {q}();\n"));
    out.push_str("  if (n > 0) v->reserve((size_t)n);\n");
    out.push_str("  for (int32_t k = 0; k < n; k++) {\n");
    match &vec.elem {
        NativeTy::Int => out.push_str("    int32_t __e = cdi_to_int(a[k]);\n"),
        NativeTy::Double => out.push_str("    double __e = cdi_to_double(a[k]);\n"),
        NativeTy::Str => out.push_str(
            // Sequenced (see the evaluation-order note in unbox_lines).
            "    int32_t __e_n = 0;\n    const char* __e_b = cdi_str_bytes(a[k], &__e_n);\n    std::string __e(__e_b, (size_t)__e_n);\n",
        ),
        NativeTy::Ptr => out.push_str("    void* __e = cdi_expect_handle(a[k], \"arg\");\n"),
        NativeTy::Class(c) => {
            let qc = qual(d, c);
            out.push_str(&format!("    {qc}* __e = ({qc}*)cdi_expect_handle(a[k], \"arg\");\n"))
        }
        NativeTy::Bool | NativeTy::Void => unreachable!("vec elements are never bool/void"),
        NativeTy::Struct(_) => unreachable!("vec elements are never structs"),
        NativeTy::Func(_) => unreachable!("vec elements are never function pointers"),
    }
    out.push_str("    v->push_back(__e);\n  }\n");
    out.push_str("  return cdi_from_ptr((void*)v);\n");
}

/// Varargs forwarding body (`extern-c/cpp … varargs`).
///
/// The Dirac side packs ALL args (fixed + extras) into `(a, n)`. Fixed
/// params unbox normally; each extra unboxes by its *runtime* tag to its
/// natural C type (C promotions apply: `bool`→`int`, `float`→`double`).
/// A flat per-arity tag-combination switch keeps every call site
/// statically typed: `case` digits are base-4 (`0=int|bool, 1=double,
/// `2=string, 3=ptr`), so arity k costs 4^k cases (cap MAX_VARARGS).
/// Anything else (arrays, `nil`, …) aborts `cannot pass … as vararg`,
/// mirroring the `cinterop` conversion-failure shape.
fn varargs_body(d: &NativeDecls, out: &mut String, w: &Wrapper, callee: &str, is_c: bool) {
    let k = w.params.len();
    for (i, t) in w.params.iter().enumerate() {
        unbox_nary_lines(d, out, i, t, is_c);
    }
    // NOTE: fixed params read `a[i]`; the n-ary shape guarantees n >= k
    // at every call site (llvmgen checks), but belt-and-braces anyway.
    out.push_str(&format!(
        "  if (n < {k}) {{ cdi_error(\"{a}: expects at least {k} argument(s)\"); }}\n",
        a = w.alias
    ));
    out.push_str(&format!(
        "  if (n - {k} > {m}) {{ cdi_error(\"{a}: at most {m} varargs\"); }}\n",
        m = MAX_VARARGS,
        a = w.alias
    ));
    // Tag-of-extra: 0=int|bool, 1=double, 2=string, 3=ptr. Reads the
    // boxed tag directly (value.h is included in the shim).
    out.push_str("  int __nx = n;\n");
    out.push_str(&format!("  __nx -= {k};\n"));
    out.push_str("  int __combo = 0;\n");
    out.push_str(&format!("  for (int __j = {k}; __j < n; __j++) {{\n"));
    out.push_str("    int __t = a[__j]->tag;\n");
    out.push_str("    int __d = (__t == CDI_INT || __t == CDI_BOOL) ? 0\n");
    out.push_str("      : (__t == CDI_FLOAT) ? 1\n");
    out.push_str("      : (__t == CDI_STR) ? 2\n");
    out.push_str("      : (__t == CDI_PTR) ? 3 : -1;\n");
    out.push_str(&format!(
        "    if (__d < 0) {{ cdi_error(\"{a}: cannot pass vararg\"); }}\n",
        a = w.alias
    ));
    out.push_str("    __combo = __combo * 4 + __d;\n  }\n");
    let fixed: Vec<String> = (0..k).map(|i| format!("p{i}")).collect();
    let call_with = |extra: &[String]| {
        let mut all = fixed.clone();
        all.extend(extra.iter().cloned());
        format!("{callee}({})", all.join(", "))
    };
    out.push_str("  switch (__nx) {\n");
    for nx in 0..=MAX_VARARGS {
        out.push_str(&format!("  case {nx}: {{\n"));
        if nx == 0 {
            box_result_prefixed(d, out, "    ", &call_with(&[]), &w.ret, is_c);
        } else {
            out.push_str("    switch (__combo) {\n");
            let cases = 4usize.pow(nx as u32);
            for combo in 0..cases {
                // base-4 digits, most significant first (extra 0 first).
                let mut digits = vec![0usize; nx];
                let mut c = combo;
                for j in (0..nx).rev() {
                    digits[j] = c % 4;
                    c /= 4;
                }
                out.push_str(&format!("    case {combo}: {{\n"));
                let mut extra = Vec::new();
                for (j, dg) in digits.iter().enumerate() {
                    let v = format!("a[{k} + {j}]");
                    match dg {
                        0 => {
                            out.push_str(&format!(
                                "      int32_t e{j} = (a[{kk} + {j}]->tag == CDI_BOOL) ? cdi_to_bool({v}) : cdi_to_int({v});\n",
                                kk = k,
                                v = v
                            ));
                            extra.push(format!("e{j}"));
                        }
                        1 => {
                            out.push_str(&format!(
                                "      double e{j} = cdi_to_double({v});\n",
                                v = v
                            ));
                            extra.push(format!("e{j}"));
                        }
                        2 if is_c => {
                            out.push_str(&format!(
                                "      char* e{j} = cdi_to_cstring({v});\n",
                                v = v
                            ));
                            extra.push(format!("e{j}"));
                        }
                        2 => {
                            out.push_str(&format!(
                                "      int32_t e{j}_n = 0;\n      const char* e{j}_b = cdi_str_bytes({v}, &e{j}_n);\n      std::string e{j}(e{j}_b, (size_t)e{j}_n);\n",
                                v = v
                            ));
                            extra.push(format!("e{j}"));
                        }
                        _ => {
                            out.push_str(&format!(
                                "      void* e{j} = cdi_expect_handle({v}, \"vararg\");\n",
                                v = v
                            ));
                            extra.push(format!("e{j}"));
                        }
                    }
                }
                box_result_prefixed(d, out, "      ", &call_with(&extra), &w.ret, is_c);
                out.push_str("    }\n");
            }
            out.push_str("    }\n");
        }
        out.push_str("  }\n");
    }
    out.push_str("  }\n");
}

/// `box_result` with an indent prefix (for emission inside switches).
fn box_result_prefixed(
    d: &NativeDecls,
    out: &mut String,
    pad: &str,
    call: &str,
    ret: &NativeTy,
    is_c: bool,
) {
    if let NativeTy::Struct(_) = ret {
        let q = cpp_param_ty(d, ret, is_c);
        out.push_str(&format!("{pad}{q} r = {call};\n"));
        box_into(d, out, "r_boxed", "r", ret, is_c);
        out.push_str(&format!("{pad}return r_boxed;\n"));
        return;
    }
    match ret {
        NativeTy::Void => {
            out.push_str(&format!("{pad}{call};\n{pad}return cdi_void();\n"));
        }
        NativeTy::Int => out.push_str(&format!(
            "{pad}int32_t r = {call};\n{pad}return cdi_from_int(r);\n"
        )),
        NativeTy::Double => out.push_str(&format!(
            "{pad}double r = {call};\n{pad}return cdi_from_double(r);\n"
        )),
        NativeTy::Bool => out.push_str(&format!(
            "{pad}bool r = {call};\n{pad}return cdi_from_bool(r ? 1 : 0);\n"
        )),
        NativeTy::Str if is_c => out.push_str(&format!(
            "{pad}char* r = {call};\n{pad}return cdi_from_cstring(r);\n"
        )),
        NativeTy::Str => out.push_str(&format!(
            "{pad}std::string r = {call};\n{pad}return cdi_mk_str(r.data(), (int32_t)r.size());\n"
        )),
        NativeTy::Ptr => out.push_str(&format!(
            "{pad}void* r = (void*)({call});\n{pad}return cdi_from_ptr(r);\n"
        )),
        NativeTy::Class(_) => out.push_str(&format!(
            "{pad}void* r = (void*)({call});\n{pad}return cdi_from_ptr(r);\n"
        )),
        NativeTy::Func(_) => out.push_str(&format!(
            "{pad}void* r = (void*)({call});\n{pad}return cdi_from_ptr(r);\n"
        )),
        NativeTy::Struct(_) => unreachable!("handled above"),
    }
}

fn wrapper_body(d: &NativeDecls, w: &Wrapper) -> String {
    // C decls call the C symbol; C++ decls call qualified names.
    // Classes/methods/fields/ctors always need C++ (shim is C++ anyway).
    let mut out = String::new();
    // Unbox fixed params. Methods take `this` as a0 outside `params`
    // (see call shapes below); ctors/fields have no extra handling.
    match &w.kind {
        WrapperKind::Method {
            class,
            method,
            is_static: false,
        } if d.vecs.contains_key(class) => {
            vec_method_body(d, &mut out, w, class, method);
        }
        WrapperKind::VecBuild { alias } => {
            vec_build_body(d, &mut out, alias);
        }
        WrapperKind::Method {
            class,
            method,
            is_static: false,
        } => {
            let q = qual(d, class);
            out.push_str(&format!(
                "  {q}* self = ({q}*)cdi_expect_handle(a0, \"self\");\n"
            ));
            for (i, t) in w.params.iter().enumerate() {
                unbox_lines(d, &mut out, i + 1, t, false);
            }
            let args: Vec<String> = (0..w.params.len()).map(|i| format!("p{}", i + 1)).collect();
            box_result(
                d,
                &mut out,
                &format!("self->{method}({})", args.join(", ")),
                &w.ret,
                false,
            );
        }
        WrapperKind::Method {
            class,
            method,
            is_static: true,
        } => {
            let q = qual(d, class);
            for (i, t) in w.params.iter().enumerate() {
                unbox_lines(d, &mut out, i, t, false);
            }
            let args: Vec<String> = (0..w.params.len()).map(|i| format!("p{i}")).collect();
            box_result(
                d,
                &mut out,
                &format!("{q}::{method}({})", args.join(", ")),
                &w.ret,
                false,
            );
        }
        WrapperKind::Ctor { class } => {
            let q = qual(d, class);
            for (i, t) in w.params.iter().enumerate() {
                unbox_lines(d, &mut out, i, t, false);
            }
            let args: Vec<String> = (0..w.params.len()).map(|i| format!("p{i}")).collect();
            out.push_str(&format!("  {q}* r = new {q}({});\n", args.join(", ")));
            out.push_str("  return cdi_from_ptr((void*)r);\n");
        }
        WrapperKind::FieldGet { class, field } => {
            let q = qual(d, class);
            out.push_str(&format!(
                "  {q}* self = ({q}*)cdi_expect_handle(a0, \"self\");\n"
            ));
            let ty = cpp_ret_ty(d, &w.ret, false);
            out.push_str(&format!("  {ty} r = self->{field};\n"));
            box_temp(d, &mut out, &w.ret);
        }
        WrapperKind::FieldSet { class, field } => {
            let q = qual(d, class);
            out.push_str(&format!(
                "  {q}* self = ({q}*)cdi_expect_handle(a0, \"self\");\n"
            ));
            unbox_lines(d, &mut out, 1, &w.params[0], false);
            // string field: assign via std::string ctor already in p1.
            out.push_str(&format!("  self->{field} = p1;\n  return cdi_void();\n"));
        }
        WrapperKind::Delete { class } => {
            let q = qual(d, class);
            out.push_str(&format!(
                "  {q}* self = ({q}*)cdi_expect_handle(a0, \"self\");\n  delete self;\n  return cdi_void();\n"
            ));
        }
        WrapperKind::Const { qualified, ty } => {
            // Snapshot with a cast: scoped enums don't convert to int
            // implicitly, and macros may be any arithmetic type.
            match ty {
                NativeTy::Int => out.push_str(&format!("  int32_t r = (int32_t)({qualified});\n")),
                NativeTy::Double => out.push_str(&format!("  double r = (double)({qualified});\n")),
                NativeTy::Bool => out.push_str(&format!("  bool r = (bool)({qualified});\n")),
                NativeTy::Str => out.push_str(&format!("  std::string r = ({qualified});\n")),
                _ => unreachable!("consts are int, double, bool, or string"),
            }
            box_temp(d, &mut out, ty);
        }
        WrapperKind::StaticGet { class, field } => {
            let q = qual(d, class);
            let ty = cpp_ret_ty(d, &w.ret, false);
            out.push_str(&format!("  {ty} r = {q}::{field};\n"));
            box_temp(d, &mut out, &w.ret);
        }
        WrapperKind::StaticSet { class, field } => {
            let q = qual(d, class);
            unbox_lines(d, &mut out, 0, &w.params[0], false);
            out.push_str(&format!("  {q}::{field} = p0;\n  return cdi_void();\n"));
        }
        WrapperKind::ExternC { c_sym, varargs } => {
            if *varargs {
                varargs_body(d, &mut out, w, &format!("::{c_sym}"), true);
                return out;
            }
            for (i, t) in w.params.iter().enumerate() {
                unbox_lines(d, &mut out, i, t, true);
            }
            let args: Vec<String> = (0..w.params.len()).map(|i| format!("p{i}")).collect();
            // Call the C symbol in global scope.
            box_result(
                d,
                &mut out,
                &format!("::{c_sym}({})", args.join(", ")),
                &w.ret,
                true,
            );
        }
        WrapperKind::ExternCpp { qualified, varargs } => {
            if *varargs {
                varargs_body(d, &mut out, w, qualified, false);
                return out;
            }
            for (i, t) in w.params.iter().enumerate() {
                unbox_lines(d, &mut out, i, t, false);
            }
            let args: Vec<String> = (0..w.params.len()).map(|i| format!("p{i}")).collect();
            box_result(
                d,
                &mut out,
                &format!("{qualified}({})", args.join(", ")),
                &w.ret,
                false,
            );
        }
    }
    out
}

/// Generate the C++ shim source for `decls` (compile with `g++ -c`).
pub fn gen_cpp_shim(d: &NativeDecls) -> String {
    let mut out = String::from(
        "// Generated by cdirac --emit-cpp-shim. Compile with:\n//   g++ -std=c++17 -c cdi_shim.cpp -o cdi_shim.o -I llvmrt <flags>\n// then link the objects with your program (see --emit-llvm header).\n#include <cstddef>\n#include <cstdint>\n#include <cstdio>\n#include <string>\n\n#include \"cinterop.h\"\n",
    );
    if !d.vecs.is_empty() {
        out.push_str("#include <vector>\n");
    }
    if !d.structs.is_empty() {
        out.push_str("#include \"containers.h\"\n");
    }
    for h in d.c_headers.iter().chain(d.cpp_headers.iter()) {
        out.push_str(&include_line(h));
        out.push('\n');
    }
    out.push('\n');
    for w in &d.wrappers {
        out.push_str(&format!("extern \"C\" {} {{\n", wrapper_sig(w)));
        out.push_str(&wrapper_body(d, w));
        out.push_str("}\n\n");
    }
    out
}

/// LLVM `declare` lines for every wrapper (called with boxed `Value*`s).
/// Instance methods/field-get/set/delete take the handle as first arg;
/// n-ary builds and varargs take the Dirac n-ary shape (array + count).
pub fn wrapper_declares(d: &NativeDecls) -> String {
    let mut out = String::new();
    for w in &d.wrappers {
        if matches!(&w.kind, WrapperKind::VecBuild { .. }) || is_varargs(&w.kind) {
            out.push_str(&format!("declare ptr @cdi_wrap_{}(ptr, i32)\n", w.idx));
            continue;
        }
        let extra = match &w.kind {
            WrapperKind::Method {
                is_static: false, ..
            }
            | WrapperKind::FieldGet { .. }
            | WrapperKind::FieldSet { .. }
            | WrapperKind::Delete { .. } => 1,
            _ => 0,
        };
        let n = w.params.len() + extra;
        let args = vec!["ptr"; n].join(", ");
        out.push_str(&format!("declare ptr @cdi_wrap_{}({args})\n", w.idx));
    }
    out
}

/// Wrapper call arity on the Dirac side (params + implicit handle).
/// Wrapper call arity on the Dirac side (params + implicit handle).
/// VecBuild matches any arity (`usize::MAX`); dispatch it explicitly
/// via `find_vec_build` after fixed overloads.
pub fn wrapper_arity(w: &Wrapper) -> usize {
    wrapper_call_arity(&w.kind, w.params.len())
}

/// Dirac-side call arity from kind + param count. VecBuild matches any
/// arity, so it reports `usize::MAX` to stay out of every (alias,
/// arity) equality check (dup detection, fixed dispatch).
fn wrapper_call_arity(kind: &WrapperKind, n_params: usize) -> usize {
    if matches!(kind, WrapperKind::VecBuild { .. }) {
        return usize::MAX;
    }
    n_params
        + match kind {
            WrapperKind::Method {
                is_static: false, ..
            }
            | WrapperKind::FieldGet { .. }
            | WrapperKind::FieldSet { .. }
            | WrapperKind::Delete { .. } => 1,
            _ => 0,
        }
}
