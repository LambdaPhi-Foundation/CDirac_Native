# filetest/ — file-based checks for `.cdc` programs

A dedicated folder of small `.cdc` programs exercised by
`tests/filetest.rs` (`cargo test --test filetest`). No toolchain
needed: everything runs through the parser and the LLVM lowering
only (execution stays in `tools/llvmcheck.py`, which needs clang).

## Layout

- `ok/` — programs that must **pass**: parse cleanly, print canonical
  SDirac, and lower to LLVM IR. Files using native directives
  (`[extern-c]`, `[extern-cpp]`, `[cpp-...]`, `[include-c...]`) must
  additionally produce a C++ shim.
- `err/` — programs that must **fail**. Each file declares its
  expectation on a comment line:
  `// ERROR: <substring of the expected diagnostic>`
  The harness accepts the failure at either stage (parse or lower)
  as long as the message contains the substring — and fails the test
  if the file unexpectedly succeeds.

## Adding a case

1. Keep it small and single-feature; name it after what it covers
   (`closure_accum.cdc`, `arity.cdc`, …).
2. For `err/` cases, pin the narrowest stable substring (e.g.
   `unbound identifier: nope`, not the `line N:` prefix).
3. Run `cargo test --test filetest` and commit green.
