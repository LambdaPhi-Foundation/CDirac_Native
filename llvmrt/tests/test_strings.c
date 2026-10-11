/* test_strings.c -- units, slices, case mapping. */
#include <stdio.h>
#include <string.h>

#include "../containers.h"
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
    /* UTF-16 units: ASCII 1, é 1, astral 2 */
    CHECK_INT(cdi_str_units("hello", 5), 5, "units-ascii");
    CHECK_INT(cdi_str_units("\xc3\xa9", 2), 1, "units-e-acute");
    /* U+1D11E MUSICAL SYMBOL G-CLEF: F0 9D 84 9E */
    CHECK_INT(cdi_str_units("A\xf0\x9d\x84\x9e" "B", 6), 4, "units-astral");

    /* slices incl. astral split -> lone half prints as `?` */
    {
        const char *s = "A\xf0\x9d\x84\x9e" "B"; /* units: A, hi, lo, B */
        int32_t n = 0;
        char *sl = cdi_str_slice(s, 6, 0, 4, &n);
        Value *v = cdi_mk_str(sl, n);
        CHECK_INT(n, 6, "slice-full-nbytes");
        CHECK_STR(cdi_display_cstr(v), "A\xf0\x9d\x84\x9e"
                                       "B",
                  "slice-full-display");
        /* split between hi and lo: [1,3) keeps hi,lo? [1,2) keeps hi only */
        char *half = cdi_str_slice(s, 6, 1, 2, &n);
        Value *h = cdi_mk_str(half, n);
        CHECK_INT(cdi_str_units(half, n), 1, "slice-half-units");
        CHECK_STR(cdi_display_cstr(h), "?", "slice-half-prints-?");
        CHECK_STR(cdi_write_cstr(h), "\"?\"", "slice-half-write-?");
    }

    /* case mapping incl. ß -> SS */
    {
        int32_t n = 0;
        char *up = cdi_str_upcase("LOUD", 4, &n);
        CHECK_INT(n, 4, "up-len");
        CHECK_STR(cdi_display_cstr(cdi_mk_str(up, n)), "LOUD", "up-ascii");
    }
    {
        /* ß = C3 9F */
        int32_t n = 0;
        char *up = cdi_str_upcase("stra\xc3\x9f" "e", 7, &n);
        CHECK_STR(cdi_display_cstr(cdi_mk_str(up, n)), "STRASSE", "up-eszett");
    }
    {
        int32_t n = 0;
        char *down = cdi_str_downcase("Brac", 4, &n);
        CHECK_STR(cdi_display_cstr(cdi_mk_str(down, n)), "brac", "down-ascii");
    }

    /* containers behind len/get */
    {
        Value *arr = cdi_array0();
        cdi_arr_push(arr, cdi_mk_int(10));
        cdi_arr_push(arr, cdi_mk_int(20));
        CHECK_STR(cdi_write_cstr(cdi_len(arr)), "2", "arr-len");
        CHECK_STR(cdi_write_cstr(cdi_get(arr, cdi_mk_int(1))), "20", "arr-get");
        /* aliasing: second handle sees the push */
        cdi_arr_push(arr, cdi_mk_int(30));
        CHECK_STR(cdi_write_cstr(arr), "[10 20 30]", "arr-alias-write");
        Value *q = cdi_queue0();
        cdi_qpush(q, cdi_mk_str("x", 1));
        CHECK_STR(cdi_write_cstr(cdi_len(q)), "1", "queue-len");
        CHECK_STR(cdi_write_cstr(cdi_get(q, cdi_mk_int(0))), "\"x\"", "queue-get");
        CHECK_STR(cdi_write_cstr(q), "[queue \"x\"]", "queue-write");
    }

    if (failures == 0) {
        printf("test_strings: all passed\n");
    }
    return failures ? 1 : 0;
}
