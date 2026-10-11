// accum.h -- header-only C++ demo library for Dirac native interop.
// Self-contained (no .cpp to link): everything is inline, so the only
// extra translation unit is the generated shim (cdi_shim.cpp).
#pragma once

#include <cctype>
#include <string>

namespace dirac_demo {

// Free functions (bound via [extern-cpp ...]).
inline int add(int a, int b) {
    return a + b;
}

inline double scale(double x, double f) {
    return x * f;
}

inline std::string shout(std::string s) {
    for (size_t i = 0; i < s.size(); i++) {
        s[i] = (char)std::toupper((unsigned char)s[i]);
    }
    return s;
}

// Opaque class (bound via [cpp-class]/[cpp-ctor]/[cpp-method]/...).
// Dirac holds `Accum*` as an untyped handle; the shim casts back.
class Accum {
public:
    explicit Accum(int start) : total_(start), count_(0) {}

    void add(int v) {
        total_ += v;
        count_++;
    }

    int total() {
        return total_;
    }

    static Accum *make(int start) {
        return new Accum(start);
    }

    int count_;

private:
    int total_;
};

} // namespace dirac_demo
