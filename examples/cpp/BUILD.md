# examples/cpp — native interop demos (C++-first)

- `accum.h` — header-only C++ demo library (namespace `dirac_demo`,
  free functions + opaque `Accum` class). No `.cpp` to link: the only
  extra translation unit is the generated shim.
- `accum.cdc` — C++ demo: extern functions, class, ctor, instance +
  static methods, field read, deletes.
- `cbasic.cdc` — thin C demo (`string.h`/`stdlib.h` decls).
- `consts.h` / `consts.cdc` — enums, named consts, static data members
  (incl. `const` read-only members).
- `vecs.cdc` — `[cpp-vector IntVec/StrVec/DVec]` bridge (needs only
  `<vector>`).
- `printf.cdc` — C varargs (`printf` with 0–4 extras via tag dispatch).
- `point.h` / `point.cdc` — struct-by-pointer (struct rides `cpp-class`).

Build — manual (needs `clang++` for the `.ll`, `g++` for the shim):

```sh
cd ../..  # E:\cdirac
./target/debug/cdirac --emit-llvm examples/cpp/accum.cdc > out.ll
./target/debug/cdirac --emit-cpp-shim examples/cpp/accum.cdc > cdi_shim.cpp
g++ -std=c++17 -c cdi_shim.cpp -o cdi_shim.o -I llvmrt -I examples/cpp
clang++ out.ll llvmrt/value.c llvmrt/arith.c llvmrt/strings.c \
  llvmrt/containers.c llvmrt/cinterop.c cdi_shim.o -o accum
./accum
# expect: 22, 2, 100, 42, HELLO (order per accum.cdc)
```

Build — driver (same, one command, needs `clang++` on PATH):

```sh
./target/debug/cdirac build examples/cpp/accum.cdc -o accum --verbose
./target/debug/cdirac build examples/cpp/vecs.cdc -o vecs
./target/debug/cdirac build examples/cpp/cbasic.cdc -o cbasic
./target/debug/cdirac build examples/cpp/consts.cdc -o consts \
  -I examples/cpp   # consts.h lives beside the demo
```

Expected outputs:

| Demo | Output (in order) |
|---|---|
| `accum` | `22` (`10+5+7`), `2` (`count_`), `100` (`make`), `42` (`add2`), `HELLO` (`shout`) |
| `cbasic` | `5`, `42` (`strlen`/`atoi` through thin C decls) |
| `consts` | enum snapshots + consts + static reads (see `consts.cdc`) |
| `vecs` | `3, 1, 4, 9, 4, [9, 2, 3], ["a", "b", "c"], 0, 3, [7.0]` |
| `printf` | `hi 42 yo, 9, 1 + 2 = 3, 10, plain, 6` (text + `printf` return counts; needs `clang` to run) |
| `point` | `25, 3, 7, P(3,4)` (struct-by-pointer; `-I examples/cpp`) |
| `struct` | `15`, `[[13 2] 7]` (by-value structs incl. nesting; `-I examples/cpp`) |
| `callback` | `43`, `5`, `4`, `7`, `7` (Dirac procs as C callbacks; `-I examples/cpp`) |

Without `clang++`, this still proves out:

```sh
./target/debug/cdirac --emit-cpp-shim examples/cpp/accum.cdc > cdi_shim.cpp
g++ -std=c++17 -fsyntax-only cdi_shim.cpp -I llvmrt -I examples/cpp
# silence = your decls resolve against the real headers
```
