// ERROR: cannot capture 't' here (out of scope)
if (true) {
  var t = 1;
  fn q() | int {
    return t;
  }
}
displayln(q());
