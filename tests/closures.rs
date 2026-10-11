//! Closures gate: nested `fn` lifts to top-level functions with hidden
//! by-reference capture params (`llvmgen` + `closures::analyze`).
//! Execution semantics are byte-gated on CI (`llvmcheck` needs clang);
//! here we gate IR shape + the static-error surface.

use cdirac::llvmgen::compile_llvm;
use cdirac::parser::parse_source;

fn prog(src: &str) -> Vec<cdirac::ast::Node> {
    parse_source(src).expect("parse")
}

fn llvm(src: &str) -> String {
    let p = prog(src);
    compile_llvm(&p).expect("llvm")
}

fn llvm_err(src: &str) -> String {
    let p = prog(src);
    compile_llvm(&p).expect_err("expected llvm error")
}

#[test]
fn basic_capture_lifts_with_cap_param() {
    let ll = llvm("fn outer(n|int) | int {\nvar m = n + 1;\nfn helper(x|int) | int {\nreturn x + m;\n}\nreturn helper(n * 2);\n}\ndisplayln(outer(10));\n");
    // Lifted above main with the capture first.
    assert!(
        ll.contains("define ptr @cdi_proc_outer_helper_"),
        "lifted:\n{ll}"
    );
    assert!(ll.contains("(ptr %cap0, ptr %p0)"), "cap-first sig:\n{ll}");
    // Reads go through the slot address (by reference).
    assert!(ll.contains("load ptr, ptr %cap0"), "cap load:\n{ll}");
    // Call passes the slot address (m's alloca), not the value.
    assert!(
        ll.contains("call ptr @cdi_proc_outer_helper_"),
        "call:\n{ll}"
    );
}

#[test]
fn no_capture_means_plain_signature() {
    let ll = llvm("fn outer() | int {\nfn helper(x|int) | int {\nreturn x + 1;\n}\nreturn helper(41);\n}\ndisplayln(outer());\n");
    assert!(ll.contains("(ptr %p0)"), "no cap params:\n{ll}");
    assert!(!ll.contains("%cap"), "no caps at all:\n{ll}");
}

#[test]
fn nested_recursion_passes_env_through() {
    let ll = llvm("fn outer(n|int) | int {\nvar step = 1;\nfn walk(k|int) | int {\nif (k <= 0) { return step; }\nreturn walk(k - 1);\n}\nreturn walk(n);\n}\ndisplayln(outer(5));\n");
    // Self-call forwards the received capture (no rebuild).
    assert!(
        ll.contains("define ptr @cdi_proc_outer_walk_"),
        "lifted:\n{ll}"
    );
    assert!(ll.contains("(ptr %cap0, ptr %p0)"), "sig:\n{ll}");
    let calls: Vec<&str> = ll.matches("call ptr @cdi_proc_outer_walk_").collect();
    assert!(calls.len() >= 2, "outer + self calls:\n{ll}");
}

#[test]
fn set_writes_through_the_capture() {
    let ll = llvm("var total|int = 0;\nfn outer() {\nfn bump(x|int) {\nset total = total + x;\n}\nbump(3);\nbump(4);\n}\nouter();\ndisplayln(total);\n");
    // Globals need no capture (direct access).
    assert!(!ll.contains("%cap"), "global access is direct:\n{ll}");
    // ...but the same shape with a *local* accumulator stores via cap.
    let ll = llvm("fn outer() | int {\nvar total = 0;\nfn bump(x|int) {\nset total = total + x;\n}\nbump(3);\nbump(4);\nreturn total;\n}\ndisplayln(outer());\n");
    assert!(ll.contains("store ptr"), "stores exist:\n{ll}");
    assert!(ll.contains("ptr %cap0"), "cap param:\n{ll}");
}

#[test]
fn deep_nesting_forwards_through_middlemen() {
    let src = "fn outer() | int {\nvar x = 7;\nfn mid() | int {\nfn leaf() | int {\nreturn x + 1;\n}\nreturn leaf();\n}\nreturn mid();\n}\ndisplayln(outer());\n";
    let ll = llvm(src);
    // leaf captures x; mid forwards it without using it.
    assert!(
        ll.contains("define ptr @cdi_proc_outer_mid_leaf_"),
        "leaf:\n{ll}"
    );
    assert!(ll.contains("(ptr %cap0)"), "nullary + cap:\n{ll}");
}

#[test]
fn sibling_calls_share_the_frame() {
    let src = "fn outer(v|int) | int {\nvar k = 2;\nfn double(x|int) | int {\nreturn x * k;\n}\nfn quad(x|int) | int {\nreturn double(double(x));\n}\nreturn quad(v);\n}\ndisplayln(outer(3));\n";
    let ll = llvm(src);
    assert!(
        ll.contains("define ptr @cdi_proc_outer_double_"),
        "double:\n{ll}"
    );
    assert!(
        ll.contains("define ptr @cdi_proc_outer_quad_"),
        "quad:\n{ll}"
    );
}

#[test]
fn nested_shadows_top_level_proc() {
    let src = "fn f(x|int) | int {\nreturn x + 1;\n}\nfn outer(v|int) | int {\nfn f(x|int) | int {\nreturn x * 10;\n}\nreturn f(v);\n}\ndisplayln(f(1));\ndisplayln(outer(1));\n";
    let ll = llvm(src);
    assert!(ll.contains("define ptr @cdi_proc_f_"), "top:\n{ll}");
    assert!(
        ll.contains("define ptr @cdi_proc_outer_f_"),
        "nested:\n{ll}"
    );
}

#[test]
fn const_captures_stay_const() {
    let src = "fn outer() | int {\nconst k = 5;\nfn get() | int {\nreturn k;\n}\nreturn get();\n}\ndisplayln(outer());\n";
    let ll = llvm(src);
    assert!(ll.contains("(ptr %cap0)"), "cap:\n{ll}");
    let e = llvm_err("fn outer() | int {\nconst k = 5;\nfn bad() {\nset k = 6;\n}\nbad();\nreturn k;\n}\ndisplayln(outer());\n");
    assert!(e.contains("cannot assign to constant 'k'"), "const: {e}");
}

#[test]
fn error_surface() {
    // Duplicate in the same frame.
    let e = llvm_err("fn outer() {\nfn q() {\nreturn 1;\n}\nfn q() {\nreturn 2;\n}\nreturn q();\n}\ndisplayln(outer());\n");
    assert!(e.contains("duplicate procedure 'q'"), "dup: {e}");
    // Local/proc clash both ways.
    let e = llvm_err(
        "fn outer() {\nvar q = 1;\nfn q() {\nreturn 2;\n}\nreturn q();\n}\ndisplayln(outer());\n",
    );
    assert!(
        e.contains("cannot redefine 'q' as a procedure"),
        "clash: {e}"
    );
    let e = llvm_err(
        "fn outer() {\nfn q() {\nreturn 1;\n}\nvar q = 2;\nreturn q;\n}\ndisplayln(outer());\n",
    );
    assert!(
        e.contains("cannot redefine procedure 'q' as a variable"),
        "clash: {e}"
    );
    // Procedural name is not a value.
    let e = llvm_err("fn outer() {\nfn q() | int {\nreturn 1;\n}\nvar f = q;\nreturn f;\n}\ndisplayln(outer());\n");
    assert!(e.contains("'q' is a procedure, not a value"), "value: {e}");
    // Call before def fails like the top-level incremental rule.
    let e = llvm_err(
        "fn outer() {\nreturn q();\nfn q() | int {\nreturn 1;\n}\n}\ndisplayln(outer());\n",
    );
    assert!(e.contains("unbound identifier: q"), "order: {e}");
    // Arity still checked on declared params.
    let e = llvm_err("fn outer() {\nfn q(a|int) | int {\nreturn a;\n}\nreturn q(1, 2);\n}\ndisplayln(outer());\n");
    assert!(
        e.contains("arity mismatch in call to q: expects 1, got 2"),
        "arity: {e}"
    );
    // break inside a nested proc is outside any loop.
    let e = llvm_err(
        "fn outer() {\nwhile (true) {\nfn q() {\nbreak;\n}\nq();\nbreak;\n}\n}\nouter();\n",
    );
    assert!(e.contains("break outside a loop"), "break: {e}");
    // Capture of a block-exited local fails gracefully at the call
    // (defining without calling is harmless: no code runs).
    let ll = llvm("if (true) {\nvar t = 1;\nfn q() | int {\nreturn t;\n}\n}\ndisplayln(1);\n");
    assert!(ll.contains("define ptr @cdi_proc_main_q_"), "lifted:\n{ll}");
    let e = llvm_err("if (true) {\nvar t = 1;\nfn q() | int {\nreturn t;\n}\n}\ndisplayln(q());\n");
    assert!(
        e.contains("cannot capture 't' here (out of scope)"),
        "scope: {e}"
    );
    // Typo inside the nested body keeps the natural error.
    let e = llvm_err("fn outer() {\nvar ok = 1;\nfn q() | int {\nreturn ok + nope;\n}\nreturn q();\n}\ndisplayln(outer());\n");
    assert!(e.contains("unbound identifier: nope"), "typo: {e}");
    // Mutual recursion needs forward declaration (incremental rule):
    // the earlier sibling cannot see the later one.
    let e = llvm_err("fn outer() {\nfn a() | int {\nreturn b();\n}\nfn b() | int {\nreturn a();\n}\nreturn a();\n}\ndisplayln(outer());\n");
    assert!(e.contains("unbound identifier: b"), "mutual: {e}");
}
