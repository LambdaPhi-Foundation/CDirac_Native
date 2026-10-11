/* test_arith.c -- wrapping, truncation, coercion, chains. */
#include <stdio.h>
#include <string.h>

#include "../arith.h"
#include "../value.h"

static int failures = 0;

static Value *I(int32_t x) {
    return cdi_mk_int(x);
}

static Value *F(double x) {
    return cdi_mk_float(x);
}

#define CHECK_WRITE(call_expr, want, name)                                  \
    do {                                                                    \
        char *got = cdi_write_cstr(call_expr);                              \
        if (strcmp(got, want) != 0) {                                       \
            printf("FAIL %s: got [%s] want [%s]\n", name, got, want);        \
            failures++;                                                     \
        }                                                                   \
    } while (0)

int main(void) {
    /* wrapping */
    {
        Value *a[] = {I(2147483647), I(1)};
        CHECK_WRITE(cdi_add_n(a, 2), "-2147483648", "wrap-add");
    }
    {
        Value *a[] = {I(-2147483647 - 1), I(-1)};
        CHECK_WRITE(cdi_div_n(a, 2), "-2147483648", "min-div-neg1");
    }
    {
        Value *a[] = {I(-2147483647 - 1), I(-1)};
        CHECK_WRITE(cdi_mul_n(a, 2), "-2147483648", "min-mul-neg1");
    }
    {
        Value *a[] = {I(-2147483647 - 1)};
        CHECK_WRITE(cdi_sub_n(a, 1), "-2147483648", "min-neg-wraps");
    }
    /* truncation + reciprocal + float rule */
    {
        Value *a[] = {I(7), I(2)};
        CHECK_WRITE(cdi_div_n(a, 2), "3", "div-trunc");
    }
    {
        Value *a[] = {I(-7), I(2)};
        CHECK_WRITE(cdi_div_n(a, 2), "-3", "div-neg");
    }
    {
        Value *a[] = {I(5)};
        CHECK_WRITE(cdi_div_n(a, 1), "0", "div-recip");
    }
    {
        Value *a[] = {F(7.0), I(2)};
        CHECK_WRITE(cdi_div_n(a, 2), "3.5", "div-float");
    }
    {
        Value *a[] = {I(10), I(3), I(2)};
        CHECK_WRITE(cdi_sub_n(a, 3), "5", "sub-nary");
    }
    {
        Value *noargs[] = {NULL};
        CHECK_WRITE(cdi_add_n(noargs, 0), "0", "add-empty");
        CHECK_WRITE(cdi_mul_n(noargs, 0), "1", "mul-empty");
    }
    /* comparisons: chains + coercion */
    {
        Value *a[] = {I(1), I(2), I(3)};
        CHECK_WRITE(cdi_lt_n(a, 3), "true", "lt-chain");
    }
    {
        Value *a[] = {I(4), I(4)};
        CHECK_WRITE(cdi_eq_n(a, 2), "true", "eq");
    }
    {
        Value *a[] = {I(2), F(2.0)};
        CHECK_WRITE(cdi_eq_n(a, 2), "true", "eq-coerce");
    }
    {
        Value *a[] = {I(2), F(2.5)};
        CHECK_WRITE(cdi_eq_n(a, 2), "false", "eq-coerce-false");
    }
    {
        Value *a[] = {I(5)};
        CHECK_WRITE(cdi_eq_n(a, 1), "true", "eq-single");
    }
    /* not: only false is falsy */
    CHECK_WRITE(cdi_not(I(0)), "false", "not-zero");
    CHECK_WRITE(cdi_not(cdi_mk_bool(0)), "true", "not-false");
    CHECK_WRITE(cdi_not(cdi_nil()), "false", "not-nil");

    if (failures == 0) {
        printf("test_arith: all passed\n");
    }
    return failures ? 1 : 0;
}
