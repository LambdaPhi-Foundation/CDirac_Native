// ERROR: duplicate procedure 'f'
fn f(x|int) | int {
  return x;
}
fn f(x|int) | int {
  return x;
}
displayln(f(1));
