// strings.cdc — string builtins lower to runtime calls.
var greeting = string-append("hello, ", "world");
displayln(greeting);
displayln(string-length("hello"));
displayln(substring("hello world", 6, 11));
displayln(number->string(42));
