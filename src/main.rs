//! `cdirac` binary: parse, emit, build, and run CDirac sources.
//!
//! * `parse` / `--sdirac` / `--emit-llvm` / `--emit-cpp-shim`: inspect
//!   the frontend and backend outputs (no toolchain needed).
//! * `build FILE.cdc [-o OUT] [--clang …] [--clangxx …] [--gc-boehm]
//!   [--include DIR]... [--verbose] [--keep-temp]`: compile + link via
//!   `clang` (plain) or `clang++` (native) + `llvmrt/*.c`.
//! * `run FILE.cdc [-- args…]`: build to temp and execute.

use cdirac::driver::{build, run, BuildConfig};
use cdirac::llvmgen::{compile_cpp_shim, compile_llvm};
use cdirac::parser::parse_source;

const VERSION: &str = "0.1.0";

fn usage() -> String {
    format!(
        "cdirac v{VERSION} — CDirac frontend (lexer/parser/AST) + LLVM backend\n\
         \n\
         Usage:\n\
         \x20 cdirac parse FILE.cdc         print the AST with spans\n\
         \x20 cdirac --sdirac FILE.cdc      print canonical SDirac (reference spelling)\n\
         \x20 cdirac --emit-llvm FILE.cdc   print LLVM IR (needs clang + llvmrt/*.c to run)\n\
         \x20 cdirac --emit-cpp-shim FILE.cdc  print the C++ interop shim (needs g++ -std=c++17)\n\
         \x20 cdirac build FILE.cdc [-o OUT] [--clang PATH] [--clangxx PATH]\n\
         \x20 \x20 \x20 \x20 \x20 \x20 [--gc-boehm] [--include DIR]... [--verbose] [--keep-temp]\n\
         \x20 \x20 \x20 \x20 \x20 \x20 [--mingw] [--mingw-gccroot PATH]\n\
         \x20 \x20 \x20 \x20 \x20 \x20 \x20 \x20 \x20 \x20 \x20 \x20 \x20 \x20 compile + link to an executable\n\
         \x20 cdirac run FILE.cdc [-- PROGARGS...]   build to temp and execute\n\
         \x20 \x20 (build/run flags also accepted before FILE; $CLANG/$CLANGXX env honored)\n\
         \x20 cdirac -h | --help            show this help"
    )
}

fn read_file(path: &str) -> String {
    match std::fs::read_to_string(path) {
        Err(_) => {
            eprintln!("cdirac: file not found: {path}");
            std::process::exit(1);
        }
        Ok(s) => s,
    }
}

fn die(msg: &str) -> ! {
    eprintln!("Error: {msg}");
    std::process::exit(1);
}

/// Parse `build`/`run` flags. Returns (config, src, prog_args).
fn parse_build_args(cmd: &str, rest: &[String]) -> (BuildConfig, String, Vec<String>) {
    let mut cfg = BuildConfig::default();
    let mut src: Option<String> = None;
    let mut prog_args: Vec<String> = Vec::new();
    let mut i = 0;
    let mut passthrough = false;
    while i < rest.len() {
        let a = &rest[i];
        if passthrough {
            prog_args.push(a.clone());
        } else if a == "--" {
            if cmd == "run" {
                passthrough = true;
            } else {
                die("build takes no positional prog args (did you mean `run`?)");
            }
        } else if a == "-o" || a == "--output" {
            i += 1;
            if i >= rest.len() {
                die("-o needs a value");
            }
            cfg.output = Some(rest[i].clone());
        } else if let Some(v) = a.strip_prefix("--clangxx=") {
            cfg.clangxx = Some(v.to_string());
        } else if a == "--clangxx" {
            i += 1;
            if i >= rest.len() {
                die("--clangxx needs a value");
            }
            cfg.clangxx = Some(rest[i].clone());
        } else if let Some(v) = a.strip_prefix("--clang=") {
            cfg.clang = Some(v.to_string());
        } else if a == "--clang" {
            i += 1;
            if i >= rest.len() {
                die("--clang needs a value");
            }
            cfg.clang = Some(rest[i].clone());
        } else if let Some(v) = a.strip_prefix("--include=") {
            cfg.extra_includes.push(v.to_string());
        } else if a == "--include" || a == "-I" {
            i += 1;
            if i >= rest.len() {
                die("--include needs a value");
            }
            cfg.extra_includes.push(rest[i].clone());
        } else if a == "--gc-boehm" {
            cfg.gc_boehm = true;
        } else if a == "--mingw" {
            cfg.mingw = true;
        } else if let Some(v) = a.strip_prefix("--mingw-gccroot=") {
            cfg.mingw_gccroot = Some(v.to_string());
        } else if a == "--mingw-gccroot" {
            i += 1;
            if i >= rest.len() {
                die("--mingw-gccroot needs a value");
            }
            cfg.mingw_gccroot = Some(rest[i].clone());
        } else if a == "--verbose" || a == "-v" {
            cfg.verbose = true;
        } else if a == "--keep-temp" {
            cfg.keep_temp = true;
        } else if a.starts_with('-') {
            die(&format!("unknown flag '{a}' (see --help)"));
        } else if src.is_none() {
            src = Some(a.clone());
        } else if cmd == "run" {
            // Bare words after FILE.cdc are prog args (also accepts `--`).
            prog_args.push(a.clone());
        } else {
            die(&format!("unexpected argument '{a}'"));
        }
        i += 1;
    }
    match src {
        Some(s) => (cfg, s, prog_args),
        None => die(&format!("{cmd} needs FILE.cdc")),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() == 1 && (args[0] == "-h" || args[0] == "--help") {
        println!("{}", usage());
        return;
    }
    if args.len() == 2 && (args[0] == "parse" || args[0] == "--sdirac") {
        let sdirac = args[0] == "--sdirac";
        let src = read_file(&args[1]);
        match parse_source(&src) {
            Ok(prog) => {
                for n in &prog {
                    if sdirac {
                        println!("{}", n.print());
                    } else {
                        println!("{n:?}");
                    }
                }
            }
            Err(e) => die(&e),
        }
        return;
    }
    if args.len() == 2 && args[0] == "--emit-llvm" {
        let src = read_file(&args[1]);
        let prog = parse_source(&src).unwrap_or_else(|e| die(&e));
        match compile_llvm(&prog) {
            Ok(ll) => print!("{ll}"),
            Err(e) => die(&e),
        }
        return;
    }
    if args.len() == 2 && args[0] == "--emit-cpp-shim" {
        let src = read_file(&args[1]);
        let prog = parse_source(&src).unwrap_or_else(|e| die(&e));
        match compile_cpp_shim(&prog) {
            Ok(shim) => print!("{shim}"),
            Err(e) => die(&e),
        }
        return;
    }
    if !args.is_empty() && (args[0] == "build" || args[0] == "run") {
        let cmd = args[0].clone();
        let (cfg, src, prog_args) = parse_build_args(&cmd, &args[1..]);
        let path = std::path::Path::new(&src);
        if cmd == "build" {
            match build(path, &cfg) {
                Ok(out) => {
                    if cfg.verbose {
                        eprintln!("built {}", out.display());
                    } else {
                        println!("{}", out.display());
                    }
                }
                Err(e) => die(&e),
            }
        } else {
            match run(path, &cfg, &prog_args) {
                Ok(code) => std::process::exit(code),
                Err(e) => die(&e),
            }
        }
        return;
    }
    eprintln!("{}", usage());
    std::process::exit(1);
}
