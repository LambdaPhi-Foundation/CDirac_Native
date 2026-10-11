/* io.c -- effects: print/newline/read-line/error (N1). */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "io.h"
#include "value.h"

Value *cdi_print(Value *v) {
    char *s = cdi_write_cstr(v);
    puts(s);
    fflush(stdout);
    return cdi_void();
}

Value *cdi_newline(void) {
    puts("");
    fflush(stdout);
    return cdi_void();
}

/* Strict UTF-8 check over one line (NUL-terminated, newline included).
 * Mirrors the reference gate: invalid bytes are `read-line: invalid
 * utf-8`, transport failures `read-line: ...`. */
static int utf8_ok(const char *s) {
    const unsigned char *p = (const unsigned char *)s;
    while (*p) {
        if (*p < 0x80) {
            p++;
        } else if ((*p & 0xE0) == 0xC0) {
            if ((p[1] & 0xC0) != 0x80) {
                return 0;
            }
            p += 2;
        } else if ((*p & 0xF0) == 0xE0) {
            if ((p[1] & 0xC0) != 0x80 || (p[2] & 0xC0) != 0x80) {
                return 0;
            }
            p += 3;
        } else if ((*p & 0xF8) == 0xF0) {
            if ((p[1] & 0xC0) != 0x80 || (p[2] & 0xC0) != 0x80 ||
                (p[3] & 0xC0) != 0x80) {
                return 0;
            }
            p += 4;
        } else {
            return 0;
        }
    }
    return 1;
}

Value *cdi_read_line(void) {
    /* Growable line buffer (arbitrary line lengths, like BufRead). */
    size_t cap = 128, len = 0;
    char *buf = (char *)CDI_MALLOC(cap);
    int c = getchar();
    if (c == EOF) {
        if (ferror(stdin)) {
            cdi_error("read-line: input error");
        }
        return cdi_mk_bool(0);
    }
    for (;;) {
        if (len + 1 >= cap) {
            cap *= 2;
            buf = (char *)CDI_REALLOC(buf, cap);
        }
        buf[len++] = (char)c;
        c = getchar();
        if (c == EOF || c == '\n') {
            break;
        }
    }
    buf[len] = '\0';
    if (!utf8_ok(buf)) {
        cdi_error("read-line: invalid utf-8");
    }
    /* Strip trailing \r/\n (mirrors the reference's pop loop). */
    while (len > 0 && (buf[len - 1] == '\n' || buf[len - 1] == '\r')) {
        buf[--len] = '\0';
    }
    return cdi_mk_str(buf, (int32_t)len);
}

Value *cdi_error_n(Value **args, int32_t n) {
    if (n == 0) {
        cdi_error("error");
    }
    char *s = cdi_display_cstr(args[0]);
    cdi_error(s);
    return cdi_nil(); /* unreachable */
}
