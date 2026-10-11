// demo.cdc -- nested procedures (closures) with by-reference captures.
//
// A nested `fn` lifts to a top-level function; outer locals become
// hidden leading params holding slot addresses, so `set` inside the
// nested proc writes through. Captures evaluate at each call.
//
// NOTE: kept out of `examples/*.cdc` on purpose — the reference
// tree-walker rejects nested procs, so the byte-exact `llvmcheck`
// gate skips this directory (IR-shape tests live in `tests/closures.rs`).
//
// run:
//   cdirac build examples\closures\demo.cdc -o demo
//   ./demo
// expect: 31, 7, 6, 120

fn outer(n|int) | int {
  var m = n + 1;
  fn helper(x|int) | int {
    return x + m;
  }
  return helper(n * 2);
}
displayln(outer(10));

fn counter() | int {
  var total = 0;
  fn bump(x|int) {
    set total = total + x;
  }
  bump(3);
  bump(4);
  return total;
}
displayln(counter());

fn fact(n|int) | int {
  fn walk(k|int, acc|int) | int {
    if (k <= 1) { return acc; }
    return walk(k - 1, acc * k);
  }
  return walk(n, 1);
}
displayln(fact(5));
displayln(fact(3));
