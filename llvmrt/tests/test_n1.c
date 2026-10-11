/* test_n1.c -- N1 builtins: int trio, small arith, predicates,
 * containers, strings, io shapes, machine memory. Abort paths
 * (div-zero, OOR, type errors) exit the process, so only value
 * paths are asserted here; aborts are covered end-to-end. */
#include <stdio.h>
#include <string.h>

#include "../arith.h"
#include "../containers.h"
#include "../io.h"
#include "../mem.h"
#include "../pred.h"
#include "../strings.h"
#include "../value.h"

static int failures = 0;

static Value *I(int32_t x) {
    return cdi_mk_int(x);
}

static Value *F(double x) {
    return cdi_mk_float(x);
}

static Value *S(const char *s) {
    return cdi_mk_str(s, (int32_t)strlen(s));
}

#define CHECK_WRITE(call_expr, want, name)                                  \
    do {                                                                    \
        char *got = cdi_write_cstr(call_expr);                              \
        if (strcmp(got, want) != 0) {                                       \
            printf("FAIL %s: got [%s] want [%s]\n", name, got, want);        \
            failures++;                                                     \
        }                                                                   \
    } while (0)

#define CHECK_TAG(call_expr, want_tag, name)                                \
    do {                                                                    \
        Value *v = (call_expr);                                             \
        if (v->tag != (want_tag)) {                                         \
            printf("FAIL %s: got tag %d want %d\n", name, v->tag, (want_tag)); \
            failures++;                                                     \
        }                                                                   \
    } while (0)

int main(void) {
    /* integer trio */
    CHECK_WRITE(cdi_quotient(I(7), I(2)), "3", "quot");
    CHECK_WRITE(cdi_quotient(I(-7), I(2)), "-3", "quot-neg");
    CHECK_WRITE(cdi_quotient(I(-2147483647 - 1), I(-1)), "-2147483648", "quot-min");
    CHECK_WRITE(cdi_remainder(I(-7), I(3)), "-1", "rem");
    CHECK_WRITE(cdi_modulo(I(7), I(-3)), "1", "mod");
    CHECK_WRITE(cdi_remainder(I(-2147483647 - 1), I(-1)), "0", "rem-min");
    /* small arith */
    CHECK_WRITE(cdi_add1(I(41)), "42", "add1");
    CHECK_WRITE(cdi_add1(F(1.5)), "2.5", "add1-f");
    CHECK_WRITE(cdi_sub1(I(0)), "-1", "sub1");
    CHECK_WRITE(cdi_abs(I(-2147483647 - 1)), "-2147483648", "abs-min");
    CHECK_WRITE(cdi_abs(F(-2.5)), "2.5", "abs-f");
    {
        Value *a[] = {I(2), F(2.0)};
        Value *m = cdi_max_n(a, 2);
        CHECK_WRITE(m, "2", "max-tie-first");
        CHECK_TAG(m, CDI_INT, "max-keeps-orig");
    }
    {
        Value *a[] = {F(1.5), I(2), I(-3)};
        CHECK_WRITE(cdi_min_n(a, 3), "-3", "min");
    }
    CHECK_WRITE(cdi_expt(I(2), I(10)), "1024", "expt-int");
    CHECK_WRITE(cdi_expt(I(2), I(-1)), "0.5", "expt-neg");
    CHECK_WRITE(cdi_sqrt(I(9)), "3.0", "sqrt");
    /* numeric predicates */
    CHECK_WRITE(cdi_zero_p(I(0)), "true", "zero");
    CHECK_WRITE(cdi_zero_p(F(0.5)), "false", "zero-f");
    CHECK_WRITE(cdi_positive_p(F(0.1)), "true", "pos");
    CHECK_WRITE(cdi_negative_p(I(-1)), "true", "neg");
    CHECK_WRITE(cdi_even_p(I(-4)), "true", "even-neg");
    CHECK_WRITE(cdi_odd_p(I(7)), "true", "odd");
    /* equality incl. coercion + container depth + array/queue split */
    CHECK_WRITE(cdi_equal_p(I(2), F(2.0)), "true", "eq-coerce");
    {
        Value *x[] = {I(1), S("a")};
        Value *y[] = {I(1), S("a")};
        CHECK_WRITE(cdi_equal_p(cdi_array_n(x, 2), cdi_array_n(y, 2)), "true", "eq-deep");
        CHECK_WRITE(cdi_equal_p(cdi_array_n(x, 1), cdi_queue_n(x, 1)), "false", "eq-arr-queue");
    }
    /* type predicates */
    CHECK_WRITE(cdi_number_p(F(1.5)), "true", "number");
    CHECK_WRITE(cdi_real_p(I(1)), "true", "real");
    CHECK_WRITE(cdi_integer_p(F(2.0)), "true", "whole-float");
    CHECK_WRITE(cdi_integer_p(F(2.5)), "false", "frac-float");
    CHECK_WRITE(cdi_integer_p(S("2")), "false", "int-str");
    CHECK_WRITE(cdi_boolean_p(cdi_mk_bool(1)), "true", "bool");
    CHECK_WRITE(cdi_string_p(S("x")), "true", "str");
    CHECK_WRITE(cdi_array_p(cdi_array0()), "true", "arr");
    CHECK_WRITE(cdi_queue_p(cdi_queue0()), "true", "queue");
    CHECK_WRITE(cdi_void_p(cdi_void()), "true", "void");
    CHECK_WRITE(cdi_empty_p(S("")), "true", "empty-str");
    CHECK_WRITE(cdi_empty_p(cdi_array0()), "true", "empty-arr");
    {
        Value *a[] = {I(1)};
        CHECK_WRITE(cdi_empty_p(cdi_array_n(a, 1)), "false", "nonempty");
    }
    /* containers */
    {
        Value *a[] = {I(1), I(2)};
        Value *arr = cdi_array_n(a, 2);
        CHECK_TAG(cdi_put(arr, I(0), I(9)), CDI_VOID, "put-void");
        CHECK_WRITE(cdi_get(arr, I(0)), "9", "put-write");
        CHECK_TAG(cdi_push(arr, I(3)), CDI_VOID, "push-void");
        CHECK_WRITE(cdi_len(arr), "3", "push-len");
        CHECK_WRITE(cdi_pop(arr), "3", "pop");
        CHECK_WRITE(cdi_len(arr), "2", "pop-len");
    }
    {
        Value *q = cdi_queue0();
        CHECK_TAG(cdi_qpush(q, I(1)), CDI_VOID, "qpush-void");
        cdi_qpush_front(q, I(0));
        CHECK_WRITE(cdi_qpeek(q), "0", "qpeek");
        CHECK_WRITE(cdi_qpeek_back(q), "1", "qpeek-back");
        CHECK_WRITE(cdi_qpop(q), "0", "qpop");
        CHECK_WRITE(cdi_qpop_back(q), "1", "qpop-back");
        CHECK_WRITE(cdi_len(q), "0", "queue-drained");
        Value *b[] = {I(5), I(6)};
        CHECK_WRITE(cdi_len(cdi_queue_n(b, 2)), "2", "queue-n");
    }
    /* strings */
    {
        Value *a[] = {S("foo"), S("bar")};
        CHECK_WRITE(cdi_string_append_n(a, 2), "\"foobar\"", "append");
        CHECK_WRITE(cdi_string_append_n(NULL, 0), "\"\"", "append-empty");
    }
    CHECK_WRITE(cdi_string_length(S("\xc3\xa9")), "1", "strlen-accent");
    CHECK_WRITE(cdi_substring(S("hello"), I(1), I(4), 1), "\"ell\"", "substr");
    CHECK_WRITE(cdi_substring(S("hello"), I(2), cdi_nil(), 0), "\"llo\"", "substr-tail");
    CHECK_WRITE(cdi_string_upcase(S("hi")), "\"HI\"", "upcase");
    CHECK_WRITE(cdi_string_downcase(S("HI")), "\"hi\"", "downcase");
    CHECK_WRITE(cdi_number_to_string(I(42)), "\"42\"", "n2s-int");
    CHECK_WRITE(cdi_number_to_string(F(1e21)), "\"1.0E21\"", "n2s-float");
    CHECK_WRITE(cdi_string_to_number(S("42")), "42", "s2n-int");
    CHECK_WRITE(cdi_string_to_number(S("2147483648")), "2.147483648E9", "s2n-overflow");
    CHECK_WRITE(cdi_string_to_number(S("  3.5 ")), "3.5", "s2n-trim-float");
    CHECK_WRITE(cdi_string_to_number(S("abc")), "false", "s2n-false");
    CHECK_WRITE(cdi_string_to_number(S("")), "false", "s2n-empty");
    CHECK_WRITE(cdi_string_to_number(S("inf")), "false", "s2n-inf");
    /* io shapes (no stdin interaction here) */
    CHECK_TAG(cdi_print(S("x")), CDI_VOID, "print-void");
    CHECK_TAG(cdi_newline(), CDI_VOID, "newline-void");
    /* machine memory roundtrip */
    {
        Value *h = cdi_machine_alloc(I(16));
        CHECK_TAG(h, CDI_PTR, "alloc-ptr");
        CHECK_TAG(cdi_machine_dealloc(h), CDI_VOID, "dealloc-void");
    }

    if (failures == 0) {
        printf("test_n1: all assertions passed\n");
    }
    return failures ? 1 : 0;
}
