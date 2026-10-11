# CDirac — CDirac-native LLVM compiler

A self-contained Cargo project (separate workspace from
`E:\bracket-jvm`). It implements the CDirac surface — lexer, parser,
AST — plus the LLVM backend that compiles it to machine code via
`clang`. Dirac stays what it is: one language (AST + semantics), two
frontends (SDirac `[...]`, CDirac C-style), several backends (tree,
JVM, LLVM, …). This crate is not a fork of the language.

```c
fn fact(n|int) | int {
  if (n <= 1) { return 1; }
  return n * fact(n - 1);
}
displayln(fact(5));
```

## Layout

```text
src/lib.rs      # crate root (ast | closures | driver | lexer | native | parser | llvmgen)
src/ast.rs      # Span, TypeExpr, span-carrying Node + canonical SDirac print
src/lexer.rs    # C tokens with spans + migration errors
src/parser.rs   # full CDirac grammar -> AST (surface desugars included)
src/closures.rs # static capture analysis for nested procedures
src/driver.rs   # `build`/`run`: IR + shim + clang/clang++ orchestration
src/native.rs   # C/C++ interop decls (C++-first) + generated-shim model
src/llvmgen.rs  # AST -> LLVM IR text (full core + native wrappers + closure lift)
src/main.rs     # `cdirac` binary: parse + inspect + emit + build + run
llvmrt/         # C runtime (value/arith/pred/strings/containers/cinterop/io/mem)
tests/corpus.rs # compatibility gate over ../bracket-jvm/examples/cdc/
tests/native_cpp.rs # C++-first interop gate (IR + shim shapes + errors)
tests/n1.rs     # full-core gate (dispatch table + lazy forms + errors)
tests/closures.rs # nested-proc gate (lift shapes + capture errors)
tests/filetest.rs # file-based gate over filetest/ok + filetest/err
tools/llvmcheck.py # differential gate: clang[++]-built binaries vs tree-walker
examples/       # local demos (hello.cdc, cpp/*, closures/demo.cdc)
filetest/       # file-based checks (ok/*.cdc must pass, err/*.cdc must fail as declared)
docs/CPP-INTEROP.md # C++-first native interop reference
docs/CLOSURES.md    # nested-procedure (closure) semantics
docs/ERRORS.md      # static-error catalog
docs/PHASES.md      # build log for this completion pass
```

## Commands

```powershell
cargo build            # E:\cdirac
cargo test             # unit + corpus + native + N1 + driver gates (62)
py llvmrt/tests/run_tests.py  # C runtime tests (needs gcc/clang on PATH)
py tools/llvmcheck.py  # differential gate vs tree-walker (needs clang 15+, bracket checkout)
# Windows without MSVC: py tools/llvmcheck.py --mingw [--native-dir examples/cpp]
#   (split flow: clang -c gnu-target + gcc link; goldens in <name>.expected)
.\target\debug\cdirac.exe parse examples\hello.cdc
.\target\debug\cdirac.exe --sdirac examples\hello.cdc
.\target\debug\cdirac.exe --emit-llvm examples\hello.cdc
.\target\debug\cdirac.exe --emit-llvm examples\cpp\accum.cdc      # IR + clang++ link line
.\target\debug\cdirac.exe --emit-cpp-shim examples\cpp\accum.cdc # C++ shim (g++ -std=c++17 -c)
.\target\debug\cdirac.exe build examples\hello.cdc -o hello.exe  # compile+link (clang; clang++ if native)
.\target\debug\cdirac.exe run examples\hello.cdc                 # build to temp and execute
# build flags: --clang PATH / --clangxx PATH (or $CLANG/$CLANGXX),
#   --gc-boehm (Boehm instead of leak malloc), --include DIR (repeat),
#   --verbose, --keep-temp, -o OUT
# Windows without MSVC: add --mingw [--mingw-gccroot PATH] (or $CLANG_MINGW)
#   to build/run: clang -c gnu-target per input, then gcc/g++ link
#   (MinGW gcc auto-detected from PATH)
```

## Toolchain

| Need | Floor | Why |
|---|---|---|
| `clang` / `clang++` | 15 | IR is opaque-pointer-only (`ptr` everywhere); verified on 23.1.2 (Windows MSVC target) |
| C++ standard (shim) | C++17 | `--emit-cpp-shim` output compiles with `g++` or `clang++ -std=c++17` |
| C standard (runtime) | C11 + `-fwrapv` | `llvmrt/*.c`; Dirac `int` is 32-bit wrapping |
| Optional Boehm | `-lgc -DCDI_GC_BOEHM` | otherwise leak-by-design malloc |

Without `clang`, `g++ -std=c++17 -fsyntax-only` on the shim still
proves your native declarations resolve against the real headers.

## Surface (normative: `../bracket-jvm/CDirac.md`)

`var` declares, `set` reassigns, `const` binds once; `{…}`-only blocks;
`f(a)` calls, `a.b(c)` dot calls, `C(a)` construction (no `new`);
`for x in xs {…}` over arrays/queues; `machine.op(…)` namespace;
`[requires-java …]` directives, `[]`, and value spellings stay bracketed.

## Status

Backend complete: full core parity (all 62 builtins, short-circuit
`and`/`or`, value-`if`, `machine.alloc`/`dealloc`), C++-first native
interop (incl. `varargs` + struct pointers + `cpp-vector` bridges),
nested procedures with by-reference captures, a `build`/`run` driver,
and green gates — `cargo test` (73: unit + corpus + native + N1 +
driver + closures + filetest + robustness), `llvmrt/tests/run_tests.py` (5 drivers),
`tools/llvmcheck.py` (byte-exact vs the tree-walker, `interop.cdc`
rejected as designed; closure demos in `examples/closures/` skipped
by design since the tree-walker rejects nested procs).

Deliberately out: static types are parsed but unenforced (no checker
crate yet), attributes (`@timeout`/`@retry`/`@tailcallopt`) are
rejected, Java interop is JVM-only, and the remaining C++ v2 list
(function pointers, GC-managed handles, `@typecheck` member
signatures) stays future work — each still needs a 2-line header-side
wrapper today.
