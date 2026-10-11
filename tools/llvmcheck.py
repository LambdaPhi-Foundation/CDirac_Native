"""Differential gate: CDirac --emit-llvm + clang[++], run, byte-compare
against the reference tree-walker (`bracket` from ../bracket-jvm).

Usage (from E:\\cdirac):
    py tools/llvmcheck.py [--filter SUBSTR] [--keep-tmp]
        [--clang PATH] [--clangxx PATH] [--bracket PATH] [--cdirac PATH]
        [--mingw] [--mingw-gccroot PATH] [--native-dir DIR]
    CLANG=/path/to/clang CLANGXX=/path/to/clang++ BRACKET=... py tools/llvmcheck.py

--native-dir DIR (repeatable; $NATIVE_DIRS, pathsep-separated) additionally
gates DIR/*.cdc by golden stdout: each program is built (with -I DIR for
shim headers) and its stdout must equal <name>.expected beside the source
(rc must be 0). For native interop demos the tree-walker cannot serve as
reference (it rejects them), so goldens are the contract.

Windows without MSVC: scoop-style `clang` has no system headers, so
the single-driver link fails on `stdio.h`. `--mingw` (env
`CLANG_MINGW=1`) switches to a split flow proven against MinGW-w64
gcc: every input is compiled with
`clang --target=x86_64-w64-windows-gnu -I<mingw includes> -c` and the
objects are linked with `gcc`/`g++`:
    py tools/llvmcheck.py --mingw [--mingw-gccroot PATH]
`--mingw-gccroot` (env `MINGW_GCCROOT`) points at the gcc install root
(the dir containing `bin/`); default is auto-detected from `gcc` on
PATH by probing `<root>/<triplet>/include/stdio.h` for a mingw
triplet. MSVC/Linux behavior is unchanged.

Toolchain resolution per driver: explicit flag > $CLANG/$CLANGXX env >
`clang`/`clang++` on PATH > legacy `D:\\LLVM\\bin\\clang*.exe`.
If no driver is found the gate SKIPs (exit 0) instead of crashing with
FileNotFoundError — run with a toolchain installed for a real verdict.

For each .cdc program:
  - interop.cdc must FAIL at compile_llvm (JVM-only surface).
  - programs with native directives take the clang++ flow: IR + the
    --emit-cpp-shim output + llvmrt/*.c, linked in one driver call.
  - plain programs take the clang flow: IR + llvmrt/*.c.
  - the reference side runs the tree-walker over our --sdirac output:
    the bracket binary is SDirac-only, so this gates the canonical
    print contract AND codegen in one comparison.
  - stdin is empty (read-line at EOF yields false on both sides).
  - stdout AND exit code must match the tree-walker; on nonzero exit
    stderr must match too (both print `Error: ...`).
  - line endings are normalized (the Windows CRT translates \\n to
    \\r\\n; Rust does not): this gates semantics, not CRT behavior.

Timeouts guard runaway loops (15s per process).
"""
import argparse
import os
import shutil
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TIMEOUT = 15

RT_SRCS = [
    "value.c", "arith.c", "strings.c", "containers.c",
    "cinterop.c", "pred.c", "io.c", "mem.c",
]


def probe(exe):
    try:
        r = subprocess.run([exe, "--version"], capture_output=True, timeout=TIMEOUT)
        return r.returncode == 0
    except (FileNotFoundError, subprocess.TimeoutExpired, OSError):
        return False


def resolve_driver(explicit, env_key, prog):
    """First probing driver: explicit > env > PATH > legacy Windows path."""
    cands = []
    if explicit:
        cands.append(explicit)
    env_v = os.environ.get(env_key)
    if env_v:
        cands.append(env_v)
    found = shutil.which(prog)
    if found:
        cands.append(found)
    if os.name == "nt":
        legacy = r"D:\LLVM\bin\%s.exe" % prog
        if os.path.isfile(legacy):
            cands.append(legacy)
    for c in cands:
        if probe(c):
            return c
    return None


def run(cmd, cwd, stdin=b""):
    try:
        r = subprocess.run(
            cmd, cwd=cwd, input=stdin,
            capture_output=True, timeout=TIMEOUT,
        )
    except subprocess.TimeoutExpired:
        return None
    except FileNotFoundError:
        # P2: distinguish missing-toolchain crashes from real failures.
        r = None
        r_missing = True
        raise FileNotFoundError(cmd[0])
    return r


def emit(cdirac, flag, src):
    try:
        r = run([cdirac, flag, src], cwd=ROOT)
    except FileNotFoundError:
        print(f"missing cdirac binary: {cdirac}")
        sys.exit(2)
    if r is None or r.returncode != 0:
        return None
    return r.stdout


def norm(b):
    return b.replace(b"\r\n", b"\n")


def resolve_mingw(explicit_root):
    """Locate a MinGW-w64 gcc install for the --mingw split flow.

    Returns dict(target, includes, gcc, gxx) or None. Root is explicit
    --mingw-gccroot / $MINGW_GCCROOT, else derived from `gcc` on PATH.
    """
    import glob

    root = explicit_root or os.environ.get("MINGW_GCCROOT") or ""
    if not root:
        gcc = shutil.which("gcc")
        if not gcc:
            return None
        root = os.path.dirname(os.path.dirname(os.path.abspath(gcc)))
    triples = sorted(
        d for d in glob.glob(os.path.join(root, "*-mingw*"))
        if os.path.isfile(os.path.join(d, "include", "stdio.h"))
    )
    if not triples:
        return None
    triple_dir = triples[0]
    triple = os.path.basename(triple_dir)
    gcc_ver_dirs = sorted(glob.glob(os.path.join(root, "lib", "gcc", triple, "*", "include")))
    incs = [os.path.join(root, "include")]
    incs += gcc_ver_dirs
    incs.append(os.path.join(root, "lib", "gcc", triple, "*", "include-fixed"))
    incs.append(os.path.join(triple_dir, "include"))
    resolved = []
    for p in incs:
        if "*" in p:
            hits = glob.glob(p)
            if hits:
                resolved.append(hits[0])
        elif os.path.isdir(p):
            resolved.append(p)
    incs = resolved
    if not incs:
        return None
    gcc_exe = os.path.join(root, "bin", "gcc.exe" if os.name == "nt" else "gcc")
    gxx_exe = os.path.join(root, "bin", "g++.exe" if os.name == "nt" else "g++")
    if not (os.path.isfile(gcc_exe) and os.path.isfile(gxx_exe)):
        return None
    # C++ standard headers for the shim compile (libstdc++ layout).
    cxx_incs = []
    for d in sorted(glob.glob(os.path.join(root, "include", "c++", "*"))):
        if os.path.isdir(d):
            cxx_incs.append(d)
            for sub in (os.path.basename(triple_dir), "backward"):
                sd = os.path.join(d, sub)
                if os.path.isdir(sd):
                    cxx_incs.append(sd)
    return {
        "target": "x86_64-w64-windows-gnu",
        "includes": incs,
        "cxx_includes": cxx_incs,
        "gcc": gcc_exe,
        "gxx": gxx_exe,
    }


def main():
    ap = argparse.ArgumentParser(description="cdirac differential gate vs tree-walker")
    ap.add_argument("--filter", default="", help="only run programs whose path contains SUBSTR")
    ap.add_argument("--keep-tmp", action="store_true", help="keep temp dirs for debugging")
    ap.add_argument("--clang", default="", help="explicit clang driver")
    ap.add_argument("--clangxx", default="", help="explicit clang++ driver")
    ap.add_argument("--bracket", default="", help="explicit bracket binary")
    ap.add_argument("--cdirac", default="", help="explicit cdirac binary")
    ap.add_argument("--mingw", action="store_true",
                    help="MinGW split flow (clang -c + gcc link; needs gcc with mingw headers)")
    ap.add_argument("--mingw-gccroot", default="",
                    help="explicit MinGW gcc install root (or $MINGW_GCCROOT)")
    ap.add_argument("--native-dir", dest="native_dirs", action="append", default=[],
                    help="also gate DIR/*.cdc by golden stdout (repeatable; or $NATIVE_DIRS)")
    ap.add_argument("--bless", action="store_true",
                    help="write/refresh <name>.expected from actual output (verify by hand!)")
    args = ap.parse_args()

    cdirac = args.cdirac or os.path.join(ROOT, "target", "debug", "cdirac.exe")
    if os.name != "nt":
        alt = os.path.join(ROOT, "target", "debug", "cdirac")
        if not os.path.isfile(cdirac) and os.path.isfile(alt):
            cdirac = alt
    bracket = (
        args.bracket
        or os.environ.get("BRACKET")
        or os.path.join(ROOT, "..", "bracket-jvm", "target", "debug", "bracket.exe")
    )
    if os.name != "nt":
        alt = os.path.join(ROOT, "..", "bracket-jvm", "target", "debug", "bracket")
        if not os.path.isfile(bracket) and os.path.isfile(alt):
            bracket = alt
    clang = resolve_driver(args.clang or None, "CLANG", "clang")
    clangxx = resolve_driver(args.clangxx or None, "CLANGXX", "clang++")

    if not os.path.isfile(cdirac):
        print(f"build first: cargo build (missing {cdirac})")
        sys.exit(2)
    if not os.path.isfile(bracket):
        print(f"missing reference binary: {bracket}")
        sys.exit(2)
    if clang is None and clangxx is None:
        no_toolchain = True
        print("note: no clang/clang++ found "
              "(set CLANG/CLANGXX or --clang/--clangxx, need clang 15+); "
              "compile-only checks still run")
    else:
        no_toolchain = False

    mingw = None
    if args.mingw or os.environ.get("CLANG_MINGW"):
        mingw = resolve_mingw(args.mingw_gccroot or None)
        if mingw is None:
            print("missing MinGW gcc (set --mingw-gccroot or $MINGW_GCCROOT, "
                  "or put a MinGW-w64 gcc on PATH)")
            sys.exit(2)
        print(f"note: MinGW split flow via {mingw['gcc']} "
              f"(target {mingw['target']}, {len(mingw['includes'])} include dirs)")

    files = sorted(
        f for f in os.listdir(os.path.join(ROOT, "examples")) if f.endswith(".cdc")
    )
    files = [os.path.join(ROOT, "examples", f) for f in files]
    cdc_dir = os.path.join(ROOT, "..", "bracket-jvm", "examples", "cdc")
    if os.path.isdir(cdc_dir):
        files += sorted(
            os.path.join(cdc_dir, f)
            for f in os.listdir(cdc_dir) if f.endswith(".cdc")
        )
    # jobs: (src, goldendir|None). Diff jobs byte-compare against the
    # tree-walker; golden jobs (native interop: the tree-walker rejects
    # them) compare stdout against <name>.expected next to the source.
    jobs = [(f, None) for f in files]
    native_dirs = list(args.native_dirs)
    if os.environ.get("NATIVE_DIRS"):
        native_dirs += os.environ["NATIVE_DIRS"].split(os.pathsep)
    for ndir in native_dirs:
        ndir = ndir.strip()
        if not ndir:
            continue
        if not os.path.isabs(ndir):
            ndir = os.path.join(ROOT, ndir)
        if not os.path.isdir(ndir):
            print(f"missing native dir: {ndir}")
            sys.exit(2)
        for f in sorted(os.listdir(ndir)):
            if f.endswith(".cdc"):
                jobs.append((os.path.join(ndir, f), ndir))
    if args.filter:
        jobs = [(f, d) for (f, d) in jobs if args.filter in f]
    if not jobs:
        print("no .cdc programs found")
        sys.exit(2)

    failures = 0
    skipped_link = 0
    for src, goldendir in jobs:
        name = os.path.basename(src)
        tmp = tempfile.mkdtemp(prefix="llvmcheck-") if args.keep_tmp \
            else tempfile.TemporaryDirectory()
        tmpdir = tmp if isinstance(tmp, str) else tmp.name
        try:
            ll = emit(cdirac, "--emit-llvm", src)
            if ll is None:
                if name == "interop.cdc":
                    print(f"PASS {name} (JVM-only: compile rejected as designed)")
                    continue
                print(f"FAIL {name}: --emit-llvm rejected the program")
                failures += 1
                continue
            with open(os.path.join(tmpdir, "out.ll"), "wb") as f:
                f.write(ll)
            shim = emit(cdirac, "--emit-cpp-shim", src)
            if goldendir is not None and shim is None:
                print(f"FAIL {name}: native dir expects a shim for {name}")
                failures += 1
                continue
            if shim is not None:
                driver = clangxx
                if driver is None:
                    print(f"SKIP {name}: needs clang++ (none found)")
                    skipped_link += 1
                    continue
                with open(os.path.join(tmpdir, "cdi_shim.cpp"), "wb") as f:
                    f.write(shim)
                extra = [os.path.join(tmpdir, "cdi_shim.cpp"),
                         "-I", os.path.join(ROOT, "llvmrt")]
                if goldendir is not None:
                    # Headers live beside native demos (accum.h, consts.h...).
                    extra += ["-I", goldendir]
            else:
                driver = clang
                if driver is None:
                    print(f"SKIP {name}: needs clang (none found)")
                    skipped_link += 1
                    continue
                extra = []
            exe = os.path.join(tmpdir, "prog.exe" if os.name == "nt" else "prog")
            if mingw is not None:
                # Split flow: clang -c every input (gnu target + MinGW
                # headers), then link objects with gcc/g++.
                # C++ dirs first (g++ order): #include_next in libstdc++
                # headers resolves against the dirs that follow. They apply
                # to the shim only: libstdc++ ships <math.h>/<stdlib.h>
                # wrappers that break plain C compiles under clang.
                cxxflags = []
                if shim is not None:
                    for d in mingw["cxx_includes"]:
                        cxxflags += ["-I", d]
                cflags = []
                for d in mingw["includes"]:
                    cflags += ["-I", d]
                if goldendir is not None:
                    cflags += ["-I", goldendir]
                cflags = ["--target=" + mingw["target"]] + cflags
                linkdriver = mingw["gxx"] if shim is not None else mingw["gcc"]
                if shim is not None:
                    with open(os.path.join(tmpdir, "cdi_shim.cpp"), "wb") as f:
                        f.write(shim)
                inputs = [os.path.join(tmpdir, "out.ll")]
                inputs += [os.path.join(ROOT, "llvmrt", s) for s in RT_SRCS]
                if shim is not None:
                    inputs.append(os.path.join(tmpdir, "cdi_shim.cpp"))
                objs = []
                ok = True
                for i, src_in in enumerate(inputs):
                    obj = os.path.join(tmpdir, f"in{i}.o")
                    extra_incs = cxxflags if src_in.endswith(".cpp") else []
                    cmd = [driver, "-c", src_in] + extra_incs + cflags \
                        + ["-I", os.path.join(ROOT, "llvmrt"), "-o", obj]
                    try:
                        r = run(cmd, cwd=tmpdir)
                    except FileNotFoundError:
                        print(f"SKIP {name}: driver not runnable: {driver}")
                        skipped_link += 1
                        ok = False
                        break
                    if r is None or r.returncode != 0:
                        err = "" if r is None else r.stderr.decode("utf-8", "replace")[-2000:]
                        print(f"FAIL {name}: compile failed ({os.path.basename(src_in)}): {err}")
                        failures += 1
                        ok = False
                        break
                    objs.append(obj)
                if not ok:
                    continue
                cmd = [linkdriver] + objs + ["-o", exe]
                try:
                    r = run(cmd, cwd=tmpdir)
                except FileNotFoundError:
                    print(f"SKIP {name}: link driver not runnable: {linkdriver}")
                    skipped_link += 1
                    continue
                if r is None or r.returncode != 0:
                    err = "" if r is None else r.stderr.decode("utf-8", "replace")[-2000:]
                    print(f"FAIL {name}: link failed: {err}")
                    failures += 1
                    continue
            else:
                cmd = ([driver, os.path.join(tmpdir, "out.ll")]
                       + [os.path.join(ROOT, "llvmrt", s) for s in RT_SRCS]
                       + extra + ["-I", os.path.join(ROOT, "llvmrt"), "-o", exe])
                try:
                    r = run(cmd, cwd=tmpdir)
                except FileNotFoundError:
                    print(f"SKIP {name}: driver not runnable: {driver}")
                    skipped_link += 1
                    continue
                if r is None or r.returncode != 0:
                    err = "" if r is None else r.stderr.decode("utf-8", "replace")[-2000:]
                    print(f"FAIL {name}: link failed: {err}")
                    failures += 1
                    continue
            got = run([exe], cwd=tmpdir, stdin=b"")
            if goldendir is not None:
                # Golden gate: stdout must equal <name>.expected, rc 0.
                exp_path = os.path.splitext(src)[0] + ".expected"
                if got is None:
                    print(f"FAIL {name}: timeout")
                    failures += 1
                    continue
                if args.bless:
                    if got.returncode != 0:
                        print(f"FAIL {name}: rc {got.returncode}, not blessing")
                        failures += 1
                        continue
                    with open(exp_path, "wb") as f:
                        f.write(norm(got.stdout))
                    print(f"BLESSED {name}")
                    continue
                if not os.path.isfile(exp_path):
                    print(f"FAIL {name}: missing golden {exp_path}")
                    failures += 1
                    continue
                with open(exp_path, "rb") as f:
                    want_out = f.read()
                if got.returncode == 0 and norm(got.stdout) == norm(want_out):
                    print(f"PASS {name} (golden)")
                else:
                    failures += 1
                    print(f"FAIL {name}: rc {got.returncode}, stdout vs golden:")
                    print(f"  --- got ---\n{got.stdout.decode('utf-8', 'replace')[:1500]}")
                    print(f"  --- want ---\n{want_out.decode('utf-8', 'replace')[:1500]}")
                continue
            # Reference: our canonical SDirac through the SDirac-only
            # tree-walker (bracket binary has no CDirac frontend).
            sdirac = emit(cdirac, "--sdirac", src)
            if sdirac is None:
                print(f"FAIL {name}: --sdirac rejected the program")
                failures += 1
                continue
            with open(os.path.join(tmpdir, "prog.sdrc"), "wb") as f:
                f.write(sdirac)
            want = run([bracket, os.path.join(tmpdir, "prog.sdrc")], cwd=ROOT, stdin=b"")
            if got is None or want is None:
                print(f"FAIL {name}: timeout")
                failures += 1
                continue
            ok = norm(got.stdout) == norm(want.stdout) and got.returncode == want.returncode
            if ok and got.returncode != 0:
                ok = norm(got.stderr) == norm(want.stderr)
            if ok:
                print(f"PASS {name}")
            else:
                failures += 1
                print(f"FAIL {name}: rc {got.returncode} vs {want.returncode}")
                if norm(got.stdout) != norm(want.stdout):
                    print(f"  --- llvm stdout ---\n{got.stdout.decode('utf-8', 'replace')[:1500]}")
                    print(f"  --- tree stdout ---\n{want.stdout.decode('utf-8', 'replace')[:1500]}")
                if got.returncode != 0 and got.stderr != want.stderr:
                    print(f"  --- llvm stderr ---\n{got.stderr.decode('utf-8', 'replace')[:800]}")
                    print(f"  --- tree stderr ---\n{want.stderr.decode('utf-8', 'replace')[:800]}")
        finally:
            if not args.keep_tmp:
                tmp.cleanup()
            else:
                print(f"  (kept {tmpdir})")
    if failures:
        print(f"{failures} program(s) diverged")
        sys.exit(1)
    tail = f", {skipped_link} skipped (no toolchain)" if skipped_link else ""
    ngolden = sum(1 for (_, d) in jobs if d is not None)
    gtail = f" (incl. {ngolden} native goldens)" if ngolden else ""
    print(f"all {len(jobs)} programs byte-exact vs the tree-walker{gtail}{tail}")


if __name__ == "__main__":
    main()
