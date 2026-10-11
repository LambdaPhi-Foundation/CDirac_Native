// varargs_printf.cdc — C varargs bridge (IR + shim only; linking
// needs clang and is covered by tools/llvmcheck.py).
[include-c "stdio.h"]
[extern-c cprintf "printf" [string] int varargs]
displayln(cprintf("hi %d %s", 42, "yo"));
displayln(cprintf("plain"));
