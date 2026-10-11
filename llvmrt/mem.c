/* mem.c -- explicit native memory: machine.alloc/dealloc. */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "mem.h"
#include "value.h"

/* Live-handle registry: exactly the pointers alloc handed out and
 * dealloc has not yet reclaimed. Linear scan is fine (explicit
 * handles are rare by design). */
static void **live = NULL;
static size_t live_len = 0;
static size_t live_cap = 0;

static void registry_add(void *p) {
    if (live_len >= live_cap) {
        live_cap = live_cap ? live_cap * 2 : 16;
        live = (void **)realloc(live, live_cap * sizeof(void *));
        if (!live) {
            cdi_error("machine.alloc: out of memory");
        }
    }
    live[live_len++] = p;
}

static int registry_take(void *p) {
    for (size_t i = 0; i < live_len; i++) {
        if (live[i] == p) {
            live[i] = live[--live_len];
            return 1;
        }
    }
    return 0;
}

Value *cdi_machine_alloc(Value *n) {
    if (n->tag != CDI_INT || n->ival < 0) {
        char *got = cdi_write_cstr(n);
        char msg[512];
        snprintf(msg, sizeof(msg),
                 "machine.alloc: expects a non-negative integer, got: %s", got);
        cdi_error(msg);
    }
    void *p = calloc((size_t)n->ival > 0 ? (size_t)n->ival : 1, 1);
    if (!p) {
        cdi_error("machine.alloc: out of memory");
    }
    registry_add(p);
    return cdi_mk_ptr(p);
}

Value *cdi_machine_dealloc(Value *p) {
    if (p->tag != CDI_PTR) {
        char *got = cdi_write_cstr(p);
        char msg[512];
        snprintf(msg, sizeof(msg),
                 "machine.dealloc: expects a pointer, got: %s", got);
        cdi_error(msg);
    }
    if (!registry_take(p->ptr)) {
        cdi_error("machine.dealloc: unknown pointer (double free?)");
    }
    free(p->ptr);
    return cdi_void();
}
