// struct.h -- aggregate value types for the struct-by-value bridge.
//
// Aggregates on purpose: the generated shim builds them with
// `T{f0, f1, ...}` and reads `.field`, so any aggregate with
// matching field order works. (Non-aggregates fail in the shim with
// the host compiler's error -- the C++ compiler is the type checker.)
#pragma once

#include <cstdint>

namespace dirac_demo {

struct Point {
    int32_t x;
    int32_t y;
};

struct Labeled {
    Point at;
    int32_t tag;
};

inline Point move_by(Point p, int32_t dx, int32_t dy) {
    return {p.x + dx, p.y + dy};
}

inline int32_t manhattan(Point p) {
    int32_t ax = p.x < 0 ? -p.x : p.x;
    int32_t ay = p.y < 0 ? -p.y : p.y;
    return ax + ay;
}

inline Labeled label_at(Point p, int32_t tag) {
    return {p, tag};
}

} // namespace dirac_demo
