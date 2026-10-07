#define _GNU_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int denied(const char *path) {
    const char *pid = getenv("SPRITE_TEST_DENIED_PROC_PID");
    char target[64];
    if (!pid) return 0;
    int length = snprintf(target, sizeof(target), "/proc/%s/stat", pid);
    return length > 0 && (size_t)length < sizeof(target) && strcmp(path, target) == 0;
}

#define INTERCEPT_OPEN(name) \
    int name(const char *path, int flags, ...) { \
        if (denied(path)) { errno = EACCES; return -1; } \
        mode_t mode = 0; \
        if ((flags & O_CREAT) || (flags & O_TMPFILE) == O_TMPFILE) { \
            va_list args; va_start(args, flags); \
            mode = va_arg(args, int); va_end(args); \
        } \
        int (*real_open)(const char *, int, ...) = dlsym(RTLD_NEXT, #name); \
        return real_open(path, flags, mode); \
    }

INTERCEPT_OPEN(open)
INTERCEPT_OPEN(open64)
