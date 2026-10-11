/* containers.c -- handles are heap structs: assignment copies the
 * pointer, so mutations show through aliases (like the reference). */
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#include "containers.h"
#include "strings.h"
#include "value.h"

static ArrData *new_arr(void) {
    ArrData *a = (ArrData *)CDI_MALLOC(sizeof(ArrData));
    a->len = 0;
    a->cap = 4;
    a->items = (Value **)CDI_MALLOC(sizeof(Value *) * 4);
    return a;
}

static void arr_push_raw(ArrData *a, Value *v) {
    if (a->len >= a->cap) {
        a->cap *= 2;
        a->items = (Value **)CDI_REALLOC(a->items, sizeof(Value *) * (size_t)a->cap);
    }
    a->items[a->len++] = v;
}

Value *cdi_array0(void) {
    Value *v = (Value *)CDI_MALLOC(sizeof(Value));
    v->tag = CDI_ARRAY;
    v->ival = 0;
    v->fval = 0.0;
    v->bval = 0;
    v->ptr = new_arr();
    return v;
}

Value *cdi_arr_push(Value *arr, Value *v) {
    arr_push_raw((ArrData *)arr->ptr, v);
    return cdi_void();
}

Value *cdi_array_n(Value **args, int32_t n) {
    Value *v = cdi_array0();
    for (int32_t i = 0; i < n; i++) {
        arr_push_raw((ArrData *)v->ptr, args[i]);
    }
    return v;
}

static int32_t to_index(const char *who, int32_t len, Value *idx) {
    int is_int = idx->tag == CDI_INT;
    int32_t i = is_int ? idx->ival : 0;
    if (!is_int) {
        char *got = cdi_write_cstr(idx);
        char msg[512];
        snprintf(msg, sizeof(msg), "%s: expects an integer index, got: %s", who, got);
        cdi_error(msg);
    }
    if (i < 0 || i >= len) {
        char msg[256];
        snprintf(msg, sizeof(msg), "%s: index %d out of range for length %d", who, (int)i,
                 (int)len);
        cdi_error(msg);
    }
    return i;
}

Value *cdi_len(Value *v) {
    if (v->tag == CDI_ARRAY) {
        return cdi_mk_int(((ArrData *)v->ptr)->len);
    }
    if (v->tag == CDI_QUEUE) {
        return cdi_mk_int(((QueData *)v->ptr)->len);
    }
    if (v->tag == CDI_STR) {
        StrData *s = (StrData *)v->ptr;
        return cdi_mk_int(cdi_str_units(s->bytes, s->nbytes));
    }
    char *got = cdi_write_cstr(v);
    char msg[512];
    snprintf(msg, sizeof(msg), "len: expects an array, a queue or a string, got: %s", got);
    cdi_error(msg);
    return cdi_nil(); /* unreachable */
}

Value *cdi_get(Value *a, Value *i) {
    if (a->tag != CDI_ARRAY && a->tag != CDI_QUEUE) {
        char *got = cdi_write_cstr(a);
        char msg[512];
        snprintf(msg, sizeof(msg), "get: expects an array or a queue, got: %s", got);
        cdi_error(msg);
    }
    ArrData *d = (ArrData *)a->ptr; /* same layout prefix as QueData */
    int32_t k = to_index("get", d->len, i);
    return d->items[k];
}

Value *cdi_queue0(void) {
    Value *v = (Value *)CDI_MALLOC(sizeof(Value));
    v->tag = CDI_QUEUE;
    v->ival = 0;
    v->fval = 0.0;
    v->bval = 0;
    QueData *q = (QueData *)CDI_MALLOC(sizeof(QueData));
    q->len = 0;
    q->cap = 4;
    q->items = (Value **)CDI_MALLOC(sizeof(Value *) * 4);
    v->ptr = q;
    return v;
}

void cdi_qpush_raw(QueData *d, Value *v) {
    if (d->len >= d->cap) {
        d->cap *= 2;
        d->items = (Value **)CDI_REALLOC(d->items, sizeof(Value *) * (size_t)d->cap);
    }
    d->items[d->len++] = v;
}

Value *cdi_qpush(Value *q, Value *v) {
    if (q->tag != CDI_QUEUE) {
        char *got = cdi_write_cstr(q);
        char msg[512];
        snprintf(msg, sizeof(msg), "qpush: expects a queue, got: %s", got);
        cdi_error(msg);
    }
    cdi_qpush_raw((QueData *)q->ptr, v);
    return cdi_void();
}

/* ------------------------------------------------------------ N1 containers */

static ArrData *expect_array(const char *who, Value *v) {
    if (v->tag != CDI_ARRAY) {
        char *got = cdi_write_cstr(v);
        char msg[512];
        snprintf(msg, sizeof(msg), "%s: expects an array, got: %s", who, got);
        cdi_error(msg);
    }
    return (ArrData *)v->ptr;
}

static QueData *expect_queue(const char *who, Value *v) {
    if (v->tag != CDI_QUEUE) {
        char *got = cdi_write_cstr(v);
        char msg[512];
        snprintf(msg, sizeof(msg), "%s: expects a queue, got: %s", who, got);
        cdi_error(msg);
    }
    return (QueData *)v->ptr;
}

Value *cdi_put(Value *a, Value *i, Value *v) {
    ArrData *d = expect_array("put", a);
    d->items[to_index("put", d->len, i)] = v;
    return cdi_void();
}

Value *cdi_push(Value *a, Value *v) {
    arr_push_raw(expect_array("push", a), v);
    return cdi_void();
}

Value *cdi_pop(Value *a) {
    ArrData *d = expect_array("pop", a);
    if (d->len == 0) {
        cdi_error("pop: empty array");
    }
    return d->items[--d->len];
}

Value *cdi_queue_n(Value **args, int32_t n) {
    Value *v = cdi_queue0();
    QueData *d = (QueData *)v->ptr;
    for (int32_t i = 0; i < n; i++) {
        cdi_qpush_raw(d, args[i]);
    }
    return v;
}

Value *cdi_qpush_front(Value *q, Value *v) {
    QueData *d = expect_queue("qpush-front", q);
    if (d->len >= d->cap) {
        d->cap *= 2;
        d->items = (Value **)CDI_REALLOC(d->items, sizeof(Value *) * (size_t)d->cap);
    }
    memmove(&d->items[1], &d->items[0], sizeof(Value *) * (size_t)d->len);
    d->items[0] = v;
    d->len++;
    return cdi_void();
}

Value *cdi_qpop(Value *q) {
    QueData *d = expect_queue("qpop", q);
    if (d->len == 0) {
        cdi_error("qpop: empty queue");
    }
    Value *v = d->items[0];
    memmove(&d->items[0], &d->items[1], sizeof(Value *) * (size_t)(d->len - 1));
    d->len--;
    return v;
}

Value *cdi_qpop_back(Value *q) {
    QueData *d = expect_queue("qpop-back", q);
    if (d->len == 0) {
        cdi_error("qpop-back: empty queue");
    }
    return d->items[--d->len];
}

Value *cdi_qpeek(Value *q) {
    QueData *d = expect_queue("qpeek", q);
    if (d->len == 0) {
        cdi_error("qpeek: empty queue");
    }
    return d->items[0];
}

Value *cdi_qpeek_back(Value *q) {
    QueData *d = expect_queue("qpeek-back", q);
    if (d->len == 0) {
        cdi_error("qpeek-back: empty queue");
    }
    return d->items[d->len - 1];
}
