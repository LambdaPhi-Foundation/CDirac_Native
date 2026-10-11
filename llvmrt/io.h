/* io.h -- effects: print/newline/read-line/error (N1). */
#ifndef CDIRAC_IO_H
#define CDIRAC_IO_H

#include <stdint.h>

#include "value.h"

#ifdef __cplusplus
extern "C" {
#endif

/* Write form + newline (fixed 1, yields void). */
Value *cdi_print(Value *v);
/* Blank line (fixed 0, yields void). */
Value *cdi_newline(void);
/* One stdin line, \r/\n stripped; EOF yields false (fixed 0). */
Value *cdi_read_line(void);
/* Abort with display of args[0], or "error" with no args (0..1). */
Value *cdi_error_n(Value **args, int32_t n);

#ifdef __cplusplus
}
#endif

#endif
