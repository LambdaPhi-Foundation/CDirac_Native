// callback_basic.cdc — Dirac proc as C callback (IR + shim only;
// execution is gated by examples/cpp/callback.cdc golden).
[include-c "cb.h"]
[callback inc [int] int]
[extern-c apply_twice "apply_twice" [inc int] int]
fn inc(x|int) | int { return x + 1; }
displayln(apply_twice(inc, 41));
