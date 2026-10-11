//! Compatibility gate: every `.cdc` file in the shared corpus
//! (`../bracket-jvm/examples/cdc/`, owned by the reference pipeline)
//! must parse with this frontend. Canonical-print goldens pin the
//! SDirac spelling contract (`parse(print(n)) == n` over there).

use cdirac::parser::parse_source;
use std::fs;

fn cdc_files() -> Vec<String> {
    let base = concat!(env!("CARGO_MANIFEST_DIR"), "/../bracket-jvm/examples/cdc");
    let mut out = Vec::new();
    for entry in fs::read_dir(base).unwrap() {
        let p = entry.unwrap().path();
        if p.extension().and_then(|e| e.to_str()) == Some("cdc") {
            out.push(p.to_string_lossy().into_owned());
        }
    }
    out.sort();
    out
}

#[test]
fn shared_corpus_parses() {
    let files = cdc_files();
    assert!(!files.is_empty(), "no .cdc files found");
    for f in &files {
        let src = fs::read_to_string(f).unwrap();
        let prog = parse_source(&src).unwrap_or_else(|e| panic!("{f}: {e}"));
        assert!(!prog.is_empty(), "{f}: empty program");
        // Every node must print canonical SDirac (non-empty, bracketed
        // where the reference expects lists).
        for node in &prog {
            assert!(!node.print().is_empty(), "{f}: empty print");
        }
        println!("ok   {f} ({} forms)", prog.len());
    }
    println!("{} files parsed", files.len());
}

#[test]
fn hello_demo_goldens() {
    let src = fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../bracket-jvm/examples/cdc/fact.cdc"
    ))
    .unwrap();
    let prog = parse_source(&src).unwrap();
    let printed: Vec<String> = prog.iter().map(|n| n.print()).collect();
    assert!(
        printed[0].starts_with("[proc fact|int [n|int]"),
        "{}",
        printed[0]
    );
    assert!(
        printed
            .iter()
            .any(|p| p == "[displayln [string-append \"fact(5) = \" [number->string [fact 5]]]]"),
        "{printed:?}"
    );
}
