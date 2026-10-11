/* containers.h -- reference-semantics arrays/queues (N0: arrays fully,
 * queues construct/read/length; mutating queue ends arrive in N1). */
#ifndef CDIRAC_CONTAINERS_H
#define CDIRAC_CONTAINERS_H

#include <stdint.h>

#include "value.h"

#ifdef __cplusplus
extern "C" {
#endif

Value *cdi_array0(void);
/* Push helpers return the void singleton so builtin wrappers share one shape. */
Value *cdi_arr_push(Value *arr, Value *v);
/* N-ary constructor over an explicit arg array (emitter ABI). */
Value *cdi_array_n(Value **args, int32_t n);
Value *cdi_len(Value *v);
Value *cdi_get(Value *a, Value *i);

/* N1: in-place array writes (arrays only, yield void). */
Value *cdi_put(Value *a, Value *i, Value *v);
Value *cdi_push(Value *a, Value *v);
Value *cdi_pop(Value *a);

Value *cdi_queue0(void);
Value *cdi_qpush(Value *q, Value *v);
/* N-ary queue constructor (emitter ABI). */
Value *cdi_queue_n(Value **args, int32_t n);
/* N1: deque ends (yield void on writes, elements on reads). */
Value *cdi_qpush_front(Value *q, Value *v);
Value *cdi_qpop(Value *q);
Value *cdi_qpop_back(Value *q);
Value *cdi_qpeek(Value *q);
Value *cdi_qpeek_back(Value *q);

#ifdef __cplusplus
}
#endif

#endif
