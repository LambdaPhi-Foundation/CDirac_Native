// hello.cdc — first CDirac-native program (frontend demo).
// run: cdirac parse examples\hello.cdc
//      cdirac --sdirac examples\hello.cdc

fn fact(n|int) | int {
  if (n <= 1) { return 1; }
  return n * fact(n - 1);
}

var total|int = 0;
for x in array(1, 2, 3, 4) {
  set total = total + x;
}
machine.displayln(total);
displayln(fact(5));
