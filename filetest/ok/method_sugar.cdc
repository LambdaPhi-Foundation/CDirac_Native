// method_sugar.cdc — dot-call sugar over a class handle.
[include-cpp "accum.h"]
[cpp-class Accum "demo::Accum" "accum.h"]
[cpp-ctor Accum [int]]
[cpp-method Accum push [int] void]
[cpp-method Accum total [] int]
var a|ptr = Accum(3);
a.push(4);
displayln(a.total());
Accum_delete(a);
