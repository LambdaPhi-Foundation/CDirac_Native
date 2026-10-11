/* pred.c -- numeric/type predicates + deep equality (N1). */
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <math.h>

#include "pred.h"
#include "value.h"

static double to_num(const char *who, Value *v) {
    if (v->tag == CDI_INT) {
        return (double)v->ival;
    }
    if (v->tag == CDI_FLOAT) {
        return v->fval;
    }
    char *got = cdi_write_cstr(v);
    char msg[512];
    snprintf(msg, sizeof(msg), "%s: expects a number, got: %s", who, got);
    cdi_error(msg);
    return 0; /* unreachable */
}

static int32_t expect_whole_int(const char *who, Value *v) {
    if (v->tag != CDI_INT) {
        char *got = cdi_write_cstr(v);
        char msg[512];
        snprintf(msg, sizeof(msg), "%s: expects an integer, got: %s", who, got);
        cdi_error(msg);
    }
    return v->ival;
}

Value *cdi_zero_p(Value *v) {
    return cdi_mk_bool(to_num("zero?", v) == 0.0);
}

Value *cdi_positive_p(Value *v) {
    return cdi_mk_bool(to_num("positive?", v) > 0.0);
}

Value *cdi_negative_p(Value *v) {
    return cdi_mk_bool(to_num("negative?", v) < 0.0);
}

Value *cdi_even_p(Value *v) {
    int32_t n = expect_whole_int("even?", v);
    /* wrapping_rem: sign follows the dividend; evenness is exact. */
    return cdi_mk_bool(n % 2 == 0);
}

Value *cdi_odd_p(Value *v) {
    int32_t n = expect_whole_int("odd?", v);
    return cdi_mk_bool(n % 2 != 0);
}

/* Deep structural equality with int/float coercion. Array never equals
 * Queue; handles compare by identity; everything else by shape. */
static int value_equal(const Value *a, const Value *b) {
    if (a->tag == CDI_INT && b->tag == CDI_INT) {
        return a->ival == b->ival;
    }
    if (a->tag == CDI_FLOAT && b->tag == CDI_FLOAT) {
        return a->fval == b->fval;
    }
    if (a->tag == CDI_INT && b->tag == CDI_FLOAT) {
        return (double)a->ival == b->fval;
    }
    if (a->tag == CDI_FLOAT && b->tag == CDI_INT) {
        return a->fval == (double)b->ival;
    }
    if (a->tag == CDI_BOOL && b->tag == CDI_BOOL) {
        return (a->bval != 0) == (b->bval != 0);
    }
    if (a->tag == CDI_STR && b->tag == CDI_STR) {
        const StrData *x = (const StrData *)a->ptr;
        const StrData *y = (const StrData *)b->ptr;
        if (x->nbytes != y->nbytes) {
            return 0;
        }
        return memcmp(x->bytes, y->bytes, (size_t)x->nbytes) == 0;
    }
    if (a->tag == CDI_NIL && b->tag == CDI_NIL) {
        return 1;
    }
    if (a->tag == CDI_VOID && b->tag == CDI_VOID) {
        return 1;
    }
    if (a->tag == CDI_ARRAY && b->tag == CDI_ARRAY) {
        const ArrData *x = (const ArrData *)a->ptr;
        const ArrData *y = (const ArrData *)b->ptr;
        if (x->len != y->len) {
            return 0;
        }
        for (int32_t i = 0; i < x->len; i++) {
            if (!value_equal(x->items[i], y->items[i])) {
                return 0;
            }
        }
        return 1;
    }
    if (a->tag == CDI_QUEUE && b->tag == CDI_QUEUE) {
        const QueData *x = (const QueData *)a->ptr;
        const QueData *y = (const QueData *)b->ptr;
        if (x->len != y->len) {
            return 0;
        }
        for (int32_t i = 0; i < x->len; i++) {
            if (!value_equal(x->items[i], y->items[i])) {
                return 0;
            }
        }
        return 1;
    }
    if (a->tag == CDI_PTR && b->tag == CDI_PTR) {
        return a->ptr == b->ptr;
    }
    return 0;
}

Value *cdi_equal_p(Value *a, Value *b) {
    return cdi_mk_bool(value_equal(a, b));
}

Value *cdi_number_p(Value *v) {
    return cdi_mk_bool(v->tag == CDI_INT || v->tag == CDI_FLOAT);
}

Value *cdi_real_p(Value *v) {
    return cdi_mk_bool(v->tag == CDI_INT || v->tag == CDI_FLOAT);
}

Value *cdi_integer_p(Value *v) {
    if (v->tag == CDI_INT) {
        return cdi_mk_bool(1);
    }
    if (v->tag == CDI_FLOAT) {
        double x = v->fval;
        /* Whole and fitting in i32 (mirrors the reference gate). */
        return cdi_mk_bool(x == trunc(x) && x >= -2147483648.0 && x <= 2147483647.0);
    }
    return cdi_mk_bool(0);
}

Value *cdi_boolean_p(Value *v) {
    return cdi_mk_bool(v->tag == CDI_BOOL);
}

Value *cdi_string_p(Value *v) {
    return cdi_mk_bool(v->tag == CDI_STR);
}

Value *cdi_array_p(Value *v) {
    return cdi_mk_bool(v->tag == CDI_ARRAY);
}

Value *cdi_queue_p(Value *v) {
    return cdi_mk_bool(v->tag == CDI_QUEUE);
}

Value *cdi_empty_p(Value *v) {
    if (v->tag == CDI_ARRAY) {
        return cdi_mk_bool(((const ArrData *)v->ptr)->len == 0);
    }
    if (v->tag == CDI_QUEUE) {
        return cdi_mk_bool(((const QueData *)v->ptr)->len == 0);
    }
    if (v->tag == CDI_STR) {
        return cdi_mk_bool(((const StrData *)v->ptr)->nbytes == 0);
    }
    char *got = cdi_write_cstr(v);
    char msg[512];
    snprintf(msg, sizeof(msg), "empty?: expects an array, a queue or a string, got: %s", got);
    cdi_error(msg);
    return cdi_nil(); /* unreachable */
}

Value *cdi_void_p(Value *v) {
    return cdi_mk_bool(v->tag == CDI_VOID);
}
