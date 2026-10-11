// while_break.cdc — while loop with break and continue.
var i = 0;
var total = 0;
while (true) {
  set i = i + 1;
  if (i > 10) { break; }
  if (i == 3) { continue; }
  set total = total + i;
}
displayln(total);
