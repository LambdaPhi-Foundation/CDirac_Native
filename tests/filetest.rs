//! File-based checks over `filetest/` (see `filetest/README.md`).
//!
//! - `filetest/ok/*.cdc` must parse, canonical-print, and lower to
//!   LLVM IR (plus a C++ shim when native directives are present).
//! - `filetest/err/*.cdc` must fail at parse or lowering time with the
//!   diagnostic declared in its `// ERROR: <substring>` marker.
//!
//! No toolchain needed; execution stays in `tools/llvmcheck.py`.

use cdirac::llvmgen::{compile_cpp_shim, compile_llvm};
use cdirac::parser::parse_source;
use std::fs;

fn filetest_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("filetest")
}

fn cdc_files(sub: &str) -> Vec<String> {
    let base = filetest_dir().join(sub);
    let mut out = Vec::new();
    for entry in fs::read_dir(&base).unwrap_or_else(|e| panic!("{base:?}: {e}")) {
        let p = entry.unwrap().path();
        if p.extension().and_then(|e| e.to_str()) == Some("cdc") {
            out.push(p.to_string_lossy().into_owned());
        }
    }
    out.sort();
    out
}

fn has_native_directives(src: &str) -> bool {
    ["[extern-c", "[extern-cpp", "[cpp-", "[include-c"]
        .iter()
        .any(|m| src.contains(m))
}

fn expected_error(src: &str, file: &str) -> String {
    for line in src.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("// ERROR:") {
            let want = rest.trim().to_string();
            assert!(!want.is_empty(), "{file}: empty // ERROR: marker");
            return want;
        }
    }
    panic!("{file}: missing // ERROR: <substring> marker");
}

#[test]
fn ok_files_pass() {
    let files = cdc_files("ok");
    assert!(!files.is_empty(), "no filetest/ok/*.cdc files found");
    for f in &files {
        let src = fs::read_to_string(f).unwrap();
        let prog = parse_source(&src).unwrap_or_else(|e| panic!("{f}: parse failed: {e}"));
        assert!(!prog.is_empty(), "{f}: empty program");
        for node in &prog {
            assert!(!node.print().is_empty(), "{f}: empty print");
        }
        compile_llvm(&prog).unwrap_or_else(|e| panic!("{f}: lowering failed: {e}"));
        if has_native_directives(&src) {
            compile_cpp_shim(&prog).unwrap_or_else(|e| panic!("{f}: shim failed: {e}"));
        }
        println!("ok   {f}");
    }
    println!("{} ok file(s) passed", files.len());
}

#[test]
fn err_files_fail_as_declared() {
    let files = cdc_files("err");
    assert!(!files.is_empty(), "no filetest/err/*.cdc files found");
    for f in &files {
        let src = fs::read_to_string(f).unwrap();
        let want = expected_error(&src, f);
        match parse_source(&src) {
            Err(e) => assert!(
                e.contains(&want),
                "{f}: parse error {e:?} does not contain {want:?}"
            ),
            Ok(prog) => {
                let e = compile_llvm(&prog).err().unwrap_or_else(|| {
                    panic!("{f}: unexpectedly succeeded, want error containing {want:?}")
                });
                assert!(
                    e.contains(&want),
                    "{f}: lowering error {e:?} does not contain {want:?}"
                );
            }
        }
        println!("ok   {f} (fails with {want:?})");
    }
    println!("{} err file(s) failed as declared", files.len());
}
