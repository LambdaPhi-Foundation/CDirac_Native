//! N1 gate: full-core builtins through `compile_llvm` (IR shape +
//! static-error surface). Runtime value semantics live in
//! `llvmrt/tests/test_n1.c`; byte-exact behavior vs the reference
//! tree-walker is `tools/llvmcheck.py`.

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
fn arith_trio_and_small_forms_emit() {
    let ll = llvm("displayln(quotient(7, 2));\ndisplayln(remainder(-7, 3));\ndisplayln(modulo(7, -3));\ndisplayln(add1(1));\ndisplayln(sub1(1.5));\ndisplayln(abs(0 - 3));\ndisplayln(max(1, 2, 3));\ndisplayln(min(1));\ndisplayln(expt(2, 10));\ndisplayln(sqrt(9));\n");
    for sym in [
        "cdi_quotient",
        "cdi_remainder",
        "cdi_modulo",
        "cdi_add1",
        "cdi_sub1",
        "cdi_abs",
        "cdi_max_n",
        "cdi_min_n",
        "cdi_expt",
        "cdi_sqrt",
    ] {
        assert!(
            ll.contains(&format!("declare ptr @{sym}")),
            "missing declare {sym}"
        );
        assert!(
            ll.contains(&format!("call ptr @{sym}")),
            "missing call {sym}"
        );
    }
    // Arity is static: fixed-2 rejects 1.
    let e = llvm_err("displayln(quotient(1));\n");
    assert!(
        e.contains("quotient: expects 2 argument(s), got 1"),
        "arity: {e}"
    );
    let e = llvm_err("displayln(sqrt(1, 2));\n");
    assert!(e.contains("sqrt: expects 1 argument, got 2"), "arity: {e}");
}

#[test]
fn predicates_emit() {
    let ll = llvm("displayln(zero?(0));\ndisplayln(positive?(1));\ndisplayln(negative?(0 - 1));\ndisplayln(even?(2));\ndisplayln(odd?(3));\ndisplayln(equal?(1, 1));\ndisplayln(number?(1));\ndisplayln(real?(1.5));\ndisplayln(integer?(2));\ndisplayln(boolean?(zero?(0)));\ndisplayln(string?(\"s\"));\ndisplayln(array?(array(1)));\ndisplayln(queue?(queue(1)));\ndisplayln(empty?(\"\"));\ndisplayln(void?(void()));\n");
    for sym in [
        "cdi_zero_p",
        "cdi_positive_p",
        "cdi_negative_p",
        "cdi_even_p",
        "cdi_odd_p",
        "cdi_equal_p",
        "cdi_number_p",
        "cdi_real_p",
        "cdi_integer_p",
        "cdi_boolean_p",
        "cdi_string_p",
        "cdi_array_p",
        "cdi_queue_p",
        "cdi_empty_p",
        "cdi_void_p",
    ] {
        assert!(
            ll.contains(&format!("call ptr @{sym}")),
            "missing call {sym}:\n{ll}"
        );
    }
}

#[test]
fn containers_emit() {
    let ll = llvm("var a = array(1, 2);\nvar q = queue(1);\nput(a, 0, 9);\npush(a, 3);\ndisplayln(pop(a));\nqpush(q, 2);\nqpush-front(q, 0);\ndisplayln(qpop(q));\ndisplayln(qpop-back(q));\ndisplayln(qpeek(q));\ndisplayln(qpeek-back(q));\n");
    for sym in [
        "cdi_put",
        "cdi_push",
        "cdi_pop",
        "cdi_queue_n",
        "cdi_qpush",
        "cdi_qpush_front",
        "cdi_qpop",
        "cdi_qpop_back",
        "cdi_qpeek",
        "cdi_qpeek_back",
    ] {
        assert!(
            ll.contains(&format!("call ptr @{sym}")),
            "missing call {sym}:\n{ll}"
        );
    }
    let e = llvm_err("var a = array(1);\nput(a, 0);\n");
    assert!(
        e.contains("put: expects 3 argument(s), got 2"),
        "arity: {e}"
    );
}

#[test]
fn strings_emit() {
    let ll = llvm("displayln(string-append(\"a\", \"b\"));\ndisplayln(string-length(\"hi\"));\ndisplayln(substring(\"hello\", 1, 3));\ndisplayln(substring(\"hello\", 2));\ndisplayln(string-upcase(\"hi\"));\ndisplayln(string-downcase(\"HI\"));\ndisplayln(number->string(42));\ndisplayln(string->number(\"42\"));\n");
    for sym in [
        "cdi_string_append_n",
        "cdi_string_length",
        "cdi_substring",
        "cdi_string_upcase",
        "cdi_string_downcase",
        "cdi_number_to_string",
        "cdi_string_to_number",
    ] {
        assert!(
            ll.contains(&format!("call ptr @{sym}")),
            "missing call {sym}:\n{ll}"
        );
    }
    // substring takes 2..3 with an i1 presence flag.
    assert!(ll.contains("cdi_substring(ptr"), "substring shape:\n{ll}");
    assert!(
        ll.contains("i1 true") && ll.contains("i1 false"),
        "presence flag:\n{ll}"
    );
    let e = llvm_err("displayln(substring(\"hi\", 1, 2, 3));\n");
    assert!(
        e.contains("substring: expects 2..3 argument(s), got 4"),
        "arity: {e}"
    );
}

#[test]
fn effects_emit() {
    let ll = llvm("print(\"hi\");\nnewline();\nvar s = read-line();\ndisplayln(s);\nvoid();\n");
    for sym in ["cdi_print", "cdi_newline", "cdi_read_line", "cdi_void"] {
        assert!(
            ll.contains(&format!("call ptr @{sym}")),
            "missing call {sym}:\n{ll}"
        );
    }
    let e = llvm_err("void(1);\n");
    assert!(
        e.contains("void: expects 0 argument(s), got 1"),
        "arity: {e}"
    );
}

#[test]
fn and_or_short_circuit() {
    // Binary: test + branch + join with phi; skipped sides never emit.
    // Short edges carry the decisive operand (or yields first truthy).
    let ll = llvm("displayln(1 == 1 && 2 == 2);\ndisplayln(1 == 2 || 2 == 2);\n");
    assert!(ll.contains("cdi_truthy"), "tests:\n{ll}");
    assert!(ll.contains("phi ptr"), "join:\n{ll}");
    assert!(ll.contains("logicshort"), "short edges:\n{ll}");
    // Empty forms are constants.
    let ll = llvm("displayln(1 == 1 && 2 == 2 && 3 == 3);\n");
    assert!(ll.contains("logicfall"), "chain:\n{ll}");
    // or carries the truthy operand, not false.
    let ll = llvm("displayln(0 || 7);\n");
    assert!(ll.matches("logicshort").count() >= 1, "or short:\n{ll}");
    // Machine rejects and/or (reference: write && / || instead).
    let e = llvm_err("machine.and(1, 2);\n");
    assert!(
        e.contains("unknown machine operation 'and'"),
        "machine and: {e}"
    );
}

#[test]
fn value_if_joins_with_phi() {
    // SDirac spelling in value position (CDirac `if` is a statement).
    let ll = llvm("var x = [if [== 1 1] 10 else 20];\ndisplayln(x);\n");
    assert!(ll.contains("phi ptr"), "join:\n{ll}");
    assert!(ll.contains("cdi_truthy"), "test:\n{ll}");
    // Missing else yields void.
    let ll = llvm("var y = [if [== 1 2] 10];\ndisplayln(y);\n");
    assert!(ll.contains("cdi_void"), "void default:\n{ll}");
    // Statement prefixes are rejected like the reference M3 rule.
    let e = llvm_err("var z = [if [== 1 1] [set z 2] else 3];\ndisplayln(z);\n");
    assert!(e.contains("value-if with statement prefix"), "prefix: {e}");
}

#[test]
fn every_builtin_dispatches_to_its_runtime_symbol() {
    // Table-driven guard: each builtin must lower to its own runtime
    // call (a missing match arm would silently take the n-ary shape).
    // and/or/if lower to control flow (covered above); machine-only
    // alloc/dealloc are covered in machine_memory_ops.
    let cases: &[(&str, &str)] = &[
        ("1 + 2", "cdi_add_n"),
        ("3 - 1", "cdi_sub_n"),
        ("2 * 3", "cdi_mul_n"),
        ("7 / 2", "cdi_div_n"),
        ("1 == 1", "cdi_eq_n"),
        ("1 < 2", "cdi_lt_n"),
        ("2 > 1", "cdi_gt_n"),
        ("1 <= 1", "cdi_le_n"),
        ("2 >= 1", "cdi_ge_n"),
        ("!(1 == 2)", "cdi_not"),
        ("display(1)", "cdi_display"),
        ("displayln(1)", "cdi_displayln"),
        ("print(1)", "cdi_print"),
        ("array(1)", "cdi_array_n"),
        ("len(\"hi\")", "cdi_len"),
        ("get(array(1), 0)", "cdi_get"),
        ("quotient(7, 2)", "cdi_quotient"),
        ("remainder(7, 2)", "cdi_remainder"),
        ("modulo(7, 2)", "cdi_modulo"),
        ("add1(1)", "cdi_add1"),
        ("sub1(1)", "cdi_sub1"),
        ("abs(0 - 1)", "cdi_abs"),
        ("max(1, 2)", "cdi_max_n"),
        ("min(1, 2)", "cdi_min_n"),
        ("expt(2, 3)", "cdi_expt"),
        ("sqrt(4)", "cdi_sqrt"),
        ("zero?(0)", "cdi_zero_p"),
        ("positive?(1)", "cdi_positive_p"),
        ("negative?(0 - 1)", "cdi_negative_p"),
        ("even?(2)", "cdi_even_p"),
        ("odd?(1)", "cdi_odd_p"),
        ("equal?(1, 1)", "cdi_equal_p"),
        ("number?(1)", "cdi_number_p"),
        ("real?(1)", "cdi_real_p"),
        ("integer?(1)", "cdi_integer_p"),
        ("boolean?(zero?(0))", "cdi_boolean_p"),
        ("string?(\"s\")", "cdi_string_p"),
        ("array?(array(1))", "cdi_array_p"),
        ("queue?(queue(1))", "cdi_queue_p"),
        ("empty?(\"x\")", "cdi_empty_p"),
        ("void?(void())", "cdi_void_p"),
        ("put(array(1, 2), 0, 9)", "cdi_put"),
        ("push(array(1), 2)", "cdi_push"),
        ("pop(array(1))", "cdi_pop"),
        ("queue(1)", "cdi_queue_n"),
        ("qpush(queue(1), 2)", "cdi_qpush"),
        ("qpush-front(queue(1), 0)", "cdi_qpush_front"),
        ("qpop(queue(1))", "cdi_qpop"),
        ("qpop-back(queue(1))", "cdi_qpop_back"),
        ("qpeek(queue(1))", "cdi_qpeek"),
        ("qpeek-back(queue(1))", "cdi_qpeek_back"),
        ("string-append(\"a\")", "cdi_string_append_n"),
        ("string-length(\"hi\")", "cdi_string_length"),
        ("substring(\"hello\", 0, 2)", "cdi_substring"),
        ("string-upcase(\"hi\")", "cdi_string_upcase"),
        ("string-downcase(\"HI\")", "cdi_string_downcase"),
        ("number->string(7)", "cdi_number_to_string"),
        ("string->number(\"7\")", "cdi_string_to_number"),
        ("newline()", "cdi_newline"),
        ("read-line()", "cdi_read_line"),
        ("error(\"x\")", "cdi_error_n"),
        ("void()", "cdi_void"),
    ];
    for (expr, sym) in cases {
        let ll = llvm(&format!("displayln({expr});\n"));
        assert!(
            ll.contains(&format!("call ptr @{sym}(")),
            "{expr} must call {sym}:\n{ll}"
        );
    }
}

#[test]
fn machine_memory_ops() {
    let ll = llvm("var h = machine.alloc(16);\nmachine.dealloc(h);\n");
    assert!(ll.contains("call ptr @cdi_machine_alloc"), "alloc:\n{ll}");
    assert!(
        ll.contains("call ptr @cdi_machine_dealloc"),
        "dealloc:\n{ll}"
    );
    // Bare alloc stays unbound (machine-only by design).
    let e = llvm_err("var h = alloc(16);\n");
    assert!(e.contains("unbound identifier: alloc"), "bare alloc: {e}");
    // Unknown machine ops still fail closed.
    let e = llvm_err("machine.frobnicate(1);\n");
    assert!(
        e.contains("unknown machine operation 'frobnicate'"),
        "closed: {e}"
    );
}

#[test]
fn error_aborts_through_runtime() {
    let ll = llvm("error(\"boom\");\n");
    assert!(ll.contains("call ptr @cdi_error_n"), "error call:\n{ll}");
    // error() with no args is the bare "error" message path.
    let ll = llvm("if (1 == 2) { error(); }\ndisplayln(1);\n");
    assert!(
        ll.contains("call ptr @cdi_error_n(ptr null, i32 0)"),
        "nullary:\n{ll}"
    );
}
