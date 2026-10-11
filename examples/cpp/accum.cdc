// accum.cdc -- C++-first interop demo (headers as first-class Dirac).
//
// Directives stay bracket-form (pragma-like); calls are ordinary Dirac:
// `Accum(10)` constructs, `a.add(5)` calls a method, `a.count` reads a
// field, `Accum.make(100)` calls a static, `Accum_delete(a)` frees.
// run:
//   cdirac --emit-llvm examples\cpp\accum.cdc > out.ll
//   cdirac --emit-cpp-shim examples\cpp\accum.cdc > cdi_shim.cpp
//   g++ -std=c++17 -c cdi_shim.cpp -o cdi_shim.o -I llvmrt -I examples\cpp
//   clang++ out.ll llvmrt\*.c cdi_shim.o -o accum

[include-cpp "accum.h"]
[extern-cpp add2 "dirac_demo::add" [int int] int]
[extern-cpp shout "dirac_demo::shout" [string] string]
[cpp-class Accum "dirac_demo::Accum" "accum.h"]
[cpp-ctor Accum [int]]
[cpp-method Accum add [int] void]
[cpp-method Accum total [] int]
[cpp-static Accum make [int] Accum]
[cpp-field Accum count_ int]

var a|ptr = Accum(10);
a.add(5);
a.add(7);
displayln(a.total());
displayln(a.count_);
var b|ptr = Accum.make(100);
displayln(b.total());
displayln(add2(20, 22));
displayln(shout("hello"));
Accum_delete(a);
Accum_delete(b);
