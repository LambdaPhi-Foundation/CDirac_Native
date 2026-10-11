// printf.cdc -- C varargs through the varargs bridge.
//
// A trailing `varargs` on an extern decl covers fixed..=fixed+4 Dirac
// args over the n-ary shape; the shim forwards extras with runtime tag
// dispatch (int/bool->int32, double, string->char*, ptr->void*).
//
// run:
//   cdirac build examples\cpp\printf.cdc -o printf
//   ./printf
// expect: hi 42 yo (then the printf return count via displayln)

[include-c "stdio.h"]
[extern-c cprintf "printf" [string] int varargs]
displayln(cprintf("hi %d %s\n", 42, "yo"));
displayln(cprintf("%d + %d = %d\n", 1, 2, 3));
displayln(cprintf("%s\n", "plain"));
