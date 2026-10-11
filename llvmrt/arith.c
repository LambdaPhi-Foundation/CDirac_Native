/* arith.c -- wrapping ints, truncating div, coerced comparisons. */
#include <stdint.h>
#include <stdio.h>
#include <math.h>

#include "arith.h"
#include "value.h"
static double to_double(const char *who, Value *v) {
    if (v->tag == CDI_INT) {
        return (double)v->ival;
    }
    if (v->tag == CDI_FLOAT) {
        return v->fval;
    }
    char *got = cdi_write_cstr(v);
    char msg[512];
    snprintf(msg, sizeof(msg), "%s: expects numbers, got: %s", who, got);
    cdi_error(msg);
    return 0; /* unreachable */
}

static int is_float_val(Value *v) {
    return v->tag == CDI_FLOAT;
}

/* Wrapping helpers (unsigned arithmetic: well-defined). */
static int32_t w_add(int32_t a, int32_t b) {
    return (int32_t)((uint32_t)a + (uint32_t)b);
}

static int32_t w_sub(int32_t a, int32_t b) {
    return (int32_t)((uint32_t)a - (uint32_t)b);
}

static int32_t w_mul(int32_t a, int32_t b) {
    return (int32_t)((uint32_t)a * (uint32_t)b);
}

Value *cdi_add_n(Value **args, int32_t n) {
    int any_float = 0;
    for (int32_t i = 0; i < n; i++) {
        if (!is_float_val(args[i]) && args[i]->tag != CDI_INT) {
            to_double("+", args[i]);
        }
        any_float = any_float || is_float_val(args[i]);
    }
    if (any_float) {
        double acc = 0.0;
        for (int32_t i = 0; i < n; i++) {
            acc += to_double("+", args[i]);
        }
        return cdi_mk_float(acc);
    }
    int32_t acc = 0;
    for (int32_t i = 0; i < n; i++) {
        acc = w_add(acc, args[i]->ival);
    }
    return cdi_mk_int(acc);
}

Value *cdi_sub_n(Value **args, int32_t n) {
    if (n < 1) {
        cdi_error("-: expects at least 1 argument, got 0");
    }
    for (int32_t i = 0; i < n; i++) {
        if (args[i]->tag != CDI_INT && !is_float_val(args[i])) {
            to_double("-", args[i]);
        }
    }
    int any_float = 0;
    for (int32_t i = 0; i < n; i++) {
        any_float = any_float || is_float_val(args[i]);
    }
    if (n == 1) { /* negation */
        if (any_float) {
            return cdi_mk_float(-to_double("-", args[0]));
        }
        return cdi_mk_int(w_sub(0, args[0]->ival));
    }
    if (any_float) {
        double acc = to_double("-", args[0]);
        for (int32_t i = 1; i < n; i++) {
            acc -= to_double("-", args[i]);
        }
        return cdi_mk_float(acc);
    }
    int32_t acc = args[0]->ival;
    for (int32_t i = 1; i < n; i++) {
        acc = w_sub(acc, args[i]->ival);
    }
    return cdi_mk_int(acc);
}

Value *cdi_mul_n(Value **args, int32_t n) {
    int any_float = 0;
    for (int32_t i = 0; i < n; i++) {
        if (args[i]->tag != CDI_INT && !is_float_val(args[i])) {
            to_double("*", args[i]);
        }
        any_float = any_float || is_float_val(args[i]);
    }
    if (any_float) {
        double acc = 1.0;
        for (int32_t i = 0; i < n; i++) {
            acc *= to_double("*", args[i]);
        }
        return cdi_mk_float(acc);
    }
    int32_t acc = 1;
    for (int32_t i = 0; i < n; i++) {
        acc = w_mul(acc, args[i]->ival);
    }
    return cdi_mk_int(acc);
}

static Value *div2(Value *a, Value *b) {
    if (is_float_val(a) || is_float_val(b)) {
        double x = to_double("/", a);
        double y = to_double("/", b);
        if (y == 0.0) {
            cdi_error("/: division by zero");
        }
        return cdi_mk_float(x / y);
    }
    if (a->tag != CDI_INT || b->tag != CDI_INT) {
        to_double("/", a->tag != CDI_INT ? a : b);
    }
    int32_t x = a->ival, y = b->ival;
    if (y == 0) {
        cdi_error("/: division by zero");
    }
    if (x == INT32_MIN && y == -1) {
        return cdi_mk_int(INT32_MIN); /* wraps, never traps */
    }
    return cdi_mk_int(x / y); /* C truncates toward zero */
}

Value *cdi_div_n(Value **args, int32_t n) {
    if (n < 1) {
        cdi_error("/: expects at least 1 argument, got 0");
    }
    if (n == 1) { /* reciprocal: 1/x */
        return div2(cdi_mk_int(1), args[0]);
    }
    Value *acc = div2(args[0], args[1]);
    for (int32_t i = 2; i < n; i++) {
        acc = div2(acc, args[i]);
    }
    return acc;
}

/* Pairwise chains with int<->float coercion; <2 args is vacuously true. */
typedef int (*cmp_fn)(double, double);

static int ceq(double a, double b) { return a == b; }
static int clt(double a, double b) { return a < b; }
static int cgt(double a, double b) { return a > b; }
static int cle(double a, double b) { return a <= b; }
static int cge(double a, double b) { return a >= b; }

static Value *chain(const char *who, Value **args, int32_t n, cmp_fn f) {
    for (int32_t i = 0; i < n; i++) {
        if (args[i]->tag != CDI_INT && args[i]->tag != CDI_FLOAT) {
            to_double(who, args[i]);
        }
    }
    for (int32_t i = 0; i + 1 < n; i++) {
        if (!f(to_double(who, args[i]), to_double(who, args[i + 1]))) {
            return cdi_mk_bool(0);
        }
    }
    return cdi_mk_bool(1);
}

Value *cdi_eq_n(Value **args, int32_t n) {
    return chain("=", args, n, ceq);
}

Value *cdi_lt_n(Value **args, int32_t n) {
    return chain("<", args, n, clt);
}

Value *cdi_gt_n(Value **args, int32_t n) {
    return chain(">", args, n, cgt);
}

Value *cdi_le_n(Value **args, int32_t n) {
    return chain("<=", args, n, cle);
}

Value *cdi_ge_n(Value **args, int32_t n) {
    return chain(">=", args, n, cge);
}

Value *cdi_not(Value *v) {
    return cdi_mk_bool(!cdi_truthy(v));
}

/* ------------------------------------------------------------ N1 arith */

static int32_t expect_int2(const char *who, Value *v) {
    if (v->tag != CDI_INT) {
        char *got = cdi_write_cstr(v);
        char msg[512];
        snprintf(msg, sizeof(msg), "%s: expects an integer, got: %s", who, got);
        cdi_error(msg);
    }
    return v->ival;
}

/* Truncating int division with the reference's wrap rule (MIN/-1->MIN,
 * never traps) and per-head div-zero message. */
static int32_t int_div(const char *who, int32_t x, int32_t y) {
    if (y == 0) {
        char msg[256];
        snprintf(msg, sizeof(msg), "%s: division by zero", who);
        cdi_error(msg);
    }
    if (x == INT32_MIN && y == -1) {
        return INT32_MIN;
    }
    return x / y; /* C truncates toward zero */
}

Value *cdi_quotient(Value *a, Value *b) {
    return cdi_mk_int(int_div("quotient", expect_int2("quotient", a), expect_int2("quotient", b)));
}

static int32_t int_rem(const char *who, int32_t x, int32_t y) {
    if (y == 0) {
        char msg[256];
        snprintf(msg, sizeof(msg), "%s: division by zero", who);
        cdi_error(msg);
    }
    if (x == INT32_MIN && y == -1) {
        return 0; /* wrapping %: sign follows the dividend */
    }
    return x % y;
}

Value *cdi_remainder(Value *a, Value *b) {
    return cdi_mk_int(int_rem("remainder", expect_int2("remainder", a), expect_int2("remainder", b)));
}

Value *cdi_modulo(Value *a, Value *b) {
    /* Alias of `%` (Racket divisor-sign fixup deleted, like the reference). */
    return cdi_mk_int(int_rem("modulo", expect_int2("modulo", a), expect_int2("modulo", b)));
}

Value *cdi_add1(Value *v) {
    if (v->tag == CDI_INT) {
        return cdi_mk_int(w_add(v->ival, 1));
    }
    if (v->tag == CDI_FLOAT) {
        return cdi_mk_float(v->fval + 1.0);
    }
    char *got = cdi_write_cstr(v);
    char msg[512];
    snprintf(msg, sizeof(msg), "add1: expects a number, got: %s", got);
    cdi_error(msg);
    return cdi_nil(); /* unreachable */
}

Value *cdi_sub1(Value *v) {
    if (v->tag == CDI_INT) {
        return cdi_mk_int(w_sub(v->ival, 1));
    }
    if (v->tag == CDI_FLOAT) {
        return cdi_mk_float(v->fval - 1.0);
    }
    char *got = cdi_write_cstr(v);
    char msg[512];
    snprintf(msg, sizeof(msg), "sub1: expects a number, got: %s", got);
    cdi_error(msg);
    return cdi_nil(); /* unreachable */
}

Value *cdi_abs(Value *v) {
    if (v->tag == CDI_INT) {
        if (v->ival < 0) {
            return cdi_mk_int(w_sub(0, v->ival)); /* MIN stays MIN */
        }
        return cdi_mk_int(v->ival);
    }
    if (v->tag == CDI_FLOAT) {
        return cdi_mk_float(fabs(v->fval));
    }
    char *got = cdi_write_cstr(v);
    char msg[512];
    snprintf(msg, sizeof(msg), "abs: expects a number, got: %s", got);
    cdi_error(msg);
    return cdi_nil(); /* unreachable */
}

/* max/min keep the winning *original* value, ties keeping the first. */
static Value *minmax(const char *who, Value **args, int32_t n, int want_max) {
    if (n < 1) {
        char msg[256];
        snprintf(msg, sizeof(msg), "%s: expects at least 1 argument", who);
        cdi_error(msg);
    }
    Value *best = args[0];
    double b = to_double(who, best);
    for (int32_t i = 1; i < n; i++) {
        double x = to_double(who, args[i]);
        if ((want_max && x > b) || (!want_max && x < b)) {
            best = args[i];
            b = x;
        }
    }
    return best;
}

Value *cdi_max_n(Value **args, int32_t n) {
    return minmax("max", args, n, 1);
}

Value *cdi_min_n(Value **args, int32_t n) {
    return minmax("min", args, n, 0);
}

Value *cdi_expt(Value *b, Value *e) {
    to_double("expt", b);
    to_double("expt", e);
    if (b->tag == CDI_INT && e->tag == CDI_INT && e->ival >= 0) {
        int32_t r = 1, m = b->ival, n = e->ival;
        while (n > 0) {
            if (n & 1) {
                r = w_mul(r, m);
            }
            m = w_mul(m, m);
            n >>= 1;
        }
        return cdi_mk_int(r);
    }
    return cdi_mk_float(pow(to_double("expt", b), to_double("expt", e)));
}

Value *cdi_sqrt(Value *v) {
    double x = to_double("sqrt", v);
    if (x < 0.0) {
        cdi_error("sqrt: negative argument");
    }
    return cdi_mk_float(sqrt(x));
}
