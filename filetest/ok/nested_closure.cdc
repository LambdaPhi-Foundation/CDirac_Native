// nested_closure.cdc — two-level nesting with middleman forwarding.
fn outer() | int {
  var x = 7;
  fn mid() | int {
    fn leaf() | int {
      return x + 1;
    }
    return leaf();
  }
  return mid();
}
displayln(outer());
