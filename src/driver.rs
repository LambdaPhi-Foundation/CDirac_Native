//! driver: `cdirac build` / `cdirac run` — from `.cdc` to executable.
//!
//! The historical flow was manual (`--emit-llvm` + `--emit-cpp-shim` +
//! hand-written `clang++` line from the IR header comment). This module
//! automates exactly that flow with std-only code (no new dependencies):
//!
//! ```text
//! FILE.cdc --parse--> AST --compile_llvm--> out.ll -+-> clang/clang++ --> prog
//!                     --compile_cpp_shim--> shim.cpp -+   + llvmrt/*.c (+ link-native)
//! ```
//!
//! * Plain programs link with `clang` (C driver); programs with native
//!   directives link with `clang++` (the shim is C++17).
//! * GC stays opt-in: default `CDI_MALLOC` leak-by-design; `--gc-boehm`
//!   adds `-DCDI_GC_BOEHM -lgc` (needs Boehm installed).
//! * Toolchain probing order per driver: explicit flag > `$CLANG` /
//!   `$CLANGXX` env > `clang`/`clang++` on `PATH` > legacy
//!   `D:\LLVM\bin\clang*.exe`. Without a usable clang, `build` fails
//!   with a helpful message (gcc cannot compile `.ll`).
//! * `--mingw` (`$CLANG_MINGW`): Windows boxes without MSVC headers.
//!   Every input is compiled with `clang --target=x86_64-w64-windows-gnu
//!   -c` against auto-detected MinGW-w64 headers (from `gcc` on `PATH`,
//!   or `--mingw-gccroot` / `$MINGW_GCCROOT`), then objects are linked
//!   with `gcc`/`g++`. Same proven flow as `tools/llvmcheck.py --mingw`.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::llvmgen::{compile_cpp_shim, compile_llvm};
use crate::native::collect_native;
use crate::parser::parse_source;

/// Runtime C sources linked into every program (mirrors
/// `tools/llvmcheck.py RT_SRCS`).
pub const RT_SRCS: &[&str] = &[
    "value.c",
    "arith.c",
    "strings.c",
    "containers.c",
    "cinterop.c",
    "pred.c",
    "io.c",
    "mem.c",
];

/// Build configuration (populated from CLI flags / env).
#[derive(Clone, Debug, Default)]
pub struct BuildConfig {
    /// Explicit `--clang PATH` (C driver for plain programs).
    pub clang: Option<String>,
    /// Explicit `--clangxx PATH` (C++ driver for native programs).
    pub clangxx: Option<String>,
    /// `-o` output executable. Defaults to `<stem>` + `.exe` on Windows.
    pub output: Option<String>,
    /// `--gc-boehm`: link Boehm (`-DCDI_GC_BOEHM -lgc`) instead of
    /// leak-by-design malloc.
    pub gc_boehm: bool,
    /// `--verbose`: print the driver command before running it.
    pub verbose: bool,
    /// Extra `-I` include dirs (repeatable `--include DIR`).
    pub extra_includes: Vec<String>,
    /// Directory containing `llvmrt/*.c` (defaults to `<crate>/llvmrt`
    /// at build time via `CARGO_MANIFEST_DIR`, overridable for tests).
    pub llvmrt_dir: Option<String>,
    /// Keep temp files (`out.ll`, `cdi_shim.cpp`) next to the output
    /// instead of deleting them (`--keep-temp`).
    pub keep_temp: bool,
    /// `--mingw` (or `$CLANG_MINGW`): split flow for Windows without
    /// MSVC headers — `clang -c` per input (gnu target, MinGW headers)
    /// then link objects with `gcc`/`g++`.
    pub mingw: bool,
    /// Explicit `--mingw-gccroot PATH` (or `$MINGW_GCCROOT`): MinGW gcc
    /// install root (the dir containing `bin/`). Defaults to
    /// auto-detect from `gcc` on `PATH`.
    pub mingw_gccroot: Option<String>,
}

/// A fully resolved build: IR + optional shim + driver command.
#[derive(Clone, Debug)]
pub struct BuildPlan {
    /// LLVM IR text for `out.ll`.
    pub ll: String,
    /// C++ shim text, if the program has native directives.
    pub shim: Option<String>,
    /// Whether the C++ driver is required.
    pub needs_cxx: bool,
    /// `link-native` flags from directives.
    pub link_flags: Vec<String>,
    /// Resolved compiler driver executable.
    pub driver: String,
    /// argv (without the executable) to invoke.
    /// Placeholders `{LL}`, `{SHIM}`, `{OUT}`, `{RT...}` are expanded
    /// by [`build`] against real temp paths.
    pub argv_template: Vec<String>,
    /// MinGW split-flow details (only when `mingw` was requested).
    /// [`build`] then compiles each input to `.o` and links with
    /// `link_driver` instead of the single-step `argv_template`.
    pub mingw: Option<MingwPlan>,
}

/// Split-compile plan for `--mingw` (mirrors `tools/llvmcheck.py`).
#[derive(Clone, Debug)]
pub struct MingwPlan {
    /// `--target=` value for the `-c` steps.
    pub target: String,
    /// `-I` dirs for C inputs (MinGW + llvmrt handled separately).
    pub c_includes: Vec<String>,
    /// `-I` dirs added for the C++ shim only (libstdc++ wrappers
    /// break plain C compiles under clang).
    pub cxx_includes: Vec<String>,
    /// Link driver: `g++` for native programs, else `gcc`.
    pub link_driver: String,
}

/// A located MinGW-w64 gcc install (pure filesystem discovery).
#[derive(Clone, Debug)]
pub struct MingwToolchain {
    /// Install root (dir containing `bin/`).
    pub root: PathBuf,
    /// Vendor triplet dir name (e.g. `x86_64-w64-mingw32`).
    pub triple: String,
    /// C include dirs (existing ones only).
    pub c_includes: Vec<PathBuf>,
    /// C++ include dirs (existing ones only).
    pub cxx_includes: Vec<PathBuf>,
    /// Link drivers (existence-checked, not executed).
    pub gcc: PathBuf,
    pub gxx: PathBuf,
}

fn env_or(k: &str) -> Option<String> {
    std::env::var(k).ok().filter(|v| !v.trim().is_empty())
}

/// True if `exe --version` runs successfully (toolchain probe).
pub fn probe_tool(exe: &str) -> bool {
    Command::new(exe)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Candidate driver paths in priority order.
pub fn driver_candidates(explicit: Option<&str>, env_key: &str, names: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(e) = explicit {
        if !e.trim().is_empty() {
            out.push(e.to_string());
        }
    }
    if let Some(v) = env_or(env_key) {
        out.push(v);
    }
    for n in names {
        out.push(n.to_string());
    }
    // Legacy Windows install location (kept last for back-compat with
    // tools/llvmcheck.py defaults).
    if cfg!(windows) {
        if env_key == "CLANGXX" {
            out.push(r"D:\LLVM\bin\clang++.exe".to_string());
        } else {
            out.push(r"D:\LLVM\bin\clang.exe".to_string());
        }
    }
    out
}

/// Resolve the compiler driver. `needs_cxx` selects the C++ driver set.
pub fn resolve_driver(needs_cxx: bool, cfg: &BuildConfig) -> Result<String, String> {
    let cands = if needs_cxx {
        driver_candidates(cfg.clangxx.as_deref(), "CLANGXX", &["clang++"])
    } else {
        driver_candidates(cfg.clang.as_deref(), "CLANG", &["clang"])
    };
    for c in &cands {
        if probe_tool(c) {
            return Ok(c.clone());
        }
    }
    if needs_cxx {
        Err(format!(
            "no C++ driver found (tried {}). Install clang++ 15+ or set CLANGXX=/path/to/clang++ (or --clangxx).",
            cands.join(", ")
        ))
    } else {
        Err(format!(
            "no clang driver found (tried {}). Install clang 15+ or set CLANG=/path/to/clang (or --clang). gcc cannot compile LLVM IR (.ll).",
            cands.join(", ")
        ))
    }
}

/// `true` when `--mingw` was requested (flag or `$CLANG_MINGW`).
pub fn want_mingw(cfg: &BuildConfig) -> bool {
    cfg.mingw || env_or("CLANG_MINGW").is_some()
}

/// Minimal `which`: first `name` on `PATH` (with `PATHEXT` on Windows).
fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let exts: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".EXE".to_string())
            .split(';')
            .map(|e| e.to_string())
            .collect()
    } else {
        vec![String::new()]
    };
    for dir in std::env::split_paths(&path) {
        if dir.as_os_str().is_empty() {
            continue;
        }
        for ext in &exts {
            let p = dir.join(format!("{name}{ext}"));
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// Candidate MinGW gcc root: explicit flag > `$MINGW_GCCROOT` >
/// derived from `gcc`/`g++` on `PATH` (parent of parent of the exe).
fn mingw_root_candidate(explicit: Option<&str>) -> Option<PathBuf> {
    if let Some(e) = explicit.filter(|s| !s.trim().is_empty()) {
        return Some(PathBuf::from(e));
    }
    if let Some(v) = env_or("MINGW_GCCROOT") {
        return Some(PathBuf::from(v));
    }
    let exe = which("gcc").or_else(|| which("g++"))?;
    exe.parent()?.parent().map(Path::to_path_buf)
}

/// Locate + validate a MinGW-w64 gcc install. Pure filesystem probing
/// (no subprocess) — unit-testable with a fabricated tree.
pub fn resolve_mingw(cfg: &BuildConfig) -> Result<Option<MingwToolchain>, String> {
    if !want_mingw(cfg) {
        return Ok(None);
    }
    let root = mingw_root_candidate(cfg.mingw_gccroot.as_deref()).ok_or_else(|| {
        "mingw requested but no gcc root found (set --mingw-gccroot PATH or $MINGW_GCCROOT, or put a MinGW-w64 gcc on PATH)".to_string()
    })?;
    if !root.is_dir() {
        return Err(format!(
            "mingw gcc root not a directory: {}",
            root.display()
        ));
    }
    // Vendor triplet holding the system headers.
    let mut triples: Vec<(String, PathBuf)> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&root) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.contains("mingw"))
                    .unwrap_or(false)
                && p.join("include").join("stdio.h").is_file()
            {
                triples.push((p.file_name().unwrap().to_string_lossy().into_owned(), p));
            }
        }
    }
    triples.sort();
    let (triple, triple_dir) = triples.into_iter().next().ok_or_else(|| {
        format!(
            "no MinGW triplet headers under {} (want <triplet>/include/stdio.h)",
            root.display()
        )
    })?;
    let mut c_includes = vec![root.join("include")];
    // gcc private headers: lib/gcc/<triplet>/<ver>/include[+-fixed].
    if let Ok(rd) = std::fs::read_dir(root.join("lib").join("gcc").join(&triple)) {
        let mut vers: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        vers.sort();
        for v in vers {
            c_includes.push(v.join("include"));
            c_includes.push(v.join("include-fixed"));
        }
    }
    c_includes.push(triple_dir.join("include"));
    let c_includes: Vec<PathBuf> = c_includes.into_iter().filter(|p| p.is_dir()).collect();
    // libstdc++ headers for the shim: include/c++/<ver> (+ arch/backward).
    let mut cxx_includes = Vec::new();
    if let Ok(rd) = std::fs::read_dir(root.join("include").join("c++")) {
        let mut vers: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        vers.sort();
        for v in vers {
            cxx_includes.push(v.clone());
            let arch = v.join(&triple);
            if arch.is_dir() {
                cxx_includes.push(arch);
            }
            let back = v.join("backward");
            if back.is_dir() {
                cxx_includes.push(back);
            }
        }
    }
    let exe = |n: &str| root.join("bin").join(n);
    let (gcc, gxx) = if cfg!(windows) {
        (exe("gcc.exe"), exe("g++.exe"))
    } else {
        (exe("gcc"), exe("g++"))
    };
    if !gcc.is_file() {
        return Err(format!("mingw gcc driver missing: {}", gcc.display()));
    }
    if !gxx.is_file() {
        return Err(format!("mingw g++ driver missing: {}", gxx.display()));
    }
    Ok(Some(MingwToolchain {
        root,
        triple,
        c_includes,
        cxx_includes,
        gcc,
        gxx,
    }))
}
pub fn llvmrt_dir(cfg: &BuildConfig) -> PathBuf {
    if let Some(d) = cfg.llvmrt_dir.as_ref().filter(|s| !s.trim().is_empty()) {
        return PathBuf::from(d);
    }
    if let Some(v) = env_or("CDIRAC_LLVMRT") {
        return PathBuf::from(v);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("llvmrt")
}

/// Default output path: `<stem>` in cwd (+ `.exe` on Windows).
pub fn default_output(src: &Path) -> PathBuf {
    let stem = src
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "prog".to_string());
    if cfg!(windows) {
        PathBuf::from(format!("{stem}.exe"))
    } else {
        PathBuf::from(stem)
    }
}

/// Parse + lower + resolve driver, without invoking the compiler.
/// Pure (no fs writes, one probe per driver) — unit-testable without clang.
pub fn plan(src_path: &Path, src_text: &str, cfg: &BuildConfig) -> Result<BuildPlan, String> {
    let prog = parse_source(src_text).map_err(|e| format!("{}: {e}", src_path.display()))?;
    let native = collect_native(&prog)?;
    let ll = compile_llvm(&prog)?;
    let shim = compile_cpp_shim(&prog).ok();
    let needs_cxx = shim.is_some();
    let driver = resolve_driver(needs_cxx, cfg)?;
    let rt = llvmrt_dir(cfg);
    let mut argv: Vec<String> = vec!["{LL}".to_string()];
    for s in RT_SRCS {
        argv.push(rt.join(s).to_string_lossy().into_owned());
    }
    if shim.is_some() {
        argv.push("{SHIM}".to_string());
        argv.push("-I".to_string());
        argv.push(rt.to_string_lossy().into_owned());
    }
    argv.push("-I".to_string());
    argv.push(rt.to_string_lossy().into_owned());
    for inc in &cfg.extra_includes {
        argv.push("-I".to_string());
        argv.push(inc.clone());
    }
    if cfg.gc_boehm {
        argv.push("-DCDI_GC_BOEHM".to_string());
    }
    argv.extend(native.link_flags.iter().cloned());
    if cfg.gc_boehm {
        argv.push("-lgc".to_string());
    }
    argv.push("-o".to_string());
    argv.push("{OUT}".to_string());
    let mingw = match resolve_mingw(cfg)? {
        Some(tc) => {
            let s = |p: &PathBuf| p.to_string_lossy().into_owned();
            Some(MingwPlan {
                target: "x86_64-w64-windows-gnu".to_string(),
                c_includes: tc.c_includes.iter().map(s).collect(),
                cxx_includes: tc.cxx_includes.iter().map(s).collect(),
                link_driver: if needs_cxx {
                    tc.gxx.to_string_lossy().into_owned()
                } else {
                    tc.gcc.to_string_lossy().into_owned()
                },
            })
        }
        None => None,
    };
    Ok(BuildPlan {
        ll,
        shim,
        needs_cxx,
        link_flags: native.link_flags.clone(),
        driver,
        argv_template: argv,
        mingw,
    })
}

fn unique_tmpdir(tag: &str) -> Result<PathBuf, String> {
    let base = std::env::temp_dir();
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = base.join(format!("cdirac-{tag}-{pid}-{nanos}"));
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create temp dir: {e}"))?;
    Ok(dir)
}

/// Execute a MinGW split-flow plan: `clang -c` every input (gnu
/// target, MinGW headers), then link objects with `gcc`/`g++`.
#[allow(clippy::too_many_arguments)]
fn build_split(
    p: &BuildPlan,
    mp: &MingwPlan,
    cfg: &BuildConfig,
    workdir: &Path,
    ll_path: &Path,
    shim_path: &Path,
    out: &Path,
    keep: bool,
) -> Result<PathBuf, String> {
    let rt = llvmrt_dir(cfg);
    // (source, is_shim): the shim alone gets the C++ include dirs.
    let mut inputs: Vec<(PathBuf, bool)> = vec![(ll_path.to_path_buf(), false)];
    for s in RT_SRCS {
        inputs.push((rt.join(s), false));
    }
    if p.shim.is_some() {
        inputs.push((shim_path.to_path_buf(), true));
    }
    let mut objs: Vec<PathBuf> = Vec::new();
    // -D flags ride the compile steps; link flags ride the link step.
    let mut defines: Vec<String> = Vec::new();
    for inc in &cfg.extra_includes {
        defines.push("-I".to_string());
        defines.push(inc.clone());
    }
    defines.push("-I".to_string());
    defines.push(rt.to_string_lossy().into_owned());
    if cfg.gc_boehm {
        defines.push("-DCDI_GC_BOEHM".to_string());
    }
    for (i, (src, is_shim)) in inputs.iter().enumerate() {
        let obj = workdir.join(format!("in{i}.o"));
        let mut argv = vec![
            format!("--target={}", mp.target),
            "-c".to_string(),
            src.to_string_lossy().into_owned(),
        ];
        // Preprocessor flags only matter for C/C++ inputs; the `.ll`
        // needs none (clang warns they are unused).
        if src.extension().and_then(|e| e.to_str()) != Some("ll") {
            if *is_shim {
                for d in &mp.cxx_includes {
                    argv.push("-I".to_string());
                    argv.push(d.clone());
                }
            }
            for d in &mp.c_includes {
                argv.push("-I".to_string());
                argv.push(d.clone());
            }
            argv.extend(defines.clone());
        }
        argv.push("-o".to_string());
        argv.push(obj.to_string_lossy().into_owned());
        if cfg.verbose {
            eprintln!("{} {}", p.driver, argv.join(" "));
        }
        let st = Command::new(&p.driver)
            .args(&argv)
            .status()
            .map_err(|e| format!("cannot run '{}': {e}", p.driver))?;
        if !st.success() {
            return Err(format!(
                "compile failed ({}): {} exited with {}",
                src.display(),
                p.driver,
                st.code()
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "?".to_string())
            ));
        }
        objs.push(obj);
    }
    let mut link_argv: Vec<String> = objs
        .iter()
        .map(|o| o.to_string_lossy().into_owned())
        .collect();
    link_argv.extend(p.link_flags.iter().cloned());
    if cfg.gc_boehm {
        link_argv.push("-lgc".to_string());
    }
    link_argv.push("-o".to_string());
    link_argv.push(out.to_string_lossy().into_owned());
    if cfg.verbose {
        eprintln!("{} {}", mp.link_driver, link_argv.join(" "));
    }
    let st = Command::new(&mp.link_driver)
        .args(&link_argv)
        .status()
        .map_err(|e| format!("cannot run '{}': {e}", mp.link_driver))?;
    if !keep {
        let _ = std::fs::remove_file(ll_path);
        let _ = std::fs::remove_file(shim_path);
        for o in &objs {
            let _ = std::fs::remove_file(o);
        }
        let _ = std::fs::remove_dir(workdir);
    }
    if !st.success() {
        return Err(format!(
            "link failed: {} exited with {}",
            mp.link_driver,
            st.code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "?".to_string())
        ));
    }
    Ok(out.to_path_buf())
}
/// Execute a build plan: write temp files, invoke the driver, place output.
pub fn build(src_path: &Path, cfg: &BuildConfig) -> Result<PathBuf, String> {
    let src_text = std::fs::read_to_string(src_path)
        .map_err(|_| format!("cdirac: file not found: {}", src_path.display()))?;
    let p = plan(src_path, &src_text, cfg)?;
    let out = match &cfg.output {
        Some(o) => PathBuf::from(o),
        None => default_output(src_path),
    };
    let keep = cfg.keep_temp;
    let workdir = if keep {
        out.parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    } else {
        unique_tmpdir("build")?
    };
    if keep {
        std::fs::create_dir_all(&workdir).map_err(|e| format!("cannot create dir: {e}"))?;
    }
    let ll_path = workdir.join("out.ll");
    std::fs::write(&ll_path, &p.ll).map_err(|e| format!("cannot write out.ll: {e}"))?;
    let shim_path = workdir.join("cdi_shim.cpp");
    if let Some(shim) = &p.shim {
        std::fs::write(&shim_path, shim).map_err(|e| format!("cannot write shim: {e}"))?;
    }
    let expand = |t: &str| -> String {
        match t {
            "{LL}" => ll_path.to_string_lossy().into_owned(),
            "{SHIM}" => shim_path.to_string_lossy().into_owned(),
            "{OUT}" => out.to_string_lossy().into_owned(),
            _ => t.to_string(),
        }
    };
    let argv: Vec<String> = p.argv_template.iter().map(|s| expand(s)).collect();
    if cfg.verbose {
        eprintln!("{} {}", p.driver, argv.join(" "));
    }
    if let Some(mp) = &p.mingw {
        return build_split(&p, mp, cfg, &workdir, &ll_path, &shim_path, &out, keep);
    }
    let status = Command::new(&p.driver)
        .args(&argv)
        .status()
        .map_err(|e| format!("cannot run '{}': {e}", p.driver))?;
    if !keep {
        let _ = std::fs::remove_file(&ll_path);
        let _ = std::fs::remove_file(&shim_path);
        let _ = std::fs::remove_dir(&workdir);
    }
    if !status.success() {
        return Err(format!(
            "link failed: {} exited with {}",
            p.driver,
            status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "?".to_string())
        ));
    }
    Ok(out)
}

/// Build to temp + run with inherited stdio. Returns the exit code.
pub fn run(src_path: &Path, cfg: &BuildConfig, prog_args: &[String]) -> Result<i32, String> {
    let dir = unique_tmpdir("run")?;
    let exe = dir.join(if cfg!(windows) { "prog.exe" } else { "prog" });
    let mut bcfg = cfg.clone();
    bcfg.output = Some(exe.to_string_lossy().into_owned());
    bcfg.keep_temp = false;
    let built = build(src_path, &bcfg)?;
    let status = Command::new(&built)
        .args(prog_args)
        .status()
        .map_err(|e| format!("cannot run '{}': {e}", built.display()))?;
    let _ = std::fs::remove_file(&built);
    let _ = std::fs::remove_dir(&dir);
    Ok(status.code().unwrap_or(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_no_probe() -> BuildConfig {
        BuildConfig {
            clang: Some("definitely-not-a-real-clang-xyz".to_string()),
            clangxx: Some("definitely-not-a-real-clangxx-xyz".to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn candidates_prefer_explicit_then_env() {
        std::env::set_var("CDIRAC_DRIVER_TEST_CLANG", "env-clang");
        let c = driver_candidates(
            Some("explicit-clang"),
            "CDIRAC_DRIVER_TEST_CLANG",
            &["clang"],
        );
        assert_eq!(&c[0], "explicit-clang");
        assert_eq!(&c[1], "env-clang");
        assert!(c.contains(&"clang".to_string()));
        std::env::remove_var("CDIRAC_DRIVER_TEST_CLANG");
    }

    #[test]
    fn resolve_fails_helpfully_without_toolchain() {
        let cfg = cfg_no_probe();
        let e = resolve_driver(false, &cfg).unwrap_err();
        assert!(e.contains("CLANG"), "{e}");
        let e = resolve_driver(true, &cfg).unwrap_err();
        assert!(e.contains("CLANGXX"), "{e}");
    }

    #[test]
    fn plan_plain_needs_c_driver() {
        // `cargo --version` succeeds, so it stands in for a present clang
        // (probing only checks `--version` exit status; no compile runs here).
        let cfg = BuildConfig {
            clang: Some("cargo".to_string()),
            ..Default::default()
        };
        let p = plan(Path::new("hello.cdc"), "displayln(1);\n", &cfg).unwrap();
        assert!(!p.needs_cxx);
        assert!(p.shim.is_none());
        assert!(!p.argv_template.iter().any(|a| a == "{SHIM}"));
    }

    #[test]
    fn plan_native_needs_cxx_and_link_flags() {
        let src = "[include-cpp \"v.h\"]\n[link-native \"-lm\"]\n[extern-cpp f \"ns::f\" [int] int]\ndisplayln(f(1));\n";
        let cfg = BuildConfig {
            clangxx: Some("definitely-not-a-real-clangxx-xyz".to_string()),
            ..Default::default()
        };
        // resolve fails, but collect/compile parts must succeed first:
        // assert on the underlying pieces directly.
        let prog = parse_source(src).unwrap();
        let native = collect_native(&prog).unwrap();
        assert_eq!(native.link_flags, vec!["-lm".to_string()]);
        assert!(compile_llvm(&prog).is_ok());
        assert!(compile_cpp_shim(&prog).is_ok());
        assert!(plan(Path::new("n.cdc"), src, &cfg)
            .unwrap_err()
            .contains("CLANGXX"));
    }

    #[test]
    fn default_output_names() {
        let o = default_output(Path::new("examples/hello.cdc"));
        if cfg!(windows) {
            assert_eq!(o, PathBuf::from("hello.exe"));
        } else {
            assert_eq!(o, PathBuf::from("hello"));
        }
    }

    /// Fabricated MinGW gcc tree (existence-only; nothing is executed).
    fn fake_gccroot() -> PathBuf {
        let root = unique_tmpdir("mingwtest").unwrap();
        let triple = root.join("x86_64-w64-mingw32-test");
        std::fs::create_dir_all(triple.join("include")).unwrap();
        std::fs::write(triple.join("include").join("stdio.h"), "").unwrap();
        std::fs::create_dir_all(root.join("include")).unwrap();
        let cxx = root.join("include").join("c++").join("v1");
        std::fs::create_dir_all(cxx.join("x86_64-w64-mingw32-test")).unwrap();
        std::fs::create_dir_all(cxx.join("backward")).unwrap();
        std::fs::create_dir_all(root.join("bin")).unwrap();
        let (gcc, gxx) = if cfg!(windows) {
            ("gcc.exe", "g++.exe")
        } else {
            ("gcc", "g++")
        };
        std::fs::write(root.join("bin").join(gcc), "").unwrap();
        std::fs::write(root.join("bin").join(gxx), "").unwrap();
        root
    }

    #[test]
    fn mingw_not_requested_is_none() {
        // No flag, no env: plain single-step plans carry no split flow.
        // (CLANG_MINGW is never set by other tests; guard anyway.)
        std::env::remove_var("CLANG_MINGW");
        let cfg = BuildConfig::default();
        assert!(resolve_mingw(&cfg).unwrap().is_none());
    }

    #[test]
    fn mingw_discovery_from_explicit_root() {
        let root = fake_gccroot();
        let cfg = BuildConfig {
            mingw: true,
            mingw_gccroot: Some(root.to_string_lossy().into_owned()),
            ..Default::default()
        };
        let tc = resolve_mingw(&cfg).unwrap().expect("toolchain");
        assert_eq!(tc.triple, "x86_64-w64-mingw32-test");
        assert!(!tc.c_includes.is_empty());
        assert!(!tc.cxx_includes.is_empty());
        assert!(tc.gcc.is_file() && tc.gxx.is_file());
    }

    #[test]
    fn mingw_missing_root_errors_helpfully() {
        let cfg = BuildConfig {
            mingw: true,
            mingw_gccroot: Some(
                PathBuf::from("definitely-not-a-real-gccroot-xyz")
                    .to_string_lossy()
                    .into_owned(),
            ),
            ..Default::default()
        };
        let e = resolve_mingw(&cfg).unwrap_err();
        assert!(e.contains("not a directory"), "{e}");
    }

    #[test]
    fn plan_mingw_split_shape_plain() {
        let root = fake_gccroot();
        let cfg = BuildConfig {
            mingw: true,
            mingw_gccroot: Some(root.to_string_lossy().into_owned()),
            clang: Some("cargo".to_string()),
            ..Default::default()
        };
        let p = plan(Path::new("hello.cdc"), "displayln(1);\n", &cfg).unwrap();
        assert!(!p.needs_cxx);
        let mp = p.mingw.expect("split plan");
        assert_eq!(mp.target, "x86_64-w64-windows-gnu");
        assert!(mp
            .link_driver
            .ends_with(if cfg!(windows) { "gcc.exe" } else { "gcc" }));
        // Default single-step template stays intact alongside.
        assert!(p.argv_template.iter().any(|a| a == "{LL}"));
    }

    #[test]
    fn plan_mingw_split_shape_native() {
        let root = fake_gccroot();
        let src = "[include-cpp \"v.h\"]\n[extern-cpp f \"ns::f\" [int] int]\ndisplayln(f(1));\n";
        let cfg = BuildConfig {
            mingw: true,
            mingw_gccroot: Some(root.to_string_lossy().into_owned()),
            clangxx: Some("cargo".to_string()),
            ..Default::default()
        };
        let p = plan(Path::new("n.cdc"), src, &cfg).unwrap();
        assert!(p.needs_cxx);
        assert!(p.shim.is_some());
        let mp = p.mingw.expect("split plan");
        assert!(mp
            .link_driver
            .ends_with(if cfg!(windows) { "g++.exe" } else { "g++" }));
        assert!(!mp.cxx_includes.is_empty());
    }
}
