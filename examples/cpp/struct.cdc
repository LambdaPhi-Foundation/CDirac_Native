// struct.cdc -- by-value structs: Dirac arrays <-> C++ aggregates.
//
// A `[cpp-struct ...]` value IS its field array on the Dirac side
// (`array(3, 4)` for a two-int struct); the shim converts field by
// field, so C++ padding never matters.
//
// run:
//   cdirac --emit-llvm examples\cpp\struct.cdc > out.ll
//   cdirac --emit-cpp-shim examples\cpp\struct.cdc > cdi_shim.cpp
//   clang++ out.ll llvmrt\*.c cdi_shim.cpp -I llvmrt -I examples\cpp -o struct
//   ./struct
// expect: 15, [[13 2] 7]

[include-cpp "struct.h"]
[cpp-struct Point "dirac_demo::Point" "struct.h" [x int] [y int]]
[cpp-struct Labeled "dirac_demo::Labeled" "struct.h" [at Point] [tag int]]
[extern-cpp move_by "dirac_demo::move_by" [Point int int] Point]
[extern-cpp manhattan "dirac_demo::manhattan" [Point] int]
[extern-cpp label_at "dirac_demo::label_at" [Point int] Labeled]

var p = move_by(array(3, 4), 10, -2);
displayln(manhattan(p));
displayln(label_at(p, 7));
