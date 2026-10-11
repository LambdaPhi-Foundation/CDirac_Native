/* test_cinterop.c -- boxed Value <-> native conversions + ptr handles. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "../cinterop.h"
#include "../value.h"

static int failures = 0;

#define CHECK_INT(got_expr, want, name)                                     \
    do {                                                                    \
        long got = (long)(got_expr);                                        \
        if (got != (long)(want)) {                                          \
            printf("FAIL %s: got %ld want %ld\n", name, got, (long)(want));  \
            failures++;                                                     \
        }                                                                   \
    } while (0)

#define CHECK_STR(got_expr, want, name)                                     \
    do {                                                                    \
        char *got = (got_expr);                                             \
        if (strcmp(got, want) != 0) {                                       \
            printf("FAIL %s: got [%s] want [%s]\n", name, got, want);        \
            failures++;                                                     \
        }                                                                   \
    } while (0)

#define CHECK_TAG(v, want, name) CHECK_INT((v)->tag, want, name)

int main(void) {
    /* scalar round-trips */
    CHECK_INT(cdi_to_int(cdi_mk_int(-7)), -7, "to-int");
    CHECK_INT(cdi_to_int(cdi_mk_float(3.0)), 3, "float-to-int-whole");
    CHECK_INT((int)(cdi_to_double(cdi_mk_int(4)) * 10), 40, "int-to-double");
    CHECK_INT((int)cdi_to_double(cdi_mk_float(2.5)), 2, "to-double");
    CHECK_INT(cdi_to_bool(cdi_mk_bool(1)), 1, "to-bool-t");
    CHECK_INT(cdi_to_bool(cdi_mk_bool(0)), 0, "to-bool-f");
    CHECK_TAG(cdi_from_int(9), CDI_INT, "from-int-tag");
    CHECK_TAG(cdi_from_long(11L), CDI_INT, "from-long-tag");
    CHECK_INT(cdi_to_int(cdi_from_long(11L)), 11, "from-long-val");
    CHECK_TAG(cdi_from_double(1.5), CDI_FLOAT, "from-double-tag");
    CHECK_TAG(cdi_from_bool(0), CDI_BOOL, "from-bool-tag");

    /* strings: borrowed bytes + NUL copy + boxing */
    {
        Value *s = cdi_mk_str("hi", 2);
        int32_t n = 0;
        const char *b = cdi_str_bytes(s, &n);
        CHECK_INT(n, 2, "str-bytes-len");
        CHECK_INT(b[0] == 'h' && b[1] == 'i', 1, "str-bytes-val");
        char *c = cdi_to_cstring(s);
        CHECK_STR(c, "hi", "to-cstring");
        CHECK_TAG(cdi_from_cstring("yo"), CDI_STR, "from-cstring-tag");
        CHECK_STR(cdi_write_cstr(cdi_from_cstring("yo")), "\"yo\"", "from-cstring-val");
        CHECK_TAG(cdi_from_cstring(NULL), CDI_NIL, "from-null-cstring-nil");
        free(c);
    }

    /* opaque handles: identity, nil for NULL, write form */
    {
        int sentinel = 0;
        Value *h = cdi_from_ptr(&sentinel);
        CHECK_TAG(h, CDI_PTR, "ptr-tag");
        CHECK_INT(cdi_to_ptr(h) == &sentinel, 1, "ptr-identity");
        CHECK_INT(cdi_expect_handle(h, "t") == &sentinel, 1, "expect-handle");
        CHECK_TAG(cdi_from_ptr(NULL), CDI_NIL, "null-ptr-nil");
        CHECK_STR(cdi_write_cstr(h), "#<ptr>", "ptr-write");
        CHECK_STR(cdi_display_cstr(h), "#<ptr>", "ptr-display");
        CHECK_INT(cdi_truthy(h), 1, "ptr-truthy");
    }

    if (failures == 0) {
        printf("test_cinterop: all assertions passed\n");
    }
    return failures ? 1 : 0;
}
