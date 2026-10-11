/* test_value.c -- ctors, truthiness, write/display forms. */
#include <math.h>
#include <stdio.h>
#include <string.h>

#include "../strings.h"
#include "../value.h"

static int failures = 0;

#define CHECK_STR(got_expr, want, name)                                     \
    do {                                                                    \
        char *got = (got_expr);                                             \
        if (strcmp(got, want) != 0) {                                       \
            printf("FAIL %s: got [%s] want [%s]\n", name, got, want);        \
            failures++;                                                     \
        }                                                                   \
    } while (0)

#define CHECK_INT(got_expr, want, name)                                     \
    do {                                                                    \
        long got = (long)(got_expr);                                        \
        if (got != (long)(want)) {                                          \
            printf("FAIL %s: got %ld want %ld\n", name, got, (long)(want));  \
            failures++;                                                     \
        }                                                                   \
    } while (0)

int main(void) {
    /* tags + write forms */
    CHECK_STR(cdi_write_cstr(cdi_mk_int(42)), "42", "int");
    CHECK_STR(cdi_write_cstr(cdi_mk_int(-2147483647 - 1)), "-2147483648", "int-min");
    CHECK_STR(cdi_write_cstr(cdi_mk_bool(1)), "true", "bool-t");
    CHECK_STR(cdi_write_cstr(cdi_mk_bool(0)), "false", "bool-f");
    CHECK_STR(cdi_write_cstr(cdi_nil()), "null", "nil");
    CHECK_STR(cdi_write_cstr(cdi_void()), "#<void>", "void");
    CHECK_STR(cdi_write_cstr(cdi_mk_str("hi", 2)), "\"hi\"", "str-write");
    CHECK_STR(cdi_display_cstr(cdi_mk_str("hi", 2)), "hi", "str-display");
    CHECK_STR(cdi_write_cstr(cdi_mk_str("a\n\"b\\c", 6)), "\"a\\n\\\"b\\\\c\"",
              "str-escapes");

    /* truthiness: only false is falsy */
    CHECK_INT(cdi_truthy(cdi_mk_bool(0)), 0, "falsy-false");
    CHECK_INT(cdi_truthy(cdi_mk_bool(1)), 1, "truthy-true");
    CHECK_INT(cdi_truthy(cdi_mk_int(0)), 1, "truthy-zero");
    CHECK_INT(cdi_truthy(cdi_mk_str("", 0)), 1, "truthy-empty-str");
    CHECK_INT(cdi_truthy(cdi_nil()), 1, "truthy-nil");

    /* float rendering (Double.toString shapes) */
    CHECK_STR(cdi_float_str(9.0), "9.0", "float-9");
    CHECK_STR(cdi_float_str(1e21), "1.0E21", "float-1e21");
    CHECK_STR(cdi_float_str(0.0), "0.0", "float-zero");
    CHECK_STR(cdi_float_str(-0.0), "-0.0", "float-negzero");
    CHECK_STR(cdi_float_str(1.0 / 0.0), "Infinity", "float-inf");
    CHECK_STR(cdi_float_str(-1.0 / 0.0), "-Infinity", "float-neginf");
    CHECK_STR(cdi_float_str(0.0 / 0.0), "NaN", "float-nan");
    CHECK_STR(cdi_float_str(3.5), "3.5", "float-3.5");
    CHECK_STR(cdi_float_str(-2.5), "-2.5", "float-neg");
    CHECK_STR(cdi_float_str(0.001), "0.001", "float-1e-3");
    CHECK_STR(cdi_float_str(0.0001), "1.0E-4", "float-1e-4");
    CHECK_STR(cdi_float_str(10000000.0), "1.0E7", "float-1e7");
    CHECK_STR(cdi_float_str(1234567.0), "1234567.0", "float-1234567");
    CHECK_STR(cdi_float_str(12345678.0), "1.2345678E7", "float-12345678");
    CHECK_STR(cdi_float_str(1e15), "1.0E15", "float-1e15");
    CHECK_STR(cdi_float_str(0.1 + 0.2), "0.30000000000000004", "float-01+02");
    CHECK_STR(cdi_float_str(3.14159265), "3.14159265", "float-pi");
    CHECK_STR(cdi_write_cstr(cdi_mk_float(2.0)), "2.0", "float-write");

    if (failures == 0) {
        printf("test_value: all passed\n");
    }
    return failures ? 1 : 0;
}
