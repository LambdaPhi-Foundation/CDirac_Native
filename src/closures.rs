//! closures: static capture analysis for nested procedures.
//!
//! Nested `fn` (defined inside a proc body or the top-level flow) lifts
//! to a top-level LLVM function. Captures are **by reference**: each
//! capture becomes a hidden leading `ptr` parameter holding the
//! *address* of the caller's slot, so `set` inside the nested proc
//! writes through to the outer variable. Captures evaluate **at each
//! call** from the caller's frame (fresh values; mutations visible).
//!
//! This module only *computes* capture sets (pure AST walk, total — it
//! never errors). Codegen (`llvmgen`) consumes them: prologues bind
//! captures as locals, calls pass slot addresses, and any imprecision
//! here degrades to the usual `unbound identifier` / `cannot capture`
//! errors there (never a miscompile).
//!
//! Rules baked into the walk (mirroring codegen name resolution):
//!
//! * Call heads are never variable reads — except the desugared method
//!   form `[recv method args...]`, where a *bound* head is the receiver
//!   (a real read) and `method` is skipped. Anything else in head
//!   position is a proc/builtin/native/keyword reference.
//! * Dotted symbols (`a.size`) read their head part (`a`).
//! * `Var`/`Const` bind (values walked first); `Set` reads its value
//!   plus its target (writes need the slot too).
//! * Nested `Proc` names are callable, never capturable; siblings are
//!   callable in codegen after their definition point (sequential —
//!   codegen enforces the order, the walk over-approximates).
//! * Propagation: `cap(X)` covers X's own uses plus every proc defined
//!   in or lexically visible-called from X's subtree (forwarding
//!   through intermediaries), minus X's params/own-vars and globals
//!   (builtins, top procs, top-level vars, natives, `machine`,
//!   machine-only ops). Iterated to a fixpoint (mutual recursion).

use std::collections::{HashMap, HashSet};

use crate::ast::{Node, NodeKind};
use crate::native::NativeDecls;

/// Key for one nested definition: ancestor path + name + span.
/// The span makes the key robust against walk-order skew (a miss then
/// degrades to a graceful error in codegen, never wrong caps).
pub fn nest_key(path: &[String], name: &str, line: usize, col: usize) -> String {
    format!("{}\0{name}\0{line}:{col}", path.join("\0"))
}

/// A top-level item for analysis: name, params, body. `@main` names the
/// top-level flow (params always empty).
pub type TopItem = (String, Vec<(String, bool)>, Vec<Node>);

/// Capture map: nest key -> ordered `(name, is_const)` captures.
pub struct ClosureMaps {
    pub caps: HashMap<String, Vec<(String, bool)>>,
}

struct ProcScan {
    /// Ancestor proc names (`[]` for top-level, `["@main"]` under main).
    path: Vec<String>,
    name: String,
    params: Vec<String>,
    /// Own bindings in this body (any depth, excluding nested bodies).
    bindings: HashMap<String, bool>,
    /// Ordered uses in this body (excluding nested bodies).
    uses: Vec<String>,
    /// Nested proc names called in this body (excluding nested bodies).
    nested_calls: Vec<String>,
    /// Directly nested definitions.
    children: Vec<ProcScan>,
    /// All nested names defined in the parent body (siblings + self,
    /// callable but never capturable).
    parent_nests: HashSet<String>,
    /// Descendant proc names (callable, never capturable).
    descendants: HashSet<String>,
    caps: Vec<(String, bool)>,
    line: usize,
    col: usize,
}

struct LexEnv<'a> {
    /// Lexical block stack of bound names (params + vars/consts so far).
    vars: Vec<HashSet<String>>,
    top_procs: &'a HashSet<String>,
    globals: &'a HashSet<String>,
    native: &'a NativeDecls,
}

fn is_builtin_name(n: &str) -> bool {
    crate::llvmgen::is_builtin_name(n)
}

impl<'a> LexEnv<'a> {
    fn is_bound(&self, name: &str) -> bool {
        self.vars.iter().any(|s| s.contains(name))
    }
}

/// Names that resolve without any capture.
fn is_globalish(env: &LexEnv, u: &str) -> bool {
    u == "machine"
        || u == "alloc"
        || u == "dealloc"
        || is_builtin_name(u)
        || env.top_procs.contains(u)
        || env.globals.contains(u)
        || env.native.by_alias.contains_key(u)
        || env.native.classes.contains_key(u)
        || env.native.find_const(u).is_some()
}

fn push_use(uses: &mut Vec<String>, seen: &mut HashSet<String>, name: &str) {
    if seen.insert(name.to_string()) {
        uses.push(name.to_string());
    }
}

fn split_dot(s: &str) -> Option<(&str, &str)> {
    let mut it = s.splitn(2, '.');
    match (it.next(), it.next()) {
        (Some(a), Some(b)) if !a.is_empty() && !b.is_empty() => Some((a, b)),
        _ => None,
    }
}

fn scan_value(node: &Node, env: &LexEnv, uses: &mut Vec<String>, seen: &mut HashSet<String>) {
    match &node.kind {
        NodeKind::Int(_)
        | NodeKind::Float(_)
        | NodeKind::Bool(_)
        | NodeKind::Str(_)
        | NodeKind::Nil => {}
        NodeKind::Sym(s) => {
            if let Some((head, _)) = split_dot(s) {
                push_use(uses, seen, head);
            } else {
                push_use(uses, seen, s);
            }
        }
        NodeKind::Var { val, .. } | NodeKind::Const { val, .. } | NodeKind::Set { val, .. } => {
            // Value-position binding forms (codegen rejects these, but
            // the walk stays total): just descend into the value.
            scan_value(val, env, uses, seen);
            if let NodeKind::Set { name, .. } = &node.kind {
                if let Some((head, _)) = split_dot(name) {
                    push_use(uses, seen, head);
                } else {
                    push_use(uses, seen, name);
                }
            }
        }
        NodeKind::Proc { .. } => {
            // Proc in value position (codegen rejects it): descend
            // structurally so the walk stays total.
            walk_proc_shallow(node, env, uses, seen);
        }
        NodeKind::List(elems, tail) => {
            if elems.is_empty() {
                if let Some(t) = tail {
                    scan_value(t, env, uses, seen);
                }
                return;
            }
            if let NodeKind::Sym(h) = &elems[0].kind {
                if let Some((recv, _)) = split_dot(h) {
                    // Dotted head: receiver read.
                    push_use(uses, seen, recv);
                    for e in &elems[1..] {
                        scan_value(e, env, uses, seen);
                    }
                } else if env.is_bound(h) || env.globals.contains(h.as_str()) {
                    // Desugared method form `[recv method args...]`:
                    // receiver is a real read, `method` is skipped.
                    push_use(uses, seen, h);
                    for e in elems.iter().skip(2) {
                        scan_value(e, env, uses, seen);
                    }
                } else {
                    // Ordinary call head: not a read.
                    for e in &elems[1..] {
                        scan_value(e, env, uses, seen);
                    }
                }
            } else {
                // Computed head (codegen rejects): descend for totality.
                for e in elems {
                    scan_value(e, env, uses, seen);
                }
            }
            if let Some(t) = tail {
                scan_value(t, env, uses, seen);
            }
        }
    }
}

/// Descend into a value-position `Proc` without treating it as a
/// definition (keeps the walk total over codegen-rejected shapes).
fn walk_proc_shallow(
    node: &Node,
    env: &LexEnv,
    uses: &mut Vec<String>,
    seen: &mut HashSet<String>,
) {
    if let NodeKind::Proc { params, body, .. } = &node.kind {
        let mut inner = env.vars.clone();
        let mut ps = HashSet::new();
        for (n, _) in params {
            ps.insert(n.clone());
        }
        inner.push(ps);
        let child_env = LexEnv {
            vars: inner,
            top_procs: env.top_procs,
            globals: env.globals,
            native: env.native,
        };
        for b in body {
            scan_garbage(b, &child_env, uses, seen);
        }
    }
}

/// Walk for codegen-rejected positions (bindings go nowhere).
fn scan_garbage(node: &Node, env: &LexEnv, uses: &mut Vec<String>, seen: &mut HashSet<String>) {
    match &node.kind {
        NodeKind::Var { val, .. } | NodeKind::Const { val, .. } | NodeKind::Set { val, .. } => {
            scan_value(val, env, uses, seen);
            if let NodeKind::Set { name, .. } = &node.kind {
                if let Some((head, _)) = split_dot(name) {
                    push_use(uses, seen, head);
                } else {
                    push_use(uses, seen, name);
                }
            }
        }
        NodeKind::Proc { .. } => walk_proc_shallow(node, env, uses, seen),
        _ => scan_value(node, env, uses, seen),
    }
}

/// Record call edges to lexically visible nested names in a value.
fn collect_calls(
    node: &Node,
    env: &LexEnv,
    nesteds: &HashSet<String>,
    out: &mut Vec<String>,
    seen: &mut HashSet<String>,
) {
    match &node.kind {
        NodeKind::List(elems, tail) => {
            if let Some(NodeKind::Sym(h)) = elems.first().map(|e| &e.kind) {
                if split_dot(h).is_none()
                    && !env.is_bound(h)
                    && !env.globals.contains(h.as_str())
                    && nesteds.contains(h.as_str())
                    && seen.insert(h.clone())
                {
                    out.push(h.clone());
                }
            }
            for e in elems {
                collect_calls(e, env, nesteds, out, seen);
            }
            if let Some(t) = tail {
                collect_calls(t, env, nesteds, out, seen);
            }
        }
        NodeKind::Var { val, .. } | NodeKind::Const { val, .. } | NodeKind::Set { val, .. } => {
            collect_calls(val, env, nesteds, out, seen);
        }
        NodeKind::Proc { .. } => {} // own scan handles nested bodies
        _ => {}
    }
}

/// Directly nested proc names in a body (this level + blocks, but never
/// inside nested proc bodies — function-wide visibility).
fn direct_child_names(body: &[Node]) -> HashSet<String> {
    let mut out = HashSet::new();
    fn deep(nodes: &[Node], out: &mut HashSet<String>) {
        for n in nodes {
            match &n.kind {
                NodeKind::Proc { name, .. } => {
                    out.insert(name.clone());
                }
                NodeKind::List(elems, _) => {
                    for e in elems {
                        deep(std::slice::from_ref(e), out);
                    }
                }
                NodeKind::Var { val, .. }
                | NodeKind::Const { val, .. }
                | NodeKind::Set { val, .. } => {
                    deep(std::slice::from_ref(val), out);
                }
                _ => {}
            }
        }
    }
    fn top(nodes: &[Node], out: &mut HashSet<String>) {
        for n in nodes {
            match &n.kind {
                NodeKind::Proc { name, .. } => {
                    out.insert(name.clone());
                }
                NodeKind::List(elems, _) => {
                    // Recurse into block structure but not proc bodies.
                    if let Some(NodeKind::Sym(h)) = elems.first().map(|e| &e.kind) {
                        if h == "if" || h == "while" || h == "for" {
                            for e in &elems[1..] {
                                top(std::slice::from_ref(e), out);
                            }
                            continue;
                        }
                    }
                    for e in elems {
                        deep(std::slice::from_ref(e), out);
                    }
                }
                NodeKind::Var { val, .. }
                | NodeKind::Const { val, .. }
                | NodeKind::Set { val, .. } => {
                    deep(std::slice::from_ref(val), out);
                }
                _ => {}
            }
        }
    }
    top(body, &mut out);
    out
}

struct Walker<'a> {
    env: LexEnv<'a>,
    nesteds: HashSet<String>,
    bindings: HashMap<String, bool>,
    uses: Vec<String>,
    seen: HashSet<String>,
    nested_calls: Vec<String>,
    nested_seen: HashSet<String>,
    children: Vec<ProcScan>,
    /// Ancestor names (excluding self) + self name, for child paths.
    path: Vec<String>,
    name: String,
}

impl<'a> Walker<'a> {
    fn walk_stmts(&mut self, body: &[Node]) {
        for node in body {
            self.walk_stmt(node);
        }
    }

    fn walk_stmt(&mut self, node: &Node) {
        match &node.kind {
            NodeKind::Var { name, val, .. } => {
                collect_calls(
                    val,
                    &self.env,
                    &self.nesteds,
                    &mut self.nested_calls,
                    &mut self.nested_seen,
                );
                scan_value(val, &self.env, &mut self.uses, &mut self.seen);
                self.bindings.insert(name.clone(), false);
                if let Some(s) = self.env.vars.last_mut() {
                    s.insert(name.clone());
                }
            }
            NodeKind::Const { name, val, .. } => {
                collect_calls(
                    val,
                    &self.env,
                    &self.nesteds,
                    &mut self.nested_calls,
                    &mut self.nested_seen,
                );
                scan_value(val, &self.env, &mut self.uses, &mut self.seen);
                self.bindings.insert(name.clone(), true);
                if let Some(s) = self.env.vars.last_mut() {
                    s.insert(name.clone());
                }
            }
            NodeKind::Set { name, val } => {
                collect_calls(
                    val,
                    &self.env,
                    &self.nesteds,
                    &mut self.nested_calls,
                    &mut self.nested_seen,
                );
                scan_value(val, &self.env, &mut self.uses, &mut self.seen);
                if let Some((head, _)) = split_dot(name) {
                    push_use(&mut self.uses, &mut self.seen, head);
                } else {
                    push_use(&mut self.uses, &mut self.seen, name);
                }
            }
            NodeKind::Proc {
                name, params, body, ..
            } => {
                let mut child_path = self.path.clone();
                child_path.push(self.name.clone());
                let ps: Vec<(String, bool)> =
                    params.iter().map(|(n, _)| (n.clone(), false)).collect();
                let mut child_nests = self.nesteds.clone();
                child_nests.insert(name.clone());
                let child = scan_proc(
                    ScanHead {
                        path: child_path,
                        name: name.clone(),
                        params: ps,
                        body,
                        line: node.span.line,
                        col: node.span.col,
                        parent_nests: child_nests,
                    },
                    &self.env,
                );
                self.children.push(child);
            }
            NodeKind::List(elems, _) if !elems.is_empty() => {
                if let NodeKind::Sym(h) = &elems[0].kind {
                    match h.as_str() {
                        "if" => {
                            self.walk_if(elems);
                            return;
                        }
                        "while" => {
                            if elems.len() > 1 {
                                collect_calls(
                                    &elems[1],
                                    &self.env,
                                    &self.nesteds,
                                    &mut self.nested_calls,
                                    &mut self.nested_seen,
                                );
                                scan_value(&elems[1], &self.env, &mut self.uses, &mut self.seen);
                            }
                            self.env.vars.push(HashSet::new());
                            for b in elems.iter().skip(2) {
                                self.walk_stmt(b);
                            }
                            self.env.vars.pop();
                            return;
                        }
                        "for" => {
                            // Codegen order: init(1), test(2), body(4..), update(3).
                            self.env.vars.push(HashSet::new());
                            if elems.len() > 1 {
                                self.walk_stmt(&elems[1]);
                            }
                            if elems.len() > 2 {
                                collect_calls(
                                    &elems[2],
                                    &self.env,
                                    &self.nesteds,
                                    &mut self.nested_calls,
                                    &mut self.nested_seen,
                                );
                                scan_value(&elems[2], &self.env, &mut self.uses, &mut self.seen);
                            }
                            self.env.vars.push(HashSet::new());
                            if elems.len() > 4 {
                                for b in elems.iter().skip(4) {
                                    self.walk_stmt(b);
                                }
                            }
                            self.env.vars.pop();
                            if elems.len() > 3 {
                                self.walk_stmt(&elems[3]);
                            }
                            self.env.vars.pop();
                            return;
                        }
                        _ => {}
                    }
                }
                collect_calls(
                    node,
                    &self.env,
                    &self.nesteds,
                    &mut self.nested_calls,
                    &mut self.nested_seen,
                );
                scan_value(node, &self.env, &mut self.uses, &mut self.seen);
            }
            _ => {
                collect_calls(
                    node,
                    &self.env,
                    &self.nesteds,
                    &mut self.nested_calls,
                    &mut self.nested_seen,
                );
                scan_value(node, &self.env, &mut self.uses, &mut self.seen);
            }
        }
    }

    fn walk_if(&mut self, elems: &[Node]) {
        // elems[0] == `if`; elems[1] == test; rest are branch bodies
        // separated by `elif test` / `else` markers (same split as codegen).
        if elems.len() > 1 {
            collect_calls(
                &elems[1],
                &self.env,
                &self.nesteds,
                &mut self.nested_calls,
                &mut self.nested_seen,
            );
            scan_value(&elems[1], &self.env, &mut self.uses, &mut self.seen);
        }
        let mut k = 2;
        let mut body: Vec<&Node> = Vec::new();
        let mut groups: Vec<(Option<Node>, Vec<&Node>)> = Vec::new();
        while k < elems.len() {
            match &elems[k].kind {
                NodeKind::Sym(s) if s == "elif" || s == "else" => {
                    let test = if s == "else" {
                        None
                    } else {
                        k += 1;
                        elems.get(k).cloned()
                    };
                    groups.push((test, std::mem::take(&mut body)));
                }
                _ => body.push(&elems[k]),
            }
            k += 1;
        }
        groups.push((None, std::mem::take(&mut body)));
        for (i, (test, bodies)) in groups.into_iter().enumerate() {
            if i > 0 {
                if let Some(t) = test {
                    collect_calls(
                        &t,
                        &self.env,
                        &self.nesteds,
                        &mut self.nested_calls,
                        &mut self.nested_seen,
                    );
                    scan_value(&t, &self.env, &mut self.uses, &mut self.seen);
                }
            }
            self.env.vars.push(HashSet::new());
            for b in bodies {
                self.walk_stmt(b);
            }
            self.env.vars.pop();
        }
    }
}

/// Inputs for scanning one procedure body.
struct ScanHead<'b> {
    path: Vec<String>,
    name: String,
    params: Vec<(String, bool)>,
    body: &'b [Node],
    line: usize,
    col: usize,
    parent_nests: HashSet<String>,
}

/// Scan one procedure body: statements in order, blocks scoped.
fn scan_proc(head: ScanHead, parent: &LexEnv) -> ProcScan {
    let ScanHead {
        path,
        name,
        params,
        body,
        line,
        col,
        parent_nests,
    } = head;
    let mut w = Walker {
        env: LexEnv {
            vars: parent.vars.clone(),
            top_procs: parent.top_procs,
            globals: parent.globals,
            native: parent.native,
        },
        nesteds: parent_nests,
        bindings: HashMap::new(),
        uses: Vec::new(),
        seen: HashSet::new(),
        nested_calls: Vec::new(),
        nested_seen: HashSet::new(),
        children: Vec::new(),
        path: path.clone(),
        name: name.clone(),
    };
    for (n, c) in &params {
        w.bindings.insert(n.clone(), *c);
    }
    {
        // Nested names visible in this body: parent's nesteds plus our
        // own direct children (sequential in codegen; over-approximated
        // here — codegen enforces the order).
        w.nesteds.extend(direct_child_names(body));
        let mut ps = HashSet::new();
        for (n, _) in &params {
            ps.insert(n.clone());
        }
        w.env.vars.push(ps);
    }
    w.walk_stmts(body);
    let descendants: HashSet<String> = w
        .children
        .iter()
        .flat_map(|c| {
            let mut v = vec![c.name.clone()];
            v.extend(c.descendants.iter().cloned());
            v
        })
        .collect();
    ProcScan {
        path,
        name,
        params: params.into_iter().map(|(n, _)| n).collect(),
        bindings: w.bindings,
        uses: w.uses,
        nested_calls: w.nested_calls,
        children: w.children,
        parent_nests: w.nesteds,
        descendants,
        caps: Vec::new(),
        line,
        col,
    }
}

/// Post-order + fixpoint capture computation over the scan forest.
fn compute_caps(roots: &mut [ProcScan], env: &LexEnv) {
    fn seed(s: &mut ProcScan, env: &LexEnv) {
        for c in &mut s.children {
            seed(c, env);
        }
        let mut ordered = Vec::new();
        let mut seen = HashSet::new();
        for u in &s.uses {
            if s.params.contains(u) {
                continue;
            }
            if s.bindings.contains_key(u) {
                continue;
            }
            if s.descendants.contains(u) || s.parent_nests.contains(u) {
                continue;
            }
            if is_globalish(env, u) {
                continue;
            }
            if seen.insert(u.clone()) {
                ordered.push((u.clone(), false));
            }
        }
        s.caps = ordered;
    }
    for r in roots.iter_mut() {
        seed(r, env);
    }
    // Name index over every scan (roots included).
    loop {
        let snap = snapshot(roots);
        let mut changed = false;
        for r in roots.iter_mut() {
            changed |= propagate(r, &snap, env);
        }
        if !changed {
            break;
        }
    }
    for r in roots.iter_mut() {
        let chain: Vec<(Vec<String>, HashMap<String, bool>)> = Vec::new();
        resolve_const(r, &chain);
    }
}

type Snap = HashMap<String, (Vec<String>, Vec<(String, bool)>)>;

fn snap_key(path: &[String], name: &str, line: usize, col: usize) -> String {
    nest_key(path, name, line, col)
}

fn snapshot(roots: &[ProcScan]) -> Snap {
    let mut m = HashMap::new();
    fn walk(s: &ProcScan, m: &mut Snap) {
        // Full ancestor path of s (path + self name).
        let mut full = s.path.clone();
        full.push(s.name.clone());
        m.insert(
            snap_key(&s.path, &s.name, s.line, s.col),
            (full, s.caps.clone()),
        );
        for c in &s.children {
            walk(c, m);
        }
    }
    // Roots (top procs, @main) carry no callable caps themselves; only
    // nested definitions participate in propagation.
    for r in roots {
        for c in &r.children {
            walk(c, &mut m);
        }
    }
    m
}

/// `t` is lexically visible from `s` iff T's defining body is s's own
/// body or an ancestor's body: T.path must be a prefix of s's full path
/// (sequential order ignored — codegen enforces it; over-approximation
/// here only adds params that the def-time filter may drop).
fn visible_from(t_path: &[String], s_full: &[String]) -> bool {
    if t_path.len() > s_full.len() {
        return false;
    }
    t_path == &s_full[..t_path.len()]
}

fn propagate(s: &mut ProcScan, snap: &Snap, env: &LexEnv) -> bool {
    let mut changed = false;
    for c in &mut s.children {
        changed |= propagate(c, snap, env);
    }
    let mut s_full = s.path.clone();
    s_full.push(s.name.clone());
    // Edges: children (defined-in-X forwarding) + nested calls.
    let mut targets: Vec<String> = Vec::new();
    let mut seen_t = HashSet::new();
    for c in &s.children {
        let k = snap_key(&c.path, &c.name, c.line, c.col);
        if seen_t.insert(k.clone()) {
            targets.push(k);
        }
    }
    // Nested calls: resolve each callee name against visible defs.
    for callee in &s.nested_calls {
        // Candidate keys: every scan named `callee` visible from s.
        // Reconstruct from snapshot paths.
        let mut cands: Vec<String> = snap
            .iter()
            .filter(|(_, (full, _))| {
                full.last().map(|n| n == callee).unwrap_or(false)
                    && visible_from(&full[..full.len() - 1], &s_full)
            })
            .map(|(k, _)| k.clone())
            .collect();
        cands.sort();
        for k in cands {
            if seen_t.insert(k.clone()) {
                targets.push(k);
            }
        }
    }
    // Self-recursion: own caps already present; nothing to add.
    let bound: HashSet<String> = s
        .bindings
        .keys()
        .cloned()
        .chain(s.params.iter().cloned())
        .chain(s.descendants.iter().cloned())
        .chain(s.parent_nests.iter().cloned())
        .collect();
    for key in targets {
        if let Some((_, caps)) = snap.get(&key) {
            for (n, _) in caps {
                if bound.contains(n) || is_globalish(env, n) {
                    continue;
                }
                if !s.caps.iter().any(|(e, _)| e == n) {
                    s.caps.push((n.clone(), false));
                    changed = true;
                }
            }
        }
    }
    changed
}

/// Fill `is_const` from the nearest binder (params are mutable).
fn resolve_const(s: &mut ProcScan, ancestors: &[(Vec<String>, HashMap<String, bool>)]) {
    for (n, flag) in s.caps.iter_mut() {
        let mut found = None;
        if s.params.contains(n) {
            found = Some(false);
        } else if let Some(c) = s.bindings.get(n) {
            found = Some(*c);
        } else {
            for (ps, bs) in ancestors.iter().rev() {
                if ps.contains(n) {
                    found = Some(false);
                    break;
                } else if let Some(c) = bs.get(n) {
                    found = Some(*c);
                    break;
                }
            }
        }
        // Unresolvable here (global/typo/error-path): codegen's def-time
        // filter drops it and the natural error fires in the body.
        *flag = found.unwrap_or(false);
    }
    let mut next = ancestors.to_vec();
    next.push((s.params.clone(), s.bindings.clone()));
    for c in &mut s.children {
        resolve_const(c, &next);
    }
}

/// Analyze a whole program.
///
/// `top_bodies` carries top-level procs in program order plus main-flow
/// statements as `@main` (params empty). `main_own` are locals bound in
/// the top-level flow — folded into `@main`'s bindings by the walk.
pub fn analyze(
    items: &[TopItem],
    top_procs: &HashSet<String>,
    globals: &HashSet<String>,
    native: &NativeDecls,
) -> ClosureMaps {
    let root_env = LexEnv {
        vars: vec![HashSet::new()],
        top_procs,
        globals,
        native,
    };
    let mut roots: Vec<ProcScan> = Vec::new();
    for (name, params, body) in items {
        let mut nests = HashSet::new();
        if name == "@main" {
            nests.extend(direct_child_names(body));
        }
        roots.push(scan_proc(
            ScanHead {
                path: Vec::new(),
                name: name.clone(),
                params: params.clone(),
                body,
                line: usize::MAX,
                col: usize::MAX,
                parent_nests: nests,
            },
            &root_env,
        ));
    }
    compute_caps(&mut roots, &root_env);
    let mut caps = HashMap::new();
    fn emit(s: &ProcScan, caps: &mut HashMap<String, Vec<(String, bool)>>) {
        for c in &s.children {
            caps.insert(snap_key(&c.path, &c.name, c.line, c.col), c.caps.clone());
            emit(c, caps);
        }
    }
    for r in &roots {
        emit(r, &mut caps);
    }
    ClosureMaps { caps }
}
