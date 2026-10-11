/* pred.h -- numeric/type predicates + deep equality (N1). */
#ifndef CDIRAC_PRED_H
#define CDIRAC_PRED_H

#include <stdint.h>

#include "value.h"

#ifdef __cplusplus
extern "C" {
#endif

/* Numeric predicates (fixed 1; zero?/positive?/negative? take ints and
 * floats, even?/odd? are int-only). */
Value *cdi_zero_p(Value *v);
Value *cdi_positive_p(Value *v);
Value *cdi_negative_p(Value *v);
Value *cdi_even_p(Value *v);
Value *cdi_odd_p(Value *v);

/* Deep structural equality with int/float coercion (fixed 2). */
Value *cdi_equal_p(Value *a, Value *b);

/* Type predicates (fixed 1). */
Value *cdi_number_p(Value *v);
Value *cdi_real_p(Value *v);
Value *cdi_integer_p(Value *v);
Value *cdi_boolean_p(Value *v);
Value *cdi_string_p(Value *v);
Value *cdi_array_p(Value *v);
Value *cdi_queue_p(Value *v);
Value *cdi_empty_p(Value *v);
Value *cdi_void_p(Value *v);

#ifdef __cplusplus
}
#endif

#endif
