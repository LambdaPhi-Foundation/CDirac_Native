// vecs.cdc -- typed STL vectors as first-class Dirac containers.
//
// A `[cpp-vector Alias elem]` bridge owns a `std::vector<E>` behind an
// opaque handle with the Dirac-array-shaped API: n-ary build, len/get/
// put/push/pop/str, and delete. Note `IntVec(5)` is the one-element
// vector [5], never a size (arrays are values, not sizes).
//
// Method names repeat across vector classes, so multi-class programs
// use explicit `Alias_op` aliases (dot sugar is ambiguous by design;
// with a single class in scope `v.len()` resolves fine).
//
// run:
//   cdirac --emit-llvm examples\cpp\vecs.cdc > out.ll
//   cdirac --emit-cpp-shim examples\cpp\vecs.cdc > cdi_shim.cpp
//   clang++ out.ll llvmrt\*.c cdi_shim.cpp -I llvmrt -o vecs
//   ./vecs
// expect: 3, 1, 4, 9, 4, [9, 2, 3], ["a", "b", "c"], 0, 3, [7.0]

[include-cpp "<vector>"]
[cpp-vector IntVec int]
[cpp-vector StrVec string]
[cpp-vector DVec double]

var v = IntVec(1, 2, 3);
displayln(IntVec_len(v));
displayln(IntVec_get(v, 0));
IntVec_push(v, 4);
displayln(IntVec_len(v));
IntVec_put(v, 0, 9);
displayln(IntVec_get(v, 0));
displayln(IntVec_pop(v));
displayln(IntVec_str(v));
var s = StrVec("a", "b");
StrVec_push(s, "c");
displayln(StrVec_str(s));
var d = DVec();
displayln(DVec_len(d));
displayln(IntVec_len(v));
var e = DVec(7);
displayln(DVec_str(e));
IntVec_delete(v);
StrVec_delete(s);
DVec_delete(d);
DVec_delete(e);
