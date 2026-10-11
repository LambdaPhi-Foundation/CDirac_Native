// callback.cdc -- Dirac procs as C callbacks via LLVM trampolines.
//
// A `[callback Alias ...]` decl exposes a top-level Dirac proc under
// a C signature; the alias doubles as a function-pointer type in
// extern decls. Bare `inc` below reads as the trampoline address.
//
// run:
//   cdirac --emit-llvm examples\cpp\callback.cdc > out.ll
//   cdirac --emit-cpp-shim examples\cpp\callback.cdc > cdi_shim.cpp
//   clang out.ll llvmrt\*.c cdi_shim.cpp -I llvmrt -o callback
//   ./callback
// expect: 43, 5, 4, 7, 7

[include-c "cb.h"]
[callback inc [int] int]
[callback is_even [int] int]
[callback is_pos [int] bool]
[callback show [int] void]
[extern-c apply_twice "apply_twice" [inc int] int]
[extern-c count_matching "count_matching" [is_even int int] int]
[extern-c count_true "count_true" [is_pos int int] int]
[extern-c call_twice "call_twice" [show int] void]

fn inc(x|int) | int { return x + 1; }
fn is_even(x|int) | int {
  if (remainder(x, 2) == 0) { return 1; }
  return 0;
}
fn is_pos(x|int) | bool { return x > 0; }
fn show(x|int) { displayln(x); }

displayln(apply_twice(inc, 41));
displayln(count_matching(is_even, 0, 10));
displayln(count_true(is_pos, 0, 5));
call_twice(show, 7);
