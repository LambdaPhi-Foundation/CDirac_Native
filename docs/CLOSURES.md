# Nested procedures (closures) — LLVM backend

Status: ACTIVE. A nested `fn` lifts to a top-level LLVM function
(`cdi_proc_<path>_<name>_<uniq>`). No closures-as-values: procs are
name-called only (a proc name in value position is still `'f' is a
procedure, not a value`).

## Semantics

* **Captures are by reference.** Each captured outer local becomes a
  hidden leading `ptr %capN` parameter holding the *caller's slot
  address*. Reads load through it; `set` writes through it — the
  accumulator pattern works (`bump` below really accumulates).
* **Captures evaluate at each call** from the caller's frame, so
  mutations between definition and call are visible. (Correlated
  wart: a shadowing block-local between def and call point is what
  gets captured — don't do that.)
* **Scope:** nested names are visible from their definition statement
  to the end of the enclosing *procedure body* (function-wide,
  sequential — calling before the definition fails like the top-level
  incremental rule, and mutual recursion needs restructuring).
  Inner definitions shadow outer names.
* A definition inside a branch still defines (codegen is static); a
  capture whose slot has exited (block over) fails at the call with
  `cannot capture 'x' here (out of scope)`. Defining without calling
  is harmless.
* `break`/`continue` inside a nested proc never touch outer loops
  (`break outside a loop`); `return` returns from the nested proc.
* `const` captures stay const (`cannot assign to constant`).
* Depth is unlimited; middlemen forward addresses they never read.

```c
fn counter() | int {
  var total = 0;
  fn bump(x|int) {
    set total = total + x;   // writes through %cap0 to total's slot
  }
  bump(3);
  bump(4);
  return total;              // 7
}
```

## Limits (deliberate)

* No first-class proc values, no anonymous lambdas, no mutual
  recursion between siblings (call-before-def fails by design).
* Globals need no capture (direct access, like top-level procs).
* The reference tree-walker still rejects nested procs, so closure
  demos live in `examples/closures/` (skipped by the byte-exact
  `llvmcheck` gate) with IR-shape tests in `tests/closures.rs`.
