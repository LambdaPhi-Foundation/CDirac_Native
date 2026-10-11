/* value.h -- boxed Dirac values for the LLVM backend (N0).
 *
 * Every Dirac value is a heap `Value*` (opaque `ptr` in IR). Tags mirror
 * the reference model: 32-bit wrapping ints, doubles, bools (only `false`
 * falsy), UTF-8 strings with UTF-16-unit API, nil, void, arrays, queues.
 *
 * Memory: CDI_MALLOC defaults to malloc (leak-by-design: generated code
 * never frees). Define CDI_GC_BOEHM and link -lgc to use Boehm instead;
 * Boehm scans the stack conservatively, so heap Values held in locals
 * stay alive either way. Explicit raw allocation is `machine.alloc` /
 * `machine.dealloc` (see mem.h, lowered by llvmgen); the box still
 * never frees on its own.
 */
#ifndef CDIRAC_VALUE_H
#define CDIRAC_VALUE_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#ifdef CDI_GC_BOEHM
#include <gc.h>
#define CDI_MALLOC(n) GC_malloc(n)
#define CDI_REALLOC(p, n) GC_realloc((p), (n))
#else
#include <stdlib.h>
#include <string.h>
#define CDI_MALLOC(n) malloc(n)
#define CDI_REALLOC(p, n) realloc((p), (n))
#endif

enum {
    CDI_INT = 0,
    CDI_FLOAT = 1,
    CDI_BOOL = 2,
    CDI_STR = 3,
    CDI_NIL = 4,
    CDI_VOID = 5,
    CDI_ARRAY = 6,
    CDI_QUEUE = 7,
    CDI_PTR = 8
};

typedef struct Value Value;
typedef struct StrData StrData;
typedef struct ArrData ArrData;
typedef struct QueData QueData;

/* Payload structs live behind `Value.ptr` (opaque in IR). */
struct StrData {
    int32_t nbytes; /* UTF-8 length */
    char bytes[];   /* not NUL-terminated */
};

struct ArrData {
    int32_t len;
    int32_t cap;
    Value **items;
};

struct QueData {
    int32_t len;
    int32_t cap;
    Value **items; /* dense, front at index 0 (memmove on pop-front) */
};

struct Value {
    int32_t tag;
    int32_t ival;
    double fval;
    int32_t bval;
    void *ptr;
};

/* Constructors (all heap boxes except the singletons). */
Value *cdi_mk_int(int32_t x);
Value *cdi_mk_float(double x);
Value *cdi_mk_bool(int b);
Value *cdi_mk_str(const char *bytes, int32_t nbytes); /* copies */
Value *cdi_nil(void);
Value *cdi_void(void);
/* Opaque native handle (C/C++ object pointers). The box owns nothing:
 * lifetime is managed by explicit delete wrappers / free. */
Value *cdi_mk_ptr(void *p);
void *cdi_to_ptr(Value *v); /* tag-checked; aborts with cdi_error on mismatch */

/* Truthiness: everything truthy except boolean false. */
int cdi_truthy(Value *v);

/* Rendering: write form (quoted strings, [..] containers) and display
 * form (raw strings). Both return malloc'd NUL-terminated C strings
 * (caller frees; tests use these for assertions). */
char *cdi_write_cstr(Value *v);
char *cdi_display_cstr(Value *v);

/* Effects: print to stdout (echo skips void), all return void value. */
Value *cdi_display(Value *v);
Value *cdi_displayln(Value *v);
Value *cdi_echo(Value *v);

/* Abort path: `Error: <msg>` on stderr, exit(1). No unwinding. */
void cdi_error(const char *msg);
void cdi_errorf(const char *who, const char *detail);

#ifdef __cplusplus
}
#endif

#endif
