"""Compile the llvmrt C unit tests with a C11 compiler and run them.

Usage (from E:\\cdirac):
    py llvmrt/tests/run_tests.py [--filter SUBSTR]   # uses CC env or gcc/clang/cc on PATH
    CC=/path/to/gcc py llvmrt/tests/run_tests.py
    CC="clang -fwrapv" py llvmrt/tests/run_tests.py  # extra driver flags allowed

Each test is one assert-counting main; any failure or nonzero exit
fails the run. Mirrors bracket-jvm's jvmrt/tests/run_tests.py shape.
"""
import os
import shlex
import shutil
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "tests", "out")
TESTS = ["test_value", "test_arith", "test_strings", "test_cinterop", "test_n1"]
SOURCES = ["value.c", "arith.c", "strings.c", "containers.c", "cinterop.c", "pred.c", "io.c", "mem.c"]
EXE = ".exe" if os.name == "nt" else ""


def find_cc():
    cc = os.environ.get("CC")
    if cc:
        return shlex.split(cc)
    for prog in ("gcc", "clang", "cc"):
        found = shutil.which(prog)
        if found:
            return [found]
    print("no C compiler found (CC env or gcc/clang/cc on PATH)")
    sys.exit(2)


def main():
    filt = ""
    for a in sys.argv[1:]:
        if a.startswith("--filter="):
            filt = a[len("--filter="):]
        elif a in ("-h", "--help"):
            print(__doc__)
            return
        else:
            print(f"unknown arg {a}")
            sys.exit(2)
    cc = find_cc()
    tests = [t for t in TESTS if filt in t]
    if not tests:
        print("no tests match filter")
        sys.exit(2)
    os.makedirs(OUT, exist_ok=True)
    failures = 0
    for t in tests:
        srcs = [os.path.join(ROOT, s) for s in SOURCES]
        srcs.append(os.path.join(ROOT, "tests", t + ".c"))
        exe = os.path.join(OUT, t + EXE)
        cmd = cc + ["-std=c11", "-Wall", "-Wextra", "-fwrapv", "-o", exe] + srcs
        r = subprocess.run(
            cmd, capture_output=True, encoding="utf-8", errors="replace"
        )
        if r.returncode != 0:
            print(f"COMPILE-FAIL {t}:\n{r.stderr}")
            failures += 1
            continue
        r = subprocess.run(
            [exe], capture_output=True, encoding="utf-8", errors="replace"
        )
        print(f"--- {t} (rc={r.returncode}) ---")
        print(r.stdout, end="")
        if r.stderr:
            print(r.stderr, end="", file=sys.stderr)
        if r.returncode != 0:
            failures += 1
    if failures:
        print(f"{failures} test driver(s) failed")
        sys.exit(1)
    print("all llvmrt tests passed")


if __name__ == "__main__":
    main()
