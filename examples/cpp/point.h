// Point demo: plain C++ structs ride the class bridge.
//
// A struct is a class with public fields: declare it with `cpp-class`
// (qualified name + header), construct by pointer, read/write fields.
// True by-value struct params/returns stay out (ABI-fragile across
// MSVC/Itanium); the supported pattern is pointer handles plus a tiny
// header-side factory for value construction.
#pragma once
#include <cstdint>
#include <string>

namespace dirac_demo {

struct Point {
  int32_t x;
  int32_t y;
  Point() : x(0), y(0) {}
  Point(int32_t x_, int32_t y_) : x(x_), y(y_) {}
  int32_t norm2() const { return x * x + y * y; }
  static Point *make(int32_t x, int32_t y) { return new Point(x, y); }
};

inline const char *describe(const Point *p, const std::string &tag) {
  static std::string buf;
  buf = tag + "(" + std::to_string(p->x) + "," + std::to_string(p->y) + ")";
  return buf.c_str();
}

}  // namespace dirac_demo
