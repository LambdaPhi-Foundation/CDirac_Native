// vec_basic.cdc — typed STL vector bridge (IR + shim only).
[include-cpp "<vector>"]
[cpp-vector IntVec int]
var v = IntVec(1, 2, 3);
displayln(IntVec_len(v));
displayln(IntVec_get(v, 0));
IntVec_push(v, 4);
displayln(IntVec_len(v));
IntVec_delete(v);
