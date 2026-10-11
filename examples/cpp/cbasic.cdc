// cbasic.cdc -- thin C header support (a few decls; focus stays on C++).
//
// run:
//   cdirac --emit-llvm examples\cpp\cbasic.cdc > out.ll
//   cdirac --emit-cpp-shim examples\cpp\cbasic.cdc > cdi_shim.cpp
//   g++ -std=c++17 -c cdi_shim.cpp -o cdi_shim.o -I llvmrt
//   clang++ out.ll llvmrt\*.c cdi_shim.o -o cbasic

[include-c "string.h" "stdlib.h"]
[extern-c cstrlen "strlen" [string] int]
[extern-c catoi "atoi" [string] int]

displayln(cstrlen("hello"));
displayln(catoi("42"));
