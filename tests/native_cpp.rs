//! Native interop gate: C++-first headers as first-class Dirac.
//!
//! Covers directives/externs/classes through `compile_llvm` (IR shape)
//! and `compile_cpp_shim` (generated C++ shape), plus the static-error
//! surface (collisions, arity, unknown/ambiguous members, top-level).

use cdirac::llvmgen::{compile_cpp_shim, compile_llvm};
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

fn shim(src: &str) -> String {
    let p = prog(src);
    compile_cpp_shim(&p).expect("shim")
}

const PRELUDE: &str = r#"
[include-cpp "accum.h"]
[extern-cpp add2 "ns::add" [int int] int]
[cpp-class Accum "ns::Accum" "accum.h"]
[cpp-ctor Accum [int]]
[cpp-method Accum push [int] void]
[cpp-method Accum total [] int]
[cpp-static Accum make [int] Accum]
[cpp-field Accum size int]
[extern-cpp shout "ns::shout" [string] string]
"#;

#[test]
fn directives_emit_no_code_but_declare_wrappers() {
    let ll = llvm(PRELUDE);
    // 2 externs + 1 ctor + 2 methods + 1 static +
    // 1 get + 1 set + 1 delete = 9 wrappers.
    for i in 0..9 {
        assert!(
            ll.contains(&format!("declare ptr @cdi_wrap_{i}(")),
            "missing declare {i}:\n{ll}"
        );
    }
    assert!(
        ll.contains("clang++"),
        "header must name the clang++ link line"
    );
    assert!(
        ll.contains("cdi_shim.cpp"),
        "header must name the shim file"
    );
}

#[test]
fn shim_carries_headers_and_calls() {
    let s = shim(PRELUDE);
    assert!(s.contains("#include \"accum.h\""), "headers verbatim:\n{s}");
    assert!(s.contains("extern \"C\""), "stable C linkage:\n{s}");
    assert!(s.contains("ns::add"), "qualified free function:\n{s}");
    assert!(
        s.contains("new ns::Accum"),
        "ctor uses qualified name:\n{s}"
    );
    assert!(s.contains("self->push"), "instance method:\n{s}");
    assert!(s.contains("ns::Accum::make"), "static method:\n{s}");
    assert!(s.contains("self->size"), "field access:\n{s}");
    assert!(s.contains("delete self"), "delete wrapper:\n{s}");
    assert!(
        s.contains("#include \"cinterop.h\""),
        "conversion helpers:\n{s}"
    );
    // String unboxing must be sequenced statements: reading the length
    // in the same expression that sets it is an evaluation-order bug
    // (shout returned "" end-to-end before this was sequenced).
    assert!(s.contains("cdi_str_bytes(a"), "string unbox:\n{s}");
    assert!(
        !s.contains("std::string p0(cdi_str_bytes"),
        "unsequenced unbox:\n{s}"
    );
}

#[test]
fn calls_lower_to_wrapper_calls() {
    let src = format!(
        "{PRELUDE}\nvar a|ptr = Accum(3);\na.push(4);\ndisplayln(a.total());\nvar m|ptr = Accum.make(9);\nAccum_delete(a);\nAccum_delete(m);\n"
    );
    let ll = llvm(&src);
    // ctor + method + getter-flavored total + static + deletes all lower.
    assert!(
        ll.contains("call ptr @cdi_wrap_"),
        "no wrapper calls:\n{ll}"
    );
    // `a.push(4)` passes the handle first (arity 2).
    // Order: add2=0, delete=1, ctor=2, push=3, total=4, make=5, get=6, set=7.
    assert!(
        ll.contains("declare ptr @cdi_wrap_3(ptr, ptr)"),
        "method arity:\n{ll}"
    );
}

#[test]
fn field_sugar_reads_and_writes() {
    let src = format!(
        "{PRELUDE}\nvar a|ptr = Accum(1);\ndisplayln(a.size);\nset a.size = 9;\nAccum_delete(a);\n"
    );
    let ll = llvm(&src);
    assert!(
        ll.contains("call ptr @cdi_wrap_"),
        "field wrappers called:\n{ll}"
    );
}

#[test]
fn overloads_dispatch_on_arity() {
    let src = r#"
[include-cpp "o.h"]
[extern-cpp f "ns::f" [int] int]
[extern-cpp f "ns::f2" [int int] int]
displayln(f(1));
displayln(f(1, 2));
"#;
    let ll = llvm(src);
    assert!(
        ll.contains("call ptr @cdi_wrap_0(ptr "),
        "arity-1 overload:\n{ll}"
    );
    assert!(
        ll.contains("call ptr @cdi_wrap_1(ptr "),
        "arity-2 overload:\n{ll}"
    );
}

#[test]
fn thin_c_functions_go_through_the_same_shim() {
    let src = r#"
[include-c "string.h"]
[extern-c cstrlen "strlen" [string] int]
displayln(cstrlen("hello"));
"#;
    let ll = llvm(src);
    assert!(
        ll.contains("declare ptr @cdi_wrap_0(ptr)"),
        "wrapper declare:\n{ll}"
    );
    let s = shim(src);
    assert!(s.contains("#include \"string.h\""), "C header:\n{s}");
    assert!(s.contains("::strlen"), "C symbol in global scope:\n{s}");
}

#[test]
fn no_native_means_no_wrappers() {
    let ll = llvm("displayln(1);\n");
    assert!(
        !ll.contains("cdi_wrap_"),
        "plain programs stay plain:\n{ll}"
    );
    let p = prog("displayln(1);\n");
    assert!(compile_cpp_shim(&p).is_err(), "shim requires directives");
}

#[test]
fn top_level_strings_define_their_globals() {
    // Regression: main-body string constants must be defined at module
    // level (clang rejects references without definitions).
    for src in [
        "displayln(\"hi\");\n".to_string(),
        format!("{PRELUDE}\ndisplayln(add2(20, 22));\ndisplayln(\"hey\");\n"),
    ] {
        let ll = llvm(src.as_str());
        for line in ll.lines() {
            if let Some(pos) = line.find("@.str.") {
                let name: String = line[pos..]
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '.' || *c == '_' || *c == '@')
                    .collect();
                assert!(
                    ll.contains(&format!("{name} = private unnamed_addr constant")),
                    "undefined string global {name}:\n{ll}"
                );
            }
        }
    }
}

#[test]
fn error_surface() {
    // Unknown type in decls.
    assert!(llvm_err("[extern-c f [nope] int]\ndisplayln(f(1));\n").contains("unknown native type"));
    // Duplicate alias.
    assert!(
        llvm_err("[extern-c f [int] int]\n[extern-c f [int] int]\ndisplayln(f(1));\n")
            .contains("duplicate native declaration")
    );
    // C has no overloads.
    assert!(
        llvm_err("[extern-c f [int] int]\n[extern-c f [int int] int]\ndisplayln(f(1));\n")
            .contains("duplicate native declaration")
    );
    // Arity mismatch at the call.
    assert!(
        llvm_err("[include-c \"s.h\"]\n[extern-c f [int] int]\ndisplayln(f(1, 2));\n")
            .contains("arity mismatch in call to f")
    );
    // Method on unknown class.
    assert!(llvm_err("[cpp-method Ghost go [] int]\n").contains("unknown cpp-class"));
    // Bare class is not a value.
    let e = llvm_err(&format!("{PRELUDE}\ndisplayln(Accum);\n"));
    assert!(
        e.contains("not a value") && e.contains("construct"),
        "bare class: {e}"
    );
    // Unknown member on a bound handle.
    let e = llvm_err(&format!(
        "{PRELUDE}\nvar a|ptr = Accum(1);\na.bogus(2);\nAccum_delete(a);\n"
    ));
    assert!(
        e.contains("unknown cpp member") || e.contains("unknown cpp method"),
        "member: {e}"
    );
    // Directive inside a proc body.
    let e = llvm_err("fn f(x) { [include-c \"s.h\"]; return x; }\ndisplayln(f(1));\n");
    assert!(e.contains("must be top-level"), "top-level: {e}");
    // One namespace: proc vs native alias.
    let e = llvm_err(
        "[include-c \"s.h\"]\n[extern-c f [int] int]\nfn f(x) { return x; }\ndisplayln(f(1));\n",
    );
    assert!(e.contains("native"), "proc collision: {e}");
    // Native alias vs builtin.
    let e = llvm_err("[extern-c display [int] int]\ndisplayln(1);\n");
    assert!(
        e.contains("collides with builtin"),
        "builtin collision: {e}"
    );
}

#[test]
fn ambiguous_methods_require_explicit_alias() {
    let src = r#"
[include-cpp "a.h" "b.h"]
[cpp-class A "a.h"]
[cpp-class B "b.h"]
[cpp-ctor A []]
[cpp-ctor B []]
[cpp-method A go [] int]
[cpp-method B go [] int]
var a|ptr = A();
displayln(a.go());
"#;
    let e = llvm_err(src);
    assert!(e.contains("ambiguous cpp method"), "ambiguity: {e}");
    // Explicit aliases still work (without the ambiguous sugar line).
    let ok = r#"
[include-cpp "a.h" "b.h"]
[cpp-class A "a.h"]
[cpp-class B "b.h"]
[cpp-ctor A []]
[cpp-ctor B []]
[cpp-method A go [] int]
[cpp-method B go [] int]
var a|ptr = A();
var b|ptr = B();
displayln(A_go(a));
displayln(B_go(b));
A_delete(a);
B_delete(b);
"#;
    let ll = llvm(ok);
    assert!(
        ll.contains("call ptr @cdi_wrap_"),
        "explicit alias calls:\n{ll}"
    );
}

const CONST_PRELUDE: &str = r#"
[include-cpp "consts.h"]
[cpp-enum RED "ns::Color::RED" GREEN "ns::Color::GREEN"]
[cpp-enum MODE_ON "ns::Mode::ON"]
[cpp-const MAX_ITEMS "MAX_ITEMS" int]
[cpp-const PI_D "ns::PI_D" double]
[cpp-const VERBOSE "ns::VERBOSE" bool]
[cpp-const APP_NAME "APP_NAME" string]
[cpp-class Cfg "ns::Cfg" "consts.h"]
[cpp-static-field Cfg count int]
[cpp-static-field Cfg tag string]
[cpp-static-field Cfg LIMIT int const]
"#;

#[test]
fn consts_read_as_values() {
    let src = format!("{CONST_PRELUDE}\ndisplayln(RED);\ndisplayln(GREEN);\ndisplayln(MODE_ON);\ndisplayln(MAX_ITEMS);\ndisplayln(PI_D);\ndisplayln(VERBOSE);\ndisplayln(APP_NAME);\n");
    let ll = llvm(&src);
    // Seven const wrappers (3 enum + 4 const), all 0-arity declares.
    assert!(ll.contains("declare ptr @cdi_wrap_"), "declares:\n{ll}");
    let s = shim(CONST_PRELUDE);
    // Scoped enums need casts; macros pass through verbatim.
    assert!(s.contains("(int32_t)(ns::Color::RED)"), "enum cast:\n{s}");
    assert!(
        s.contains("(int32_t)(ns::Mode::ON)"),
        "scoped enum cast:\n{s}"
    );
    assert!(s.contains("(double)(ns::PI_D)"), "double const:\n{s}");
    assert!(s.contains("(bool)(ns::VERBOSE)"), "bool const:\n{s}");
    assert!(
        s.contains("std::string r = (APP_NAME);"),
        "string const:\n{s}"
    );
    // Call form works too (0-arity alias).
    let ll = llvm(&format!("{CONST_PRELUDE}\ndisplayln(RED());\n"));
    assert!(ll.contains("call ptr @cdi_wrap_"), "call form:\n{ll}");
}

#[test]
fn static_fields_sugar_and_aliases() {
    let src = format!("{CONST_PRELUDE}\ndisplayln(Cfg.count);\nset Cfg.count = 41;\ndisplayln(Cfg.tag);\nset Cfg.tag = \"v9\";\ndisplayln(Cfg.LIMIT);\n");
    let ll = llvm(&src);
    assert!(ll.contains("call ptr @cdi_wrap_"), "static calls:\n{ll}");
    let s = shim(CONST_PRELUDE);
    assert!(s.contains("ns::Cfg::count;"), "static read:\n{s}");
    assert!(s.contains("ns::Cfg::tag = p0;"), "static write:\n{s}");
    // Explicit sget/sset aliases lower directly.
    let ll = llvm(&format!(
        "{CONST_PRELUDE}\ndisplayln(Cfg_count_sget());\nCfg_count_sset(7);\n"
    ));
    assert!(
        ll.contains("call ptr @cdi_wrap_"),
        "explicit aliases:\n{ll}"
    );
}

#[test]
fn constness_is_dirac_checked() {
    // Const statics: reads fine, writes fail before the C++ compiler.
    let e = llvm_err(&format!("{CONST_PRELUDE}\nset Cfg.LIMIT = 3;\n"));
    assert!(
        e.contains("cannot assign to constant 'Cfg.LIMIT'"),
        "static const: {e}"
    );
    // Named constants are not assignable either.
    let e = llvm_err(&format!("{CONST_PRELUDE}\nset RED = 5;\n"));
    assert!(
        e.contains("cannot assign to constant 'RED'"),
        "bare const: {e}"
    );
    // Instance const fields behave the same.
    let src = "[include-cpp \"w.h\"]\n[cpp-class W \"w.h\"]\n[cpp-ctor W []]\n[cpp-field W id int const]\nvar w = W();\nset w.id = 1;\n";
    let e = llvm_err(src);
    assert!(
        e.contains("cannot assign to constant 'w.id'"),
        "instance const: {e}"
    );
}

#[test]
fn const_static_error_surface() {
    // Odd pair counts, bad shapes, bad types.
    let e = llvm_err("[cpp-enum RED]\n");
    assert!(e.contains("pairs"), "enum pairs: {e}");
    let e = llvm_err("[cpp-const X \"Y\"]\n");
    assert!(e.contains("expects Alias"), "const shape: {e}");
    let e = llvm_err("[cpp-const X \"Y\" ptr]\n");
    assert!(
        e.contains("must be int, double, bool, or string"),
        "const type: {e}"
    );
    let e = llvm_err("[cpp-static-field Ghost f int]\n");
    assert!(
        e.contains("unknown cpp-class 'Ghost'"),
        "unknown class: {e}"
    );
    let e = llvm_err(&format!("{CONST_PRELUDE}\ndisplayln(Cfg.bogus);\n"));
    assert!(
        e.contains("unknown cpp static 'bogus'"),
        "unknown static: {e}"
    );
    // Instance/static cross-hints both directions.
    let e = llvm_err(&format!("{CONST_PRELUDE}\ndisplayln(Cfg.count_sget);\n"));
    assert!(
        e.contains("unknown cpp static") || e.contains("unbound"),
        "sget is alias-only: {e}"
    );
    // Duplicate const alias.
    let e = llvm_err("[cpp-enum A \"ns::A\" A \"ns::B\"]\n");
    assert!(e.contains("duplicate native declaration 'A'"), "dup: {e}");
    // Const alias colliding with a class.
    let e = llvm_err("[cpp-class W \"w.h\"]\n[cpp-const W \"ns::W\" int]\n");
    assert!(e.contains("collides with cpp-class"), "collision: {e}");
    // Lookup-first: a proc-local var named like a const wins on reads
    // (top-level redefinition stays a collision, like all natives).
    let ll = llvm(&format!(
        "{CONST_PRELUDE}\nfn f() | int {{ var RED = 99; return RED; }}\ndisplayln(f());\n"
    ));
    assert!(ll.contains("99"), "local shadows const");
}

const VEC_PRELUDE: &str = r#"
[include-cpp "<vector>"]
[cpp-vector IntVec int]
[cpp-vector StrVec string]
"#;
#[test]
fn vectors_bridge_len_get_push_and_aliases() {
    // Two vector classes share method names, so dot sugar is ambiguous:
    // the documented form is explicit `Alias_op` aliases (see vecs.cdc,
    // which uses one class per method-name where sugar applies... it uses
    // explicit aliases for the shared ones too).
    let src = format!("{VEC_PRELUDE}\nvar v = IntVec(1, 2, 3);\ndisplayln(IntVec_len(v));\ndisplayln(IntVec_get(v, 0));\nIntVec_push(v, 4);\ndisplayln(IntVec_len(v));\nIntVec_delete(v);\n");
    let ll = llvm(&src);
    // Build + ops + delete all lower through wrappers.
    assert!(ll.contains("call ptr @cdi_wrap_"), "vec calls:\n{ll}");
    let s = shim(VEC_PRELUDE);
    assert!(s.contains("std::vector<int32_t>"), "elem map:\n{s}");
    assert!(s.contains("std::vector<std::string>"), "string elem:\n{s}");
    assert!(s.contains("extern \"C\""), "stable linkage:\n{s}");
    // Single-class prelude: dot sugar resolves (no ambiguity).
    let solo = "[include-cpp \"<vector>\"]\n[cpp-vector Solo int]\nvar v = Solo(1);\ndisplayln(v.len());\nSolo_delete(v);\n";
    let ll = llvm(solo);
    assert!(ll.contains("call ptr @cdi_wrap_"), "sugar lowers:\n{ll}");
    // ...but the two-class prelude rejects sugar with an ambiguity error.
    let e = llvm_err(&format!(
        "{VEC_PRELUDE}\nvar v = IntVec(1);\ndisplayln(v.len());\nIntVec_delete(v);\n"
    ));
    assert!(e.contains("ambiguous cpp method"), "ambiguity: {e}");
}

#[test]
fn vectors_reject_bool_void_nested() {
    assert!(llvm_err("[cpp-vector B bool]\n").contains("bit-packed"));
    assert!(llvm_err("[cpp-vector V void]\n").contains("not an element type"));
    // Nested: a vector alias as another vector's element.
    let e = llvm_err("[cpp-vector A int]\n[cpp-vector B A]\n");
    assert!(e.contains("nested vectors"), "nested: {e}");
    // IntVec(5) builds [5]: single-element build lowers (no size ctor).
    let ll = llvm(&format!(
        "{VEC_PRELUDE}\nvar v = IntVec(5);\ndisplayln(IntVec_len(v));\nIntVec_delete(v);\n"
    ));
    assert!(ll.contains("call ptr @cdi_wrap_"), "build lowers:\n{ll}");
}

const VARARGS_PRELUDE: &str = r#"
[include-c "stdio.h"]
[extern-c cprintf "printf" [string] int varargs]
"#;

#[test]
fn varargs_calls_lower_to_nary_wrappers() {
    // Fixed arity + 0..4 extras all lower through the n-ary shape.
    for call in [
        "cprintf(\"x\\n\")",
        "cprintf(\"%d\\n\", 1)",
        "cprintf(\"%d %s\\n\", 42, \"yo\")",
        "cprintf(\"%d %d %f %s\\n\", 1, 2, 3.5, \"z\")",
    ] {
        let ll = llvm(&format!("{VARARGS_PRELUDE}\ndisplayln({call});\n"));
        assert!(
            ll.contains("declare ptr @cdi_wrap_0(ptr, i32)"),
            "n-ary declare:\n{ll}"
        );
        assert!(
            ll.contains("call ptr @cdi_wrap_0(ptr "),
            "n-ary call:\n{ll}"
        );
    }
    // Below the fixed floor still fails, naming the range.
    let e = llvm_err("[extern-c f [int int] int varargs]\ndisplayln(f(1));\n");
    assert!(e.contains("expects 2 or more (got 1)"), "range error: {e}");
    // Above the cap (fixed + 4) fails the same way.
    let e = llvm_err(&format!(
        "{VARARGS_PRELUDE}\ndisplayln(cprintf(\"x\", 1, 2, 3, 4, 5));\n"
    ));
    assert!(e.contains("expects 1 or more (got 6)"), "cap error: {e}");
}

#[test]
fn varargs_shim_forwards_by_tag() {
    let s = shim(VARARGS_PRELUDE);
    assert!(s.contains("Value** a, int32_t n"), "n-ary sig:\n{s}");
    assert!(s.contains("::printf(p0"), "fixed format + extras:\n{s}");
    assert!(s.contains("cdi_to_cstring"), "string extras:\n{s}");
    assert!(s.contains("cdi_to_double"), "double extras:\n{s}");
    assert!(s.contains("at most 4 varargs"), "cap guard:\n{s}");
    assert!(s.contains("cannot pass vararg"), "bad-tag guard:\n{s}");
}

#[test]
fn varargs_error_surface() {
    // Void is never a parameter type, varargs or not.
    assert!(llvm_err("[extern-c f [void] int]\ndisplayln(f(1));\n")
        .contains("void is only valid as the return type"));
    assert!(
        llvm_err("[extern-c f [void] int varargs]\ndisplayln(f(1));\n")
            .contains("void is only valid as the return type")
    );
    // A second decl on a varargs alias is a duplicate (range covers all).
    let e =
        llvm_err("[extern-c f [int] int varargs]\n[extern-c f [int int] int]\ndisplayln(f(1));\n");
    assert!(e.contains("duplicate native declaration"), "dup: {e}");
    // ...in either order.
    let e = llvm_err("[extern-c f [int] int]\n[extern-c f [int] int varargs]\ndisplayln(f(1));\n");
    assert!(e.contains("duplicate native declaration"), "dup: {e}");
    // C++ spelling works too.
    let ll = llvm(
        "[include-cpp \"x.h\"]\n[extern-cpp g \"ns::g\" [int] int varargs]\ndisplayln(g(1, 2));\n",
    );
    assert!(
        ll.contains("declare ptr @cdi_wrap_0(ptr, i32)"),
        "cxx n-ary:\n{ll}"
    );
    let s = shim("[include-cpp \"x.h\"]\n[extern-cpp g \"ns::g\" [int] int varargs]\n");
    assert!(s.contains("ns::g(p0"), "qualified callee:\n{s}");
}

const STRUCT_PRELUDE: &str = r#"
[include-cpp "s.h"]
[cpp-struct Point "ns::Point" "s.h" [x int] [y int]]
[extern-cpp move_by "ns::move_by" [Point int] Point]
"#;

#[test]
fn structs_convert_by_value_both_ways() {
    let ll = llvm(&format!(
        "{STRUCT_PRELUDE}\ndisplayln(move_by(array(3, 4), 10));\n"
    ));
    assert!(ll.contains("call ptr @cdi_wrap_"), "wrapper call:\n{ll}");
    let s = shim(STRUCT_PRELUDE);
    // Unbox: array shape check + per-field conversion + aggregate init.
    assert!(
        s.contains("cannot pass value as Point (expects array of 2 fields)"),
        "tag check:\n{s}"
    );
    assert!(
        s.contains("cannot pass array as Point (expects 2 fields)"),
        "arity check:\n{s}"
    );
    assert!(s.contains("ns::Point p0{p0_f0, p0_f1};"), "init:\n{s}");
    // Box: array build from the returned fields.
    assert!(
        s.contains("ns::Point r = ns::move_by(p0, p1);"),
        "call:\n{s}"
    );
    assert!(
        s.contains("Value* r_boxed = cdi_array0();"),
        "box array:\n{s}"
    );
    assert!(
        s.contains("cdi_arr_push(r_boxed, r_boxed_f0);"),
        "push:\n{s}"
    );
}

#[test]
fn structs_nest_and_mix_scalar_kinds() {
    let src = "[include-cpp \"s.h\"]\n[cpp-struct Inner \"ns::Inner\" \"s.h\" [v double] [ok bool] [tag string]]\n[cpp-struct Outer \"ns::Outer\" \"s.h\" [id int] [inner Inner] [name string]]\n[extern-cpp roundtrip \"ns::roundtrip\" [Outer] Outer]\ndisplayln(roundtrip(array(1, array(2.5, true, \"x\"), \"y\")));\n";
    let s = shim(src);
    assert!(s.contains("ns::Inner p0_f1{p0_f1_f0"), "nested init:\n{s}");
    assert!(
        s.contains("std::string p0_f2(p0_f2_b, (size_t)p0_f2_n);"),
        "string field:\n{s}"
    );
    assert!(
        s.contains("bool p0_f1_f1 = cdi_to_bool(p0_f1_v1) != 0;"),
        "bool field:\n{s}"
    );
    let ll = llvm(src);
    assert!(ll.contains("call ptr @cdi_wrap_"), "call:\n{ll}");
}

#[test]
fn structs_reject_bad_decls() {
    // Shape: alias + qualified + header + >= 1 field.
    let e = llvm_err("[cpp-struct OnlyAlias]\ndisplayln(1);\n");
    assert!(e.contains("cpp-struct: expects Alias"), "shape: {e}");
    // Duplicate struct name.
    let e = llvm_err("[cpp-struct S \"ns::S\" \"s.h\" [x int]]\n[cpp-struct S \"ns::S\" \"s.h\" [x int]]\ndisplayln(1);\n");
    assert!(e.contains("duplicate native declaration 'S'"), "dup: {e}");
    // Collisions across namespaces.
    let e = llvm_err("[cpp-class S \"ns::S\" \"s.h\"]\n[cpp-struct S \"ns::S\" \"s.h\" [x int]]\ndisplayln(1);\n");
    assert!(e.contains("collides with cpp-class"), "class: {e}");
    let e = llvm_err(
        "[extern-c S [int] int]\n[cpp-struct S \"ns::S\" \"s.h\" [x int]]\ndisplayln(1);\n",
    );
    assert!(e.contains("collides with native alias"), "alias: {e}");
    let e = llvm_err(
        "[cpp-struct S \"ns::S\" \"s.h\" [x int]]\n[extern-c S [int] int]\ndisplayln(1);\n",
    );
    assert!(e.contains("collides with cpp-struct"), "ralias: {e}");
    let e = llvm_err("[cpp-struct S \"ns::S\" \"s.h\" [x int]]\n[cpp-class S \"ns::S\" \"s.h\"]\ndisplayln(1);\n");
    assert!(e.contains("collides with cpp-struct"), "rclass: {e}");
    // Field shape, dup fields, void fields, unknown types.
    let e = llvm_err("[cpp-struct S \"ns::S\" \"s.h\" [x]]\ndisplayln(1);\n");
    assert!(e.contains("expects [field type] pairs"), "fshape: {e}");
    let e = llvm_err("[cpp-struct S \"ns::S\" \"s.h\" [x int] [x int]]\ndisplayln(1);\n");
    assert!(e.contains("duplicate field 'x'"), "fdup: {e}");
    let e = llvm_err("[cpp-struct S \"ns::S\" \"s.h\" [x void]]\ndisplayln(1);\n");
    assert!(e.contains("void is not a field type"), "void: {e}");
    let e = llvm_err("[cpp-struct S \"ns::S\" \"s.h\" [x nope]]\ndisplayln(1);\n");
    assert!(e.contains("unknown native type: nope"), "unknown: {e}");
    // Use-before-declare reads as unknown (order matters, like classes).
    let e = llvm_err("[extern-cpp f \"ns::f\" [S] int]\n[cpp-struct S \"ns::S\" \"s.h\" [x int]]\ndisplayln(1);\n");
    assert!(e.contains("unknown native type: S"), "order: {e}");
    // Bare alias is not a value; procs/vars cannot take the name.
    let e = llvm_err(&format!("{STRUCT_PRELUDE}\ndisplayln(Point);\n"));
    assert!(e.contains("is a cpp-struct, not a value"), "value: {e}");
    let e = llvm_err(&format!(
        "{STRUCT_PRELUDE}\nfn Point(x|int) | int {{ return x; }}\ndisplayln(1);\n"
    ));
    assert!(
        e.contains("cannot redefine native 'Point' as a procedure"),
        "proc: {e}"
    );
    let e = llvm_err(&format!(
        "{STRUCT_PRELUDE}\nvar Point = 1;\ndisplayln(1);\n"
    ));
    assert!(
        e.contains("cannot redefine native 'Point' as a variable"),
        "var: {e}"
    );
    // Structs are not vector elements.
    let e = llvm_err("[cpp-struct S \"ns::S\" \"s.h\" [x int]]\n[cpp-vector V S]\ndisplayln(1);\n");
    assert!(e.contains("struct elements are not supported"), "vec: {e}");
}

const CALLBACK_PRELUDE: &str = r#"
[include-c "cb.h"]
[callback inc [int] int]
[extern-c apply_twice "apply_twice" [inc int] int]
fn inc(x|int) | int { return x + 1; }
"#;

#[test]
fn callbacks_emit_trampolines() {
    let ll = llvm(&format!(
        "{CALLBACK_PRELUDE}\ndisplayln(apply_twice(inc, 41));\n"
    ));
    // Trampoline: C ABI in, boxed call, unboxed return.
    assert!(ll.contains("define i32 @cdi_cb_inc_"), "trampoline:\n{ll}");
    assert!(
        ll.contains("call ptr @cdi_mk_int(i32 %a0)"),
        "box arg:\n{ll}"
    );
    assert!(ll.contains("call ptr @cdi_proc_inc_"), "proc call:\n{ll}");
    assert!(ll.contains("call i32 @cdi_to_int(ptr "), "unbox ret:\n{ll}");
    // Bare name boxes the trampoline address (not proc-not-value).
    assert!(
        ll.contains("call ptr @cdi_from_ptr(ptr @cdi_cb_inc_"),
        "address box:\n{ll}"
    );
    // Conversion declares ride along only when callbacks exist.
    for sym in [
        "declare i32 @cdi_to_int(ptr)",
        "declare ptr @cdi_from_cstring(ptr)",
        "declare ptr @cdi_to_cstring(ptr)",
        "declare ptr @cdi_from_ptr(ptr)",
        "declare ptr @cdi_to_ptr(ptr)",
    ] {
        assert!(ll.contains(sym), "declare {sym}:\n{ll}");
    }
    // ...and stay out of callback-free IR.
    let plain = llvm("displayln(1);\n");
    assert!(
        !plain.contains("cdi_from_ptr"),
        "no conversion declares:\n{plain}"
    );
}

#[test]
fn callbacks_cover_all_scalar_kinds() {
    let src = "[callback f [int double bool string ptr] void]\nfn f(a|int, b|int, c|int, d|int, e|int) { displayln(a); }\ndisplayln(1);\n";
    let ll = llvm(src);
    assert!(ll.contains("define void @cdi_cb_f_"), "trampoline:\n{ll}");
    assert!(
        ll.contains("(i32 %a0, double %a1, i8 %a2, ptr %a3, ptr %a4)"),
        "sig:\n{ll}"
    );
    assert!(ll.contains("zext i8 %a2 to i32"), "bool box:\n{ll}");
    assert!(
        ll.contains("call ptr @cdi_from_cstring(ptr %a3)"),
        "string box:\n{ll}"
    );
    assert!(
        ll.contains("call ptr @cdi_from_ptr(ptr %a4)"),
        "ptr box:\n{ll}"
    );
    assert!(ll.contains("ret void"), "void ret:\n{ll}");
    // Bool results truncate back to i8.
    let ll = llvm("[callback p [int] bool]\nfn p(x|int) | bool { return x > 0; }\ndisplayln(1);\n");
    assert!(ll.contains("define i8 @cdi_cb_p_"), "trampoline:\n{ll}");
    assert!(ll.contains("(i32 %a0)"), "bool sig:\n{ll}");
    assert!(ll.contains("trunc i32 "), "bool unbox:\n{ll}");
    assert!(ll.contains("ret i8 "), "bool ret:\n{ll}");
}

#[test]
fn callbacks_reject_bad_decls() {
    // Shape: alias + params + ret.
    let e = llvm_err("[callback OnlyAlias]\ndisplayln(1);\n");
    assert!(e.contains("callback: expects Alias"), "shape: {e}");
    // Duplicate callback.
    let e = llvm_err("[callback f [int] int]\n[callback f [int] int]\ndisplayln(1);\n");
    assert!(e.contains("duplicate native declaration 'f'"), "dup: {e}");
    // Collisions across namespaces, both orders.
    let e = llvm_err("[cpp-class C \"ns::C\" \"s.h\"]\n[callback C [int] int]\ndisplayln(1);\n");
    assert!(e.contains("collides with cpp-class"), "class: {e}");
    let e = llvm_err("[callback C [int] int]\n[cpp-class C \"ns::C\" \"s.h\"]\ndisplayln(1);\n");
    assert!(e.contains("collides with callback"), "rclass: {e}");
    let e = llvm_err("[extern-c f [int] int]\n[callback f [int] int]\ndisplayln(1);\n");
    assert!(e.contains("collides with native alias"), "alias: {e}");
    let e = llvm_err("[callback f [int] int]\n[extern-c f [int] int]\ndisplayln(1);\n");
    assert!(e.contains("collides with callback"), "ralias: {e}");
    // Signature stays scalar: no void params, no class/struct/func.
    let e = llvm_err("[callback f [void] int]\ndisplayln(1);\n");
    assert!(e.contains("void is not a parameter type"), "void: {e}");
    let e = llvm_err("[cpp-class C \"ns::C\" \"s.h\"]\n[callback f [C] int]\ndisplayln(1);\n");
    assert!(e.contains("signatures support"), "class: {e}");
    let e = llvm_err("[extern-cpp f \"ns::f\" [Cb] int]\n[callback Cb [int] int]\ndisplayln(1);\n");
    assert!(e.contains("unknown native type: Cb"), "order: {e}");
    // Target must be a matching top-level proc.
    let e = llvm_err("[callback f [int] int]\ndisplayln(1);\n");
    assert!(
        e.contains("has no matching top-level procedure 'f'"),
        "missing: {e}"
    );
    let e =
        llvm_err("[callback f [int int] int]\nfn f(x|int) | int { return x; }\ndisplayln(1);\n");
    assert!(e.contains("arity mismatch"), "arity: {e}");
    let e = llvm_err("fn outer() {\nfn q() | int {\nreturn 1;\n}\nreturn q();\n}\n[callback q [int] int]\ndisplayln(outer());\n");
    assert!(e.contains("must name a top-level procedure"), "nested: {e}");
    // A same-named var is still a collision (trampoline needs the proc).
    let e = llvm_err(
        "[callback f [int] int]\nfn f(x|int) | int { return x; }\nvar f = 1;\ndisplayln(1);\n",
    );
    assert!(e.contains("cannot redefine"), "var: {e}");
}
