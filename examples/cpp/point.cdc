// point.cdc -- struct-by-pointer: structs ride the class bridge.
//
// run:
//   cdirac build examples\cpp\point.cdc -o point -I examples\cpp
//   ./point
// expect: 25, 3, 7, P(3,4)

[include-cpp "point.h"]
[cpp-class Point "dirac_demo::Point" "point.h"]
[cpp-ctor Point [int int]]
[cpp-method Point norm2 [] int]
[cpp-static Point make [int int] Point]
[cpp-field Point x int]
[cpp-field Point y int]
[extern-cpp describe "dirac_demo::describe" [Point string] string]

var p|ptr = Point(3, 4);
displayln(p.norm2());
displayln(p.x);
set p.x = 0;
set p.y = 7;
displayln(p.y);
var q|ptr = Point.make(3, 4);
displayln(describe(q, "P"));
Point_delete(p);
Point_delete(q);
