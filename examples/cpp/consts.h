// consts.h -- header-only C++ demo library for constants + statics.
// Self-contained like accum.h: everything inline, so the only extra
// translation unit is the generated shim (cdi_shim.cpp).
#pragma once

#include <string>

namespace dirac_demo {

// Plain (unscoped) enum: enumerators live in the namespace.
enum Color { RED = 1, GREEN = 2, BLUE = 4 };

// Scoped enum: needs an explicit cast to int (the shim does it).
enum class Mode : int { OFF = 0, ON = 1 };

// Macros and constexprs of each const type.
#define MAX_ITEMS 64
#define APP_NAME "dirac"
constexpr double PI_D = 3.14159;
constexpr bool VERBOSE = true;

// Static data members need no per-instance handle. `inline static`
// (C++17) keeps this header self-contained: declaration + definition.
class Config {
public:
    inline static int instances = 0;
    inline static std::string tag = "v1";
    static const int LIMIT = 10;

    static void bump() {
        instances++;
    }
};

} // namespace dirac_demo
