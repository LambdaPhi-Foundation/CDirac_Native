// struct_basic.cdc — by-value struct decl + extern call (IR + shim only).
[include-cpp "s.h"]
[cpp-struct Point "ns::Point" "s.h" [x int] [y int]]
[extern-cpp move_by "ns::move_by" [Point int] Point]
displayln(move_by(array(3, 4), 10));
