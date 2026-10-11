/* value.c -- boxed values: ctors, truthiness, rendering, abort path. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>

#include "value.h"
#include "strings.h"

static Value BOOL_TRUE = {CDI_BOOL, 0, 0.0, 1, NULL};
static Value BOOL_FALSE = {CDI_BOOL, 0, 0.0, 0, NULL};
static Value NIL_VAL = {CDI_NIL, 0, 0.0, 0, NULL};
static Value VOID_VAL = {CDI_VOID, 0, 0.0, 0, NULL};

Value *cdi_mk_int(int32_t x) {
    Value *v = (Value *)CDI_MALLOC(sizeof(Value));
    v->tag = CDI_INT;
    v->ival = x;
    v->fval = 0.0;
    v->bval = 0;
    v->ptr = NULL;
    return v;
}

Value *cdi_mk_float(double x) {
    Value *v = (Value *)CDI_MALLOC(sizeof(Value));
    v->tag = CDI_FLOAT;
    v->ival = 0;
    v->fval = x;
    v->bval = 0;
    v->ptr = NULL;
    return v;
}

Value *cdi_mk_bool(int b) {
    return b ? &BOOL_TRUE : &BOOL_FALSE;
}

Value *cdi_mk_str(const char *bytes, int32_t nbytes) {
    StrData *s = (StrData *)CDI_MALLOC(sizeof(StrData) + (size_t)nbytes);
    s->nbytes = nbytes;
    if (nbytes > 0) {
        memcpy(s->bytes, bytes, (size_t)nbytes);
    }
    Value *v = (Value *)CDI_MALLOC(sizeof(Value));
    v->tag = CDI_STR;
    v->ival = 0;
    v->fval = 0.0;
    v->bval = 0;
    v->ptr = s;
    return v;
}

Value *cdi_nil(void) {
    return &NIL_VAL;
}

Value *cdi_void(void) {
    return &VOID_VAL;
}

Value *cdi_mk_ptr(void *p) {
    Value *v = (Value *)CDI_MALLOC(sizeof(Value));
    v->tag = CDI_PTR;
    v->ival = 0;
    v->fval = 0.0;
    v->bval = 0;
    v->ptr = p;
    return v;
}

void *cdi_to_ptr(Value *v) {
    if (v->tag != CDI_PTR) {
        cdi_error("cannot pass non-pointer value as ptr");
    }
    return v->ptr;
}

int cdi_truthy(Value *v) {
    if (v->tag == CDI_BOOL) {
        return v->bval ? 1 : 0;
    }
    return 1; /* only false is falsy */
}

void cdi_error(const char *msg) {
    fprintf(stderr, "Error: %s\n", msg);
    exit(1);
}

void cdi_errorf(const char *who, const char *detail) {
    char buf[1024];
    snprintf(buf, sizeof(buf), "%s: %s", who, detail);
    cdi_error(buf);
}

/* ---- rendering (dynamic buffer) ---- */

typedef struct {
    char *data;
    size_t len;
    size_t cap;
} Buf;

static void buf_putc(Buf *b, char c) {
    if (b->len + 1 >= b->cap) {
        b->cap = b->cap ? b->cap * 2 : 64;
        b->data = (char *)CDI_REALLOC(b->data, b->cap);
    }
    b->data[b->len++] = c;
}

static void buf_puts(Buf *b, const char *s, size_t n) {
    for (size_t i = 0; i < n; i++) {
        buf_putc(b, s[i]);
    }
}

static void buf_cstr(Buf *b, const char *s) {
    buf_puts(b, s, strlen(s));
}

/* Write one string payload: quoted+escaped (write=true) or raw. Lone
 * CESU-8 surrogate halves print as `?`, like the reference. */
static void render_str(Buf *b, StrData *s, int quoted) {
    if (quoted) {
        buf_putc(b, '"');
    }
    int32_t i = 0;
    while (i < s->nbytes) {
        unsigned char c = (unsigned char)s->bytes[i];
        if (c < 0x80) {
            if (quoted) {
                switch (c) {
                case '"': buf_puts(b, "\\\"", 2); i++; continue;
                case '\\': buf_puts(b, "\\\\", 2); i++; continue;
                case '\n': buf_puts(b, "\\n", 2); i++; continue;
                case '\t': buf_puts(b, "\\t", 2); i++; continue;
                case '\r': buf_puts(b, "\\r", 2); i++; continue;
                default: break;
                }
            }
            buf_putc(b, (char)c);
            i++;
        } else if (c >= 0xED && i + 2 < s->nbytes && (unsigned char)s->bytes[i + 1] >= 0xA0 &&
                   (unsigned char)s->bytes[i + 1] <= 0xBF &&
                   ((unsigned char)s->bytes[i + 2] & 0xC0) == 0x80) {
            /* CESU-8 lone surrogate half -> `?`. */
            buf_putc(b, '?');
            i += 3;
        } else {
            int n = 1;
            if ((c & 0xE0) == 0xC0) {
                n = 2;
            } else if ((c & 0xF0) == 0xE0) {
                n = 3;
            } else if ((c & 0xF8) == 0xF0) {
                n = 4;
            }
            buf_puts(b, s->bytes + i, (size_t)n);
            i += n;
        }
    }
    if (quoted) {
        buf_putc(b, '"');
    }
}

static void render_value(Buf *b, Value *v, int quoted_strings);

static void render_container(Buf *b, Value **items, int32_t n, int is_queue) {
    if (is_queue) {
        buf_puts(b, "[queue", 6);
    } else {
        buf_putc(b, '[');
    }
    for (int32_t i = 0; i < n; i++) {
        if (i > 0 || is_queue) {
            buf_putc(b, ' ');
        }
        render_value(b, items[i], 1);
    }
    buf_putc(b, ']');
}

static void render_value(Buf *b, Value *v, int quoted_strings) {
    char tmp[64];
    switch (v->tag) {
    case CDI_INT:
        snprintf(tmp, sizeof(tmp), "%d", (int)v->ival);
        buf_cstr(b, tmp);
        break;
    case CDI_FLOAT:
        buf_cstr(b, cdi_float_str(v->fval));
        break;
    case CDI_BOOL:
        buf_cstr(b, v->bval ? "true" : "false");
        break;
    case CDI_STR:
        render_str(b, (StrData *)v->ptr, quoted_strings);
        break;
    case CDI_NIL:
        buf_cstr(b, "null");
        break;
    case CDI_VOID:
        buf_cstr(b, "#<void>");
        break;
    case CDI_ARRAY: {
        ArrData *a = (ArrData *)v->ptr;
        render_container(b, a->items, a->len, 0);
        break;
    }
    case CDI_QUEUE: {
        QueData *q = (QueData *)v->ptr;
        render_container(b, q->items, q->len, 1);
        break;
    }
    case CDI_PTR:
        buf_cstr(b, "#<ptr>");
        break;
    default:
        buf_cstr(b, "#<unknown>");
        break;
    }
}

static char *buf_finish(Buf *b) {
    buf_putc(b, '\0');
    return b->data;
}

char *cdi_write_cstr(Value *v) {
    Buf b = {NULL, 0, 0};
    render_value(&b, v, 1);
    return buf_finish(&b);
}

char *cdi_display_cstr(Value *v) {
    Buf b = {NULL, 0, 0};
    render_value(&b, v, 0);
    return buf_finish(&b);
}

Value *cdi_display(Value *v) {
    char *s = cdi_display_cstr(v);
    fputs(s, stdout);
    fflush(stdout);
    return cdi_void();
}

Value *cdi_displayln(Value *v) {
    char *s = cdi_display_cstr(v);
    puts(s);
    fflush(stdout);
    return cdi_void();
}

Value *cdi_echo(Value *v) {
    if (v->tag == CDI_VOID) {
        return v;
    }
    char *s = cdi_write_cstr(v);
    puts(s);
    fflush(stdout);
    return cdi_void();
}
