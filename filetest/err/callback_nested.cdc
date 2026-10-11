// ERROR: must name a top-level procedure
fn outer() {
  fn q() | int {
    return 1;
  }
  return q();
}
[callback q [int] int]
displayln(outer());
