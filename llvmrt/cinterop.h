/* cinterop.h -- boxed Value <-> native conversions for generated shims.
 *
 * The C++ shim (compiled with clang++/g++) includes this header to
 * unbox Dirac arguments, call real C/C++ code with static types, and
 * re-box results. All helpers abort via cdi_error on type mismatch,
 * mirroring the reference error model (`<fn>: cannot pass ...`).
 *
 * Link: compiled as C (or C++ with the guard below); no C++ types here.
 */
#ifndef CDIRAC_CINTEROP_H
#define CDIRAC_CINTEROP_H

#include <stdint.h>

#include "value.h"

#ifdef __cplusplus
extern "C" {
#endif

/* Unbox scalars (tag-checked). */
int32_t cdi_to_int(Value *v);
double cdi_to_double(Value *v);
int cdi_to_bool(Value *v);

/* String payload access (borrowed; valid while the Value is alive). */
const char *cdi_str_bytes(Value *v, int32_t *out_nbytes);

/* NUL-terminated copy of a Dirac string (malloc'd; caller frees).
 * Used for `const char*` C parameters. */
char *cdi_to_cstring(Value *v);

/* Box scalars. */
Value *cdi_from_int(int32_t x);
Value *cdi_from_long(long x); /* range-checked to i32 */
Value *cdi_from_double(double x);
Value *cdi_from_bool(int b);
Value *cdi_from_cstring(const char *s); /* NUL; NULL -> nil */
Value *cdi_from_ptr(void *p);           /* NULL -> nil */

/* Expect a class-handle argument (CDI_PTR). Returns the raw pointer. */
void *cdi_expect_handle(Value *v, const char *what);

#ifdef __cplusplus
}
#endif

#endif
