/* mem.h -- explicit native memory: machine.alloc/dealloc.
 *
 * Dirac values are GC/leak-managed, but C++ interop and long-running
 * programs need an explicit escape hatch. These ops are `machine`-only
 * (bare `alloc` stays unbound): `[machine alloc n]` reserves zeroed
 * bytes and yields an opaque handle; `[machine dealloc h]` frees it.
 * Double frees and foreign pointers abort (the registry owns exactly
 * what alloc handed out).
 */
#ifndef CDIRAC_MEM_H
#define CDIRAC_MEM_H

#include <stdint.h>

#include "value.h"

#ifdef __cplusplus
extern "C" {
#endif

/* Fixed 1: non-negative int byte count -> zeroed handle. */
Value *cdi_machine_alloc(Value *n);
/* Fixed 1: live handle -> void (unknown/double free aborts). */
Value *cdi_machine_dealloc(Value *p);

#ifdef __cplusplus
}
#endif

#endif
