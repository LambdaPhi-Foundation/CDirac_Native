/* cinterop.c -- boxed Value <-> native conversions for generated shims. */
#include <limits.h>
#include <stdio.h>

#include "cinterop.h"

int32_t cdi_to_int(Value *v) {
    if (v->tag == CDI_INT) {
        return v->ival;
    }
    if (v->tag == CDI_FLOAT) {
        double x = v->fval;
        if (x > 2147483647.0 || x < -2147483648.0 || x != (double)(int32_t)x) {
            cdi_error("cannot pass non-integral float as int");
        }
        return (int32_t)x;
    }
    cdi_error("cannot pass non-number value as int");
    return 0;
}

double cdi_to_double(Value *v) {
    if (v->tag == CDI_FLOAT) {
        return v->fval;
    }
    if (v->tag == CDI_INT) {
        return (double)v->ival;
    }
    cdi_error("cannot pass non-number value as double");
    return 0.0;
}

int cdi_to_bool(Value *v) {
    if (v->tag == CDI_BOOL) {
        return v->bval ? 1 : 0;
    }
    cdi_error("cannot pass non-boolean value as bool");
    return 0;
}

const char *cdi_str_bytes(Value *v, int32_t *out_nbytes) {
    if (v->tag != CDI_STR) {
        cdi_error("cannot pass non-string value as string");
    }
    StrData *s = (StrData *)v->ptr;
    if (out_nbytes) {
        *out_nbytes = s->nbytes;
    }
    return s->bytes;
}

char *cdi_to_cstring(Value *v) {
    int32_t n = 0;
    const char *b = cdi_str_bytes(v, &n);
    char *out = (char *)CDI_MALLOC((size_t)n + 1);
    for (int32_t i = 0; i < n; i++) {
        out[i] = b[i];
    }
    out[n] = '\0';
    return out;
}

Value *cdi_from_int(int32_t x) {
    return cdi_mk_int(x);
}

Value *cdi_from_long(long x) {
    if (x > INT32_MAX || x < INT32_MIN) {
        cdi_error("long value out of range for int");
    }
    return cdi_mk_int((int32_t)x);
}

Value *cdi_from_double(double x) {
    return cdi_mk_float(x);
}

Value *cdi_from_bool(int b) {
    return cdi_mk_bool(b ? 1 : 0);
}

Value *cdi_from_cstring(const char *s) {
    int32_t n;
    const char *b;
    if (s == NULL) {
        return cdi_nil();
    }
    n = 0;
    b = s;
    while (*b++) {
        n++;
    }
    return cdi_mk_str(s, n);
}

Value *cdi_from_ptr(void *p) {
    if (p == NULL) {
        return cdi_nil();
    }
    return cdi_mk_ptr(p);
}

void *cdi_expect_handle(Value *v, const char *what) {
    if (v->tag != CDI_PTR) {
        char buf[256];
        snprintf(buf, sizeof(buf), "%s: expects a native handle, got a Dirac value", what);
        cdi_error(buf);
    }
    return v->ptr;
}
