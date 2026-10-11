// cb.h -- plain-C callbacks for the function-pointer bridge.
//
// Header-only on purpose: `static` gives each function internal
// linkage in the single shim TU that includes this header, and the
// generated wrappers call them directly (no link line changes).
#pragma once

// bool (not _Bool): this header is included from the C++ shim, where
// _Bool is not a keyword. Same 1-byte ABI on both sides.
#include <stdbool.h>

static int apply_twice(int (*f)(int), int x) {
    return f(f(x));
}

static int count_matching(int (*pred)(int), int lo, int hi) {
    int n = 0;
    for (int i = lo; i < hi; ++i) {
        if (pred(i)) {
            ++n;
        }
    }
    return n;
}

static int count_true(bool (*p)(int), int lo, int hi) {
    int n = 0;
    for (int i = lo; i < hi; ++i) {
        if (p(i)) {
            ++n;
        }
    }
    return n;
}

static void call_twice(void (*f)(int), int x) {
    f(x);
    f(x);
}
