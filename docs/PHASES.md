# Build log — completing `cdirac`

Scope per user (2026-10-10): Production-harden + implement closures +
keep Boehm opt-in + varargs/structs first. Types stay parsed-but-unenforced;
attributes/Java stay rejected (parity only).

## P0 — Baseline (2026-10-10)

* `cargo test`: 39 passed (13 unit + 2 corpus + 10 n1 + 14 native_cpp).
  README said 35 — drift noted, README updated in P2/P3.
* `py llvmrt/tests/run_tests.py`: 5/5 drivers pass with gcc
  (`C:\Users\ITER\scoop\apps\gcc`).
* `py tools/llvmcheck.py`: BLOCKED — `FileNotFoundError` spawning
  `D:\LLVM\bin\clang.exe` (no clang on PATH, no `D:\LLVM`).
  `bracket.exe` present at `E:\bracket-jvm\target\debug\bracket.exe`.
  Confirms P2 need: PATH probing + graceful skip.
* `cdirac --emit-llvm examples\hello.cdc`: IR header + declares emit fine.

## P1 — `build`/`run` driver (2026-10-10)

* New `src/driver.rs` (std-only, no deps): `BuildConfig`, `RT_SRCS`,
  `probe_tool` (`exe --version`), `driver_candidates`
  (flag > `$CLANG`/`$CLANGXX` > `clang`/`clang++` on PATH > legacy
  `D:\LLVM\bin`), `resolve_driver`, `llvmrt_dir`, `default_output`,
  `plan` (parse+lower, no fs writes), `build` (temp `out.ll`/`shim.cpp`
  + driver invoke), `run` (build-temp + exec, inherited stdio).
* `src/main.rs`: `build FILE.cdc [-o OUT] [--clang[xx] PATH] [--gc-boehm]
  [--include DIR]... [--verbose] [--keep-temp]` prints output path;
  `run FILE.cdc [-- args…]` exits with prog code. Old commands unchanged.
* GC stays opt-in: default leak malloc; `--gc-boehm` adds
  `-DCDI_GC_BOEHM -lgc` (plus `link-native` flags preserved).
* Native detection via `compile_cpp_shim` success → `clang++`, else `clang`.
* Tests: 5 new `driver::tests` (candidates order, helpful resolve errors,
  plain→C-driver plan, native→CXX + `-lm` link flag, default output names).
  `cargo test`: 44 passed (18 lib incl. 5 driver + 2 + 10 + 14).
* Manual: `--help` lists new cmds; `build` without clang errors helpfully
  (`gcc cannot compile LLVM IR (.ll)`); `--bogus` rejected; fake-driver
  `--clang=cargo --verbose` prints full argv then surfaces `link failed`.
* `README.md` Commands updated (44 tests + build/run lines + flags).

## P2 — Portable gates + CI (2026-10-10)

* `tools/llvmcheck.py`: `resolve_driver` probes
  (flag > `$CLANG`/`$CLANGXX` > PATH > legacy `D:\LLVM\bin`) via
  `--version`; no-toolchain → per-file SKIP + compile-only checks
  (`interop.cdc` PASS) with exit 0 (was `FileNotFoundError` crash).
  New flags: `--filter SUB`, `--keep-tmp`, `--clang/--clangxx/--bracket/--cdirac`;
  non-Windows `cdirac`/`bracket`/`prog` names handled (no hardcoded `.exe`).
* `llvmrt/tests/run_tests.py`: `CC` may carry flags (`shlex.split`);
  fallback chain `gcc → clang → cc`; `--filter=` + `--help`.
* New `.github/workflows/ci.yml`: `cargo test` + clippy + fmt (win+linux),
  llvmrt C tests (win+linux), differential gate (ubuntu+clang; bracket-jvm
  sibling checkout documented).
* Verified: `py tools/llvmcheck.py` → 1 PASS (interop) + 9 SKIP, rc=0;
  `run_tests.py --filter=test_value` passes.

## P3 — Docs + vec tests + example fix (2026-10-10)

* `llvmrt/value.h`: stale “planned, not here” comment fixed — points at
  `mem.h` + `llvmgen` lowering.
* `docs/CPP-INTEROP.md §7` written: `[cpp-vector]` desugar table,
  element map, ownership, bans, ambiguity rule, `vecs.cdc` outputs.
* New `docs/ERRORS.md`: lexer / parser / core-static / intentional
  (Java+attrs) / native catalog with triggers and fixes.
* `examples/cpp/BUILD.md`: driver one-liners, `consts`/`vecs` rows,
  expected-output table; fixed stale `T:\` cwd.
* **Bug fix — `examples/cpp/vecs.cdc` did not compile**: three vector
  classes + `v.len()` sugar → `ambiguous cpp method 'len'`. Rewrote with
  explicit `Alias_op` aliases (outputs unchanged); all 5 examples now
  `--emit-llvm` rc=0.
* `tests/native_cpp.rs`: +2 vec tests (bridge lowers + elem map + solo
  sugar + ambiguity error; `bool`/`void`/nested bans; `[5]` build).
  `cargo test`: 46 passed (18 + 2 + 10 + 16).

## P4 — C++ v2a: varargs + struct pointers (2026-10-10)

* Syntax: trailing `varargs` on extern decls —
  `[extern-c cprintf "printf" [string] int varargs]` (same for
  `extern-cpp`). `WrapperKind::ExternC/ExternCpp` gain `varargs: bool`;
  new `is_varargs` + `MAX_VARARGS = 4`.
* Lowering (`llvmgen` alias dispatch): exact fixed → varargs
  (`fixed..=fixed+4`, `find_varargs`) → vec-build → `expects K or more`
  range error. One varargs decl per alias (dup rule).
* Shim: fixed params via new `unbox_nary_lines` (`a[i]` indexing —
  first version wrongly emitted `a0`, caught by `g++ -fsyntax-only`),
  then a flat per-arity tag-combination switch (base-4:
  int|bool→`int32_t`, double, string→`char*`/`std::string`,
  ptr→`void*`; others abort `cannot pass vararg`; `>4` aborts
  `at most 4 varargs`). C promotions honored.
* Structs: pointer pattern documented (`struct == cpp-class` + real C++
  ctor + header factory); new `examples/cpp/point.h` + `point.cdc`.
  Syntax check caught 2 demo bugs (missing ctor, `void*`→`const Point*`
  needs the class type) — fixed, proving the shim-as-typechecker loop.
* Examples: new `printf.cdc` (0–4 extras). All 7 shims
  (`accum/cbasic/consts/vecs/printf/point` + plain) pass
  `g++ -std=c++17 -fsyntax-only` (no clang on this box; runtime
  byte-exactness deferred to CI `llvmcheck`).
* Tests: +3 `native_cpp` (n-ary lower ×4 arities, floor/cap errors,
  shim tag-guard text, dup both orders, C++ spelling).
  `cargo test`: 49 passed (18 + 2 + 10 + 19).
* Docs: `CPP-INTEROP.md` §6 updated + new §8; `ERRORS.md` varargs rows;
  `BUILD.md` new demos; `native.rs` header line.

## P5 — Nested procs/closures via lambda-lift (2026-10-10)

* Semantics (per user vote to implement): nested `fn` lifts to
  `cdi_proc_<path>_<name>_<uniq>` with hidden leading `ptr %capN`
  capture params holding **caller slot addresses** — `set` writes
  through, captures evaluate per call, consts stay const, depth
  unlimited with middleman forwarding. Docs: new `docs/CLOSURES.md`.
* New `src/closures.rs` (~600 lines, pure/total): lexical use-scan
  (receiver-aware heads, dotted receivers, block scoping mirroring
  codegen incl. for init/test/body/update order), seed + children +
  callee propagation to fixpoint, const-flag resolution. Keys are
  `path + name + span` (misses degrade to errors, never wrong caps).
* `src/llvmgen.rs`: `ProcFrame` stack + `frame_bases` (lookups bounded
  to the current LLVM function), `NestedSig` table, def-time
  capture filter (typos/error-names take natural errors), call-time
  slot resolution (`cannot capture … out of scope`), hoisted `lifted`
  sink flushed end-of-module, loops cleared per body, dup/clash/proc-
  value checks extended. Top-level path byte-identical by construction.
* Proof: old-vs-new binary differential over all 17 gated programs —
  15 byte-identical IR, 0 RC flips (no corpus program nests); only
  diff is the new `examples/closures/demo.cdc` (old rejects, new
  lifts, expect `31, 7, 6, 120`). Demo in subdir so `llvmcheck`
  (top-level `*.cdc` only) stays green vs the rejecting tree-walker.
* Tests: new `tests/closures.rs` (10: lift shapes, plain sigs,
  recursion, write-through, deep forward, siblings, shadowing, const,
  9-case error surface incl. mutual-recursion rejection).
  `cargo test`: 58 passed (18 + 2 + 10 + 19 + 9).
* `README.md` Layout + Status rewritten (58 tests, closures + varargs
  done; remaining v2 narrowed to fn-pointers/GC-handles/`@typecheck`).

## P6 — Hardening sweep (2026-10-10)

* `cargo fmt` clean (was never fmt-clean, not even `ast.rs`);
  `cargo clippy --all-targets -- -D warnings` clean — fixed 12 lints
  (mine: arg-bundling `ScanHead`, `TopItem`/`ProcDef` aliases,
  `.ok()`; pre-existing in `native.rs`: `contains`, `is_empty`,
  `is_multiple_of`, if-same-branches in `include_line` — all
  behavior-preserving, verified by suite).
* **Found + fixed a real regression:** P4 had narrowed the second
  `void`-param check to varargs-only (`[extern-c f [void] int]`
  slipped through); restored unconditional + regression tests.
* New `tests/robustness.rs`: ~1000 deterministic mutations (LCG,
  fixed seed) over 7 seeds + all examples through
  parse/lower/shim — zero panics; corpus-wide lower sweep clean.
  (`unreachable!` invariants hold; any future panic here must become
  `Err`.)
* Perf smoke: `--emit-llvm` over the 9-file corpus takes 0.23 s;
  varargs shim for `printf.cdc` is 2660 lines / 106 KB (tag-combo
  switch for ≤4 extras — the documented cost of heterogeneous C
  varargs from boxed values; `g++ -fsyntax-only` still instant).
* `cargo test`: 60 passed (18 + 2 + 10 + 19 + 9 + 2 robustness);
  `llvmrt` 5/5; `llvmcheck` SKIP-clean without clang (1 PASS +
  skips, rc=0). README count updated to 60.

## P7 — filetest/ folder: file-based .cdc checks (2026-10-10)

* New dedicated `filetest/` folder (conventions in
  `filetest/README.md`): `ok/` (6 programs: fact, strings,
  containers, closure_accum, varargs_printf, vec_basic) must parse +
  print + lower (plus shim when native directives are present);
  `err/` (8 programs: unbound, arity, dup_proc, capture_scope,
  vec_bool, void_param, proc_value, const_assign) must fail with the
  diagnostic in their `// ERROR: <substring>` marker, at parse or
  lowering stage.
* New `tests/filetest.rs` harness (`ok_files_pass` +
  `err_files_fail_as_declared`); toolchain-free like the other
  frontend gates (execution stays in `tools/llvmcheck.py`).
* Drive-by fix: pre-existing `clippy::single_match` lint in
  `tests/robustness.rs` that blocked `cargo clippy --all-targets
  -- -D warnings` for the whole workspace.
* `cargo test`: 62 passed (18 + 2 + 10 + 19 + 9 + 2 filetest + 2
  robustness); `cargo fmt --check` and clippy strict both clean.

## P9 — standalone driver: `--mingw` split flow in `cdirac build/run`

* Scope per user: driver only — no interop changes, no feature cuts.
  `cdirac build`/`run` now work out of the box on this MSVC-less box.
* `src/driver.rs`: `BuildConfig` gains `mingw: bool` (`--mingw`,
  `$CLANG_MINGW`) + `mingw_gccroot: Option<String>`
  (`--mingw-gccroot`, `$MINGW_GCCROOT`). New `MingwToolchain` /
  `MingwPlan` types; `resolve_mingw` auto-detects MinGW gcc from
  `PATH` (root = parent-of-parent of `gcc`, validated by
  `<triplet>/include/stdio.h`) with pure-filesystem probing.
* `BuildPlan` carries the split plan alongside the untouched
  single-step `argv_template` (existing tests unaffected);
  `build()` compiles each input to `.o` (`--target=` + MinGW
  headers, C++ dirs for the shim only) then links with `gcc`/`g++`
  (`link-native` + `-lgc` ride the link step, `-D`/extra `-I` ride
  compile). Preprocessor flags are skipped for `.ll` inputs (clang
  warned they were unused).
* `src/main.rs`: `--mingw` / `--mingw-gccroot[=PATH]` on `build` and
  `run` (+ usage text); `README.md` flags documented.
* Tests: +5 driver unit tests over a fabricated gccroot (discovery,
  missing-root error, plain/native split shapes, link-driver
  selection). Verified live: `build` + `run` on `hello.cdc`
  (`10`, `120`), native `build` on `accum.cdc` (`22 2 100 42 HELLO`,
  matches its golden).
* Still out (next): `CLANG`/`CLANGXX` must currently be set or passed
  explicitly on boxes without clang on `PATH` — no scoop probing by
  design; sibling-checkout gates untouched per scope vote.
* `cargo test`: 67 passed (23 lib incl. 10 driver + 2 + 10 + 19 + 9 +
  2 filetest + 2 robustness); `cargo fmt --check` and clippy strict
  both clean.

## P8 — llvmcheck green on this box via MinGW split flow (2026-10-10)

* Environment: scoop `llvm` 23.1.3 (MSVC target, no SDK/headers) +
  scoop `gcc` 15.2.0 (MinGW-w64, has headers). Plain single-driver
  link fails on `stdio.h` — expected with no MSVC, not a cdirac bug.
* Proven manually first: `hello.cdc` → IR → `clang -c` (gnu target) +
  `gcc` link → prints `10`/`120`, byte-identical to the tree-walker.
  (PowerShell `>` writes UTF-16 BOM which clang rejects — red herring
  during manual probing only; `llvmcheck.py` writes bytes.)
* New `--mingw` mode (`$CLANG_MINGW`, `--mingw-gccroot`/`$MINGW_GCCROOT`):
  auto-detects MinGW gcc from PATH, compiles every input with
  `clang --target=x86_64-w64-windows-gnu -I<mingw dirs> -c`, links
  objects with `gcc`/`g++`. Two ordering bugs found+fixed along the
  way: C++ dirs must precede C dirs (`#include_next` in libstdc++),
  and libstdc++ dirs apply to the shim only (its `<math.h>` wrapper
  breaks plain C compiles under clang).
* New `--native-dir DIR` golden gate (repeatable, `$NATIVE_DIRS`):
  `DIR/*.cdc` built with `-I DIR`, stdout+rc0 compared against
  `<name>.expected`; plus `--bless` to author goldens (verify by hand!).
  The tree-walker rejects native programs (`unknown type: ptr`), so
  goldens — not differential — are the contract here.
* Bless-then-verify caught **three wrong docs**: `point` prints
  `25, 3, 7, P(3,4)` (not `25, 0, …`), `vecs` ends `[7.0]` (double
  vector renders the float), `printf` counts are `9, 10, 6`. Fixed
  `BUILD.md` + the two `.cdc` header comments; goldens hold the truth.
* Result: `llvmcheck.py --mingw --native-dir examples/cpp` →
  **16/16 green** (10 differential + 6 goldens) — first full green run
  anywhere outside CI. New `*.expected` files committed beside sources.
* `README.md`: fixed stale test count (44 → 62), documented the
  `--mingw` invocation.

## P10 — Structs by value + filetest expansion (2026-10-10)

* Scope per user: first CPP-header v2 slice = struct-by-value params
  (function pointers, GC handles, `@typecheck` sigs stay out), plus
  broader filetest coverage. No constructor sugar (arrays suffice).
* New `[cpp-struct Alias Qualified Header [field type] ...]`:
  `NativeTy::Struct` + `StructInfo` + `NativeDecls.structs`, threaded
  through `parse_ty`/`parse_params`. Dirac values are field arrays;
  the shim converts field by field (aggregate brace-init in — parens
  are C++20, floor is C++17 — array build out), so padding never
  matters. Order-dependent registration (like classes) rules out
  by-value cycles; structs compose by nesting, not by vectorizing
  (`cpp-vector` rejects them at collect, bool/void-style).
* Conversions plug into the shared helpers: `unbox_lines` /
  `unbox_nary_lines` generalized to `unbox_expr(src, dst)` (byte-
  identical output, test-pinned), new `box_into(dst, src)` for the
  Struct arms of `box_temp` / `box_result` / `box_result_prefixed`
  (scalar arms untouched). Structs work in extern/ctor/method/field
  positions; `cpp-const` still scalar-only by its existing match.
* `llvmgen.rs`: structs join the builtin/machine + redefine-as-proc/
  var collision chains; bare alias errors as `is a cpp-struct, not a
  value`. No callgen changes (arity/wrapper machinery is type-generic).
* End-to-end caught two real bugs before any test ran: the shim
  missed `#include "containers.h"` (`cdi_len/cdi_get/cdi_array0/
  cdi_arr_push` undeclared — first shim code to need it), and paren
  aggregate-init is C++20 (switched to braces). New `examples/cpp/
  struct.h` + `struct.cdc`, golden `struct.expected` (`15`,
  `[[13 2] 7]`); `llvmcheck --mingw --native-dir examples/cpp` is
  **17/17 green** (incl. 7 native goldens).
* filetest: +4 ok (`struct_basic`, `machine_mem`, `while_break`,
  `nested_closure`), +6 err (`struct_shape`, `struct_void_field`,
  `struct_unknown`, `struct_dup_field`, `machine_unknown`,
  `break_outside`) — 10 ok + 14 err total, all green first try.
* Docs: `CPP-INTEROP.md` §6 marked + new §9, `ERRORS.md` rows,
  `BUILD.md` struct row, module-doc directive line.
* `cargo test`: 70 passed (23 lib + 2 + 10 + 22 native_cpp + 9 + 2
  filetest + 2 robustness); `cargo fmt --check` and clippy strict
  both clean.

## P11 — Function pointers via LLVM trampolines (2026-10-10)

* Scope per user: second CPP-header v2 slice (GC handles,
  `@typecheck` sigs stay out). No libffi, no name mangling: the
  trampoline lives in the same LLVM module as the proc (proc symbols
  carry a `uniq` suffix, so only `llvmgen` can reference them).
* New `[callback Alias [params] ret]`: `CallbackInfo` +
  `NativeDecls.callbacks`; the alias doubles as a function-pointer
  type (`NativeTy::Func`) in extern decls. Bare proc name with a
  callback decl reads as the trampoline address boxed via
  `cdi_from_ptr` — the one way a proc name is a value.
* `llvmgen.rs`: `Gen.callback_tramps` (cloned into per-proc gens);
  resolve-after-procs (top-level + arity match; nested vs unknown
  targets get tailored errors); trampolines into the lifted sink
  (`define i32 @cdi_cb_inc_N(i32 %a0)` — C ABI in, boxed call,
  unboxed result; `bool` is `i8` both ways); conversion declares
  emitted only when callbacks exist (callback-free IR byte-identical).
  Sigs stay scalar (`int double bool string ptr`, `void` returns);
  `string` is `char*`/`std::string` per side, matching the pair.
* `native.rs`: `Func` arms in `cpp_param_ty` (cast spelling),
  `unbox_expr` (`reinterpret_cast`), `box_into`/`box_temp`/
  `box_result*` (address box); full collision coverage both orders;
  vec elems reject fnptrs at collect. Callbacks are not varargs
  extras (tag switch has no function case — documented).
* End-to-end caught two real bugs before any test ran: `_Bool` is
  not C++ (header uses `bool` + `<stdbool.h>`), and C declarator
  syntax needs the name inside (`int32_t (*p0)(…)` — new
  `fn_sig_decl` helper). New `examples/cpp/cb.h` + `callback.cdc`,
  golden (`43`, `5`, `4`, `7`, `7` — int, bool, and void callbacks
  all execute); gate is **18/18 green** (incl. 8 native goldens).
* filetest: +3 ok (`callback_basic`, `method_sugar`, `enum_const`),
  +4 err (`callback_no_proc/arity/nested/void_param`) — 13 ok + 18
  err total, all green first try.
* Docs: `CPP-INTEROP.md` §6 marked + new §10, `ERRORS.md` rows,
  `BUILD.md` row, module-doc directive line.
* `cargo test`: 73 passed (23 lib + 2 + 10 + 25 native_cpp + 9 + 2
  filetest + 2 robustness); `cargo fmt --check` and clippy strict
  both clean.
