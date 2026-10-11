// closure_accum.cdc — nested proc captures an outer local by
// reference; `set` inside writes through to the caller's slot.
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
