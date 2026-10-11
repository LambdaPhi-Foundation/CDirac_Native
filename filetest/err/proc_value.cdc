// ERROR: 'f' is a procedure, not a value
fn f(x|int) | int {
  return x;
}
displayln(f);
