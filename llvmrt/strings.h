/* strings.h -- UTF-8 strings with UTF-16-unit API + float printing. */
#ifndef CDIRAC_STRINGS_H
#define CDIRAC_STRINGS_H

#include <stdint.h>

#include "value.h"

#ifdef __cplusplus
extern "C" {
#endif

/* Shortest-round-trip `Double.toString`-shape rendering (malloc'd).
 * `NaN`, `Infinity`/`-Infinity`, `-0.0`, decimal for [1e-3,1e7),
 * else `d.dEexp` with uppercase E and mandatory `.0`. */
char *cdi_float_str(double x);

/* Count UTF-16 code units in valid UTF-8 (astral = 2; CESU-8 lone
 * surrogate triplets count 1). */
int32_t cdi_str_units(const char *bytes, int32_t nbytes);

/* Slice units [i, j) end-exclusive (negative/j<i/past-end is a caller
 * error); splitting an astral char yields CESU-8 lone halves, which
 * print as `?`. Returns malloc'd bytes + length. */
char *cdi_str_slice(const char *bytes, int32_t nbytes, int32_t i, int32_t j,
                    int32_t *out_nbytes);

/* Case mapping (ASCII + Latin-1; upcase maps U+00DF to "SS", like
 * Locale.ROOT). Returns malloc'd bytes + length. */
char *cdi_str_upcase(const char *bytes, int32_t nbytes, int32_t *out_nbytes);
char *cdi_str_downcase(const char *bytes, int32_t nbytes, int32_t *out_nbytes);

/* N1 builtins over boxed values (message parity with the reference). */
Value *cdi_string_append_n(Value **args, int32_t n);
Value *cdi_string_length(Value *v);
Value *cdi_substring(Value *s, Value *i, Value *j, int has_j);
Value *cdi_string_upcase(Value *v);
Value *cdi_string_downcase(Value *v);
Value *cdi_number_to_string(Value *v);
Value *cdi_string_to_number(Value *v);

#ifdef __cplusplus
}
#endif

#endif
