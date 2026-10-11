/* arith.h -- n-ary arithmetic, comparisons, `not` (N0 subset). */
#ifndef CDIRAC_ARITH_H
#define CDIRAC_ARITH_H

#include <stdint.h>

#include "value.h"

#ifdef __cplusplus
extern "C" {
#endif

/* N-ary ops over explicit arg arrays (the emitter builds one alloca per
 * call site). Heads are hardcoded per function, matching reference
 * messages (`+: expects numbers`, `/: division by zero`, ...). */
Value *cdi_add_n(Value **args, int32_t n);
Value *cdi_sub_n(Value **args, int32_t n);
Value *cdi_mul_n(Value **args, int32_t n);
Value *cdi_div_n(Value **args, int32_t n);
Value *cdi_eq_n(Value **args, int32_t n);
Value *cdi_lt_n(Value **args, int32_t n);
Value *cdi_gt_n(Value **args, int32_t n);
Value *cdi_le_n(Value **args, int32_t n);
Value *cdi_ge_n(Value **args, int32_t n);
Value *cdi_not(Value *v);

/* N1: integer-division trio (fixed 2, int-only, own div-zero head). */
Value *cdi_quotient(Value *a, Value *b);
Value *cdi_remainder(Value *a, Value *b);
Value *cdi_modulo(Value *a, Value *b);

/* N1: unary/small arithmetic (fixed 1; max/min n-ary keeping the winner). */
Value *cdi_add1(Value *v);
Value *cdi_sub1(Value *v);
Value *cdi_abs(Value *v);
Value *cdi_max_n(Value **args, int32_t n);
Value *cdi_min_n(Value **args, int32_t n);
Value *cdi_expt(Value *b, Value *e);
Value *cdi_sqrt(Value *v);

#ifdef __cplusplus
}
#endif

#endif
