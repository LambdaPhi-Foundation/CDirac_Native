//! Robustness gate: the frontend and lowering must never panic.
//!
//! `parse_source` / `compile_llvm` / `compile_cpp_shim` return `Err` on
//! bad input; `unreachable!`/`expect` inside are documented invariants,
//! not user-error paths. This test throws deterministic mutations at
//! all three and fails on any panic (with the offending input attached).
//! New `unreachable!` panics found here must be converted to `Err`.

use std::collections::HashSet;

use cdirac::llvmgen::{compile_cpp_shim, compile_llvm};
use cdirac::parser::parse_source;

fn seeds() -> Vec<String> {
    let mut out = vec![
        "displayln(1);\n".to_string(),
        "fn fact(n|int) | int {\nif (n <= 1) { return 1; }\nreturn n * fact(n - 1);\n}\ndisplayln(fact(5));\n".to_string(),
        "var x = \"hi \\\"yo\\\"\";\nfor x in array(1, 2) {\nset x = x + 1;\n}\nmachine.displayln(x);\n".to_string(),
        "[include-c \"stdio.h\"]\n[extern-c cprintf \"printf\" [string] int varargs]\ndisplayln(cprintf(\"%d %s\", 42, \"yo\"));\n".to_string(),
        "fn outer(n|int) | int {\nvar m = n + 1;\nfn helper(x|int) | int {\nreturn x + m;\n}\nreturn helper(n);\n}\ndisplayln(outer(3));\n".to_string(),
        "var f = 3.14159265358979;\nvar big = 99999999999999999999999;\nvar neg = 0 - 3.5;\ndisplayln(f);\n".to_string(),
        "[cpp-vector V int]\n[include-cpp \"<vector>\"]\nvar v = V(1);\nV_push(v, 2);\nV_delete(v);\n".to_string(),
    ];
    // Plus every local example (exercises real shapes, incl. errors).
    let dirs = [
        "examples",
        "examples/cpp",
        "examples/closures",
        "filetest/ok",
        "filetest/err",
    ];
    let mut seen = HashSet::new();
    for d in dirs {
        let rd = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(d);
        let Ok(rd) = std::fs::read_dir(rd) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|e| e.to_str()) == Some("cdc") {
                if let Ok(s) = std::fs::read_to_string(&p) {
                    if seen.insert(s.clone()) {
                        out.push(s);
                    }
                }
            }
        }
    }
    out
}

fn lcg(state: &mut u64) -> u64 {
    *state = state
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    *state >> 33
}

const PIECES: &[&str] = &[
    "(",
    ")",
    "{",
    "}",
    "[",
    "]",
    ";",
    "\"",
    ".",
    "|",
    "fn",
    "var",
    "set",
    "if",
    "else",
    "for",
    "while",
    "return",
    "machine",
    "displayln",
    "0",
    "999999999999999999999999",
    "3.5",
    "1e999",
    "-",
    "&&",
    "&",
    "--",
    "'",
    "`",
    "#",
    "@",
    "varargs",
    "\n",
    " ",
    "\u{7f}",
    "\u{e9}",
    "\u{1f600}",
];

fn mutate(seed: &str, state: &mut u64, out: &mut String) {
    out.clear();
    out.push_str(seed);
    let ops = 1 + (lcg(state) % 3) as usize;
    for _ in 0..ops {
        let chars: Vec<char> = out.chars().collect();
        if chars.is_empty() {
            out.push_str(PIECES[(lcg(state) as usize) % PIECES.len()]);
            continue;
        }
        match lcg(state) % 4 {
            0 => {
                // Replace a char.
                let i = (lcg(state) as usize) % chars.len();
                let mut v: Vec<char> = chars;
                let r = PIECES[(lcg(state) as usize) % PIECES.len()];
                v[i] = r.chars().next().unwrap_or('x');
                *out = v.into_iter().collect();
            }
            1 => {
                // Insert a piece.
                let i = (lcg(state) as usize) % (chars.len() + 1);
                let mut v: String = chars[..i].iter().collect();
                v.push_str(PIECES[(lcg(state) as usize) % PIECES.len()]);
                v.extend(chars[i..].iter());
                *out = v;
            }
            2 => {
                // Delete a span.
                let i = (lcg(state) as usize) % chars.len();
                let n = 1 + (lcg(state) as usize) % 8;
                let v: Vec<char> = chars;
                *out = v
                    .into_iter()
                    .enumerate()
                    .filter(|(j, _)| *j < i || *j >= i + n)
                    .map(|(_, c)| c)
                    .collect();
            }
            _ => {
                // Truncate.
                let i = (lcg(state) as usize) % (chars.len() + 1);
                *out = chars[..i].iter().collect();
            }
        }
    }
}

#[test]
fn no_panics_on_mutated_inputs() {
    let seeds = seeds();
    assert!(!seeds.is_empty());
    let mut state: u64 = 0x1234_5678_9abc_def1;
    let mut buf = String::new();
    let mut exercised_err = 0;
    let mut exercised_ok = 0;
    for seed in &seeds {
        for _ in 0..80 {
            mutate(seed, &mut state, &mut buf);
            let input = buf.clone();
            let r = std::panic::catch_unwind(|| {
                let mut ok = false;
                let mut err = false;
                match parse_source(&input) {
                    Ok(prog) => {
                        ok = true;
                        // Lowering must not panic either (errors fine).
                        if compile_llvm(&prog).is_ok() {
                            ok = true;
                        } else {
                            err = true;
                        }
                        let _ = compile_cpp_shim(&prog);
                    }
                    Err(_) => err = true,
                }
                (ok, err)
            });
            match r {
                Ok((ok, err)) => {
                    if ok {
                        exercised_ok += 1;
                    }
                    if err {
                        exercised_err += 1;
                    }
                }
                Err(_) => panic!("panic on mutated input (seed {seed:?}): {input:?}"),
            }
        }
    }
    // The campaign must actually exercise both outcomes.
    assert!(exercised_ok > 0, "no mutated input parsed");
    assert!(exercised_err > 0, "no mutated input rejected");
}

#[test]
fn corpus_files_lower_without_panics() {
    // Shared corpus + examples: parse errors are fine (some files are
    // negative tests for *this* frontend), panics are not.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    for d in [
        "examples",
        "examples/cpp",
        "examples/closures",
        "filetest/ok",
        "filetest/err",
    ] {
        if let Ok(rd) = std::fs::read_dir(root.join(d)) {
            for e in rd.flatten() {
                if e.path().extension().and_then(|e| e.to_str()) == Some("cdc") {
                    files.push(e.path());
                }
            }
        }
    }
    let cdc = root.join("../bracket-jvm/examples/cdc");
    if let Ok(rd) = std::fs::read_dir(&cdc) {
        for e in rd.flatten() {
            if e.path().extension().and_then(|e| e.to_str()) == Some("cdc") {
                files.push(e.path());
            }
        }
    }
    assert!(!files.is_empty());
    for f in files {
        let src = std::fs::read_to_string(&f).unwrap();
        let r = std::panic::catch_unwind(|| {
            if let Ok(prog) = parse_source(&src) {
                let _ = compile_llvm(&prog);
                let _ = compile_cpp_shim(&prog);
            }
        });
        assert!(r.is_ok(), "panic on file {}", f.display());
    }
}
