/* strings.c -- UTF-8 storage, UTF-16-unit semantics, float printing. */
#include <ctype.h>
#include <limits.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "strings.h"
#include "value.h"

/* ---- UTF-8/CESU-8 unit walk ----
 * Units are Unicode scalar values, except astral chars count 2
 * (high+low surrogate entries) and CESU-8 surrogate triplets count 1
 * (lone halves from allowed astral splits). Returns 0xFFFD marker via
 * *valid=0 on invalid bytes (caller raises invalid-UTF-8 errors). */

static uint32_t decode_unit(const unsigned char *s, int32_t left, int *consumed) {
    unsigned char c = s[0];
    if (c < 0x80) {
        *consumed = 1;
        return c;
    }
    if ((c & 0xE0) == 0xC0 && left >= 2 && (s[1] & 0xC0) == 0x80) {
        *consumed = 2;
        return ((uint32_t)(c & 0x1F) << 6) | (s[1] & 0x3F);
    }
    if ((c & 0xF0) == 0xE0 && left >= 3 && (s[1] & 0xC0) == 0x80 &&
        (s[2] & 0xC0) == 0x80) {
        *consumed = 3;
        uint32_t cp = ((uint32_t)(c & 0x0F) << 12) | ((uint32_t)(s[1] & 0x3F) << 6) |
                      (s[2] & 0x3F);
        return cp; /* CESU-8 triplets land in D800-DFFF: lone halves */
    }
    if ((c & 0xF8) == 0xF0 && left >= 4 && (s[1] & 0xC0) == 0x80 &&
        (s[2] & 0xC0) == 0x80 && (s[3] & 0xC0) == 0x80) {
        *consumed = 4;
        return ((uint32_t)(c & 0x07) << 18) | ((uint32_t)(s[1] & 0x3F) << 12) |
               ((uint32_t)(s[2] & 0x3F) << 6) | (s[3] & 0x3F);
    }
    *consumed = 1;
    return 0xFFFFFFFFu; /* invalid */
}

/* Decode whole string to unit array (astral -> high,low entries).
 * Returns NULL with *ok=0 on invalid bytes. */
static uint32_t *decode_all(const char *bytes, int32_t nbytes, int32_t *nunits,
                            int *ok) {
    uint32_t *units = (uint32_t *)CDI_MALLOC((size_t)(nbytes + 1) * 2 * sizeof(uint32_t));
    int32_t n = 0;
    int32_t i = 0;
    *ok = 1;
    while (i < nbytes) {
        int consumed = 0;
        uint32_t cp =
            decode_unit((const unsigned char *)bytes + i, nbytes - i, &consumed);
        if (cp == 0xFFFFFFFFu) {
            *ok = 0;
            return units;
        }
        if (cp > 0xFFFFu) {
            uint32_t v = cp - 0x10000u;
            units[n++] = 0xD800u + (v >> 10);
            units[n++] = 0xDC00u + (v & 0x3FFu);
        } else {
            units[n++] = cp;
        }
        i += consumed;
    }
    *nunits = n;
    return units;
}

static int encode_unit(uint32_t u, char out[4]) {
    if (u >= 0xD800u && u <= 0xDFFFu) {
        /* Lone half -> CESU-8 triplet (prints as `?`). */
        out[0] = (char)(0xE0 | (u >> 12));
        out[1] = (char)(0x80 | ((u >> 6) & 0x3F));
        out[2] = (char)(0x80 | (u & 0x3F));
        return 3;
    }
    if (u < 0x80u) {
        out[0] = (char)u;
        return 1;
    }
    if (u < 0x800u) {
        out[0] = (char)(0xC0 | (u >> 6));
        out[1] = (char)(0x80 | (u & 0x3F));
        return 2;
    }
    out[0] = (char)(0xE0 | (u >> 12));
    out[1] = (char)(0x80 | ((u >> 6) & 0x3F));
    out[2] = (char)(0x80 | (u & 0x3F));
    return 3;
}

int32_t cdi_str_units(const char *bytes, int32_t nbytes) {
    int ok = 0;
    int32_t n = 0;
    uint32_t *units = decode_all(bytes, nbytes, &n, &ok);
    (void)units; /* count only */
    return n;
}

char *cdi_str_slice(const char *bytes, int32_t nbytes, int32_t i, int32_t j,
                    int32_t *out_nbytes) {
    int ok = 0;
    int32_t n = 0;
    uint32_t *units = decode_all(bytes, nbytes, &n, &ok);
    char *out = (char *)CDI_MALLOC((size_t)(j - i) * 4 + 1);
    int32_t k = 0;
    for (int32_t u = i; u < j; u++) {
        /* Re-pair astral halves that survived the cut together. */
        uint32_t c = units[u];
        if (c >= 0xD800u && c <= 0xDBFFu && u + 1 < j && units[u + 1] >= 0xDC00u &&
            units[u + 1] <= 0xDFFFu) {
            uint32_t hi = c - 0xD800u, lo = units[u + 1] - 0xDC00u;
            uint32_t cp = 0x10000u + (hi << 10) + lo;
            out[k++] = (char)(0xF0 | (cp >> 18));
            out[k++] = (char)(0x80 | ((cp >> 12) & 0x3F));
            out[k++] = (char)(0x80 | ((cp >> 6) & 0x3F));
            out[k++] = (char)(0x80 | (cp & 0x3F));
            u++;
        } else {
            char tmp[4];
            int m = encode_unit(c, tmp);
            memcpy(out + k, tmp, (size_t)m);
            k += m;
        }
    }
    *out_nbytes = k;
    return out;
}

/* ---- case mapping (ASCII + Latin-1; ß special) ---- */

static uint32_t up_one(uint32_t c, uint32_t *extra, int *has_extra) {
    *has_extra = 0;
    if (c >= 'a' && c <= 'z') {
        return c - 32;
    }
    if (c >= 0xE0u && c <= 0xFEu && c != 0xF7u) {
        return c - 32;
    }
    if (c == 0xDFu) { /* ß -> SS */
        *extra = 'S';
        *has_extra = 1;
        return 'S';
    }
    return c;
}

static uint32_t down_one(uint32_t c) {
    if (c >= 'A' && c <= 'Z') {
        return c + 32;
    }
    if (c >= 0xC0u && c <= 0xDEu && c != 0xD7u) {
        return c + 32;
    }
    return c;
}

char *cdi_str_upcase(const char *bytes, int32_t nbytes, int32_t *out_nbytes) {
    int ok = 0;
    int32_t n = 0;
    uint32_t *units = decode_all(bytes, nbytes, &n, &ok);
    char *out = (char *)CDI_MALLOC((size_t)n * 4 + 8);
    int32_t k = 0;
    for (int32_t u = 0; u < n; u++) {
        /* Keep astral pairs together through casing (halves map plain). */
        uint32_t c = units[u];
        if (c >= 0xD800u && c <= 0xDBFFu && u + 1 < n && units[u + 1] >= 0xDC00u &&
            units[u + 1] <= 0xDFFFu) {
            char tmp[4];
            int m = encode_unit(c, tmp);
            memcpy(out + k, tmp, (size_t)m);
            k += m;
            m = encode_unit(units[u + 1], tmp);
            memcpy(out + k, tmp, (size_t)m);
            k += m;
            u++;
            continue;
        }
        uint32_t extra = 0;
        int has_extra = 0;
        uint32_t m = up_one(c, &extra, &has_extra);
        char tmp[4];
        int w = encode_unit(m, tmp);
        memcpy(out + k, tmp, (size_t)w);
        k += w;
        if (has_extra) {
            w = encode_unit(extra, tmp);
            memcpy(out + k, tmp, (size_t)w);
            k += w;
        }
    }
    *out_nbytes = k;
    return out;
}

char *cdi_str_downcase(const char *bytes, int32_t nbytes, int32_t *out_nbytes) {
    int ok = 0;
    int32_t n = 0;
    uint32_t *units = decode_all(bytes, nbytes, &n, &ok);
    char *out = (char *)CDI_MALLOC((size_t)n * 4 + 1);
    int32_t k = 0;
    for (int32_t u = 0; u < n; u++) {
        char tmp[4];
        int w = encode_unit(down_one(units[u]), tmp);
        memcpy(out + k, tmp, (size_t)w);
        k += w;
    }
    *out_nbytes = k;
    return out;
}

/* ---- float printing: shortest round-trip, Java shapes ----
 * Try precisions 1..17 (`%.*e`), keep the SHORTEST whose strtod equals
 * x, strip its zeros, then reframe exactly like the reference
 * (decimal for exp in [-3,7), else d.dEexp, mandatory `.0`). */

char *cdi_float_str(double x) {
    char *out = (char *)CDI_MALLOC(64);
    if (isnan(x)) {
        strcpy(out, "NaN");
        return out;
    }
    if (isinf(x)) {
        strcpy(out, x > 0 ? "Infinity" : "-Infinity");
        return out;
    }
    if (x == 0.0) {
        strcpy(out, signbit(x) ? "-0.0" : "0.0");
        return out;
    }
    char cand[64];
    int best_p = 17;
    for (int p = 1; p <= 17; p++) {
        snprintf(cand, sizeof(cand), "%.*e", p, x);
        if (strtod(cand, NULL) == x) {
            best_p = p;
            break;
        }
    }
    snprintf(cand, sizeof(cand), "%.*e", best_p, x);
    /* Split mantissa/exponent; strip trailing zeros from the fraction. */
    char *epos = strchr(cand, 'e');
    int exp = atoi(epos + 1);
    *epos = '\0';
    int neg = cand[0] == '-';
    char *mant = neg ? cand + 1 : cand;
    char *dot = strchr(mant, '.');
    /* digits = lead + frac without dot */
    char digits[64];
    {
        char lead[64], frac[64];
        if (dot) {
            size_t L = (size_t)(dot - mant);
            memcpy(lead, mant, L);
            lead[L] = '\0';
            strcpy(frac, dot + 1);
        } else {
            strcpy(lead, mant);
            frac[0] = '\0';
        }
        snprintf(digits, sizeof(digits), "%s%s", lead, frac);
    }
    /* Strip trailing zeros (keep at least one digit). */
    {
        size_t L = strlen(digits);
        while (L > 1 && digits[L - 1] == '0') {
            digits[--L] = '\0';
        }
    }
    if (exp >= -3 && exp < 7) {
        long pos = (long)exp + 1;
        long n = (long)strlen(digits);
        char body[64];
        if (pos <= 0) {
            snprintf(body, sizeof(body), "0.%.*s%s", (int)(-pos), "0000000000000000000", digits);
        } else if (pos >= n) {
            char zeros[64] = {0};
            memset(zeros, '0', (size_t)(pos - n));
            snprintf(body, sizeof(body), "%s%s.0", digits, zeros);
        } else {
            char head[64], tail[64];
            memcpy(head, digits, (size_t)pos);
            head[pos] = '\0';
            strcpy(tail, digits + pos);
            snprintf(body, sizeof(body), "%s.%s", head, tail);
        }
        snprintf(out, 64, "%s%s", neg ? "-" : "", body);
    } else {
        char mj[64];
        if (strlen(digits) == 1) {
            snprintf(mj, sizeof(mj), "%s.0", digits);
        } else {
            snprintf(mj, sizeof(mj), "%c.%s", digits[0], digits + 1);
        }
        snprintf(out, 64, "%s%sE%d", neg ? "-" : "", mj, exp);
    }
    return out;
}

/* ------------------------------------------------------------ N1 builtins */

static const StrData *expect_strdata(const char *who, Value *v) {
    if (v->tag != CDI_STR) {
        char *got = cdi_write_cstr(v);
        char msg[512];
        snprintf(msg, sizeof(msg), "%s: expects a string, got: %s", who, got);
        cdi_error(msg);
    }
    return (const StrData *)v->ptr;
}

static int32_t expect_idx(const char *who, Value *v) {
    if (v->tag != CDI_INT) {
        char *got = cdi_write_cstr(v);
        char msg[512];
        snprintf(msg, sizeof(msg), "%s: expects an integer, got: %s", who, got);
        cdi_error(msg);
    }
    return v->ival;
}

Value *cdi_string_append_n(Value **args, int32_t n) {
    /* Two passes: measure, then fill (no quadratic copies). */
    int32_t total = 0;
    for (int32_t i = 0; i < n; i++) {
        total += expect_strdata("string-append", args[i])->nbytes;
    }
    char *buf = (char *)CDI_MALLOC((size_t)total > 0 ? (size_t)total : 1);
    int32_t off = 0;
    for (int32_t i = 0; i < n; i++) {
        const StrData *s = expect_strdata("string-append", args[i]);
        memcpy(buf + off, s->bytes, (size_t)s->nbytes);
        off += s->nbytes;
    }
    Value *v = cdi_mk_str(buf, total);
    return v;
}

Value *cdi_string_length(Value *v) {
    const StrData *s = expect_strdata("string-length", v);
    return cdi_mk_int(cdi_str_units(s->bytes, s->nbytes));
}

Value *cdi_substring(Value *s, Value *i, Value *j, int has_j) {
    const StrData *d = expect_strdata("substring", s);
    int32_t start = expect_idx("substring", i);
    int32_t finish = cdi_str_units(d->bytes, d->nbytes);
    if (has_j) {
        finish = expect_idx("substring", j);
    }
    if (start < 0 || finish < start || finish > cdi_str_units(d->bytes, d->nbytes)) {
        cdi_error("substring: index out of range");
    }
    int32_t out_n = 0;
    char *out = cdi_str_slice(d->bytes, d->nbytes, start, finish, &out_n);
    return cdi_mk_str(out, out_n);
}

Value *cdi_string_upcase(Value *v) {
    const StrData *s = expect_strdata("string-upcase", v);
    int32_t out_n = 0;
    char *out = cdi_str_upcase(s->bytes, s->nbytes, &out_n);
    return cdi_mk_str(out, out_n);
}

Value *cdi_string_downcase(Value *v) {
    const StrData *s = expect_strdata("string-downcase", v);
    int32_t out_n = 0;
    char *out = cdi_str_downcase(s->bytes, s->nbytes, &out_n);
    return cdi_mk_str(out, out_n);
}

Value *cdi_number_to_string(Value *v) {
    if (v->tag != CDI_INT && v->tag != CDI_FLOAT) {
        char *got = cdi_write_cstr(v);
        char msg[512];
        snprintf(msg, sizeof(msg), "number->string: expects a number, got: %s", got);
        cdi_error(msg);
    }
    char *w = cdi_write_cstr(v);
    /* cdi_write_cstr renders write form already; re-box as a string. */
    int32_t n = (int32_t)strlen(w);
    return cdi_mk_str(w, n);
}

/* Digit-led gate: optional sign, then a digit or `.`+digit (so "inf",
 * "NaN", hex, and bare exponents stay `false`, like the reference). */
static int digit_led(const char *t) {
    size_t i = 0;
    if (t[0] == '+' || t[0] == '-') {
        i = 1;
    }
    if (t[i] >= '0' && t[i] <= '9') {
        /* Reject hex: Rust's parser never accepts `0x`. */
        if (t[i] == '0' && (t[i + 1] == 'x' || t[i + 1] == 'X')) {
            return 0;
        }
        return 1;
    }
    return t[i] == '.' && t[i + 1] >= '0' && t[i + 1] <= '9';
}

Value *cdi_string_to_number(Value *v) {
    const StrData *s = expect_strdata("string->number", v);
    /* Trim ASCII whitespace (mirrors str::trim for these inputs). */
    int32_t lo = 0, hi = s->nbytes;
    while (lo < hi && isspace((unsigned char)s->bytes[lo])) {
        lo++;
    }
    while (hi > lo && isspace((unsigned char)s->bytes[hi - 1])) {
        hi--;
    }
    if (lo >= hi) {
        return cdi_mk_bool(0);
    }
    /* Copy to a NUL-terminated scratch buffer. */
    char *t = (char *)CDI_MALLOC((size_t)(hi - lo) + 1);
    memcpy(t, s->bytes + lo, (size_t)(hi - lo));
    t[hi - lo] = '\0';
    char *end = NULL;
    /* long long is 64-bit everywhere (Windows long is 32-bit, which
     * would clamp instead of overflowing into the float path). */
    long long n = strtoll(t, &end, 10);
    if (end != t && *end == '\0' && n >= INT32_MIN && n <= INT32_MAX) {
        return cdi_mk_int((int32_t)n);
    }
    /* i32 overflow falls through to float (reference: "2147483648" ->
     * 2.147483648E9); anything not fully consumed is not a number. */
    double x = strtod(t, &end);
    if (end != t && *end == '\0' && digit_led(t)) {
        return cdi_mk_float(x);
    }
    return cdi_mk_bool(0);
}
