#define _GNU_SOURCE

#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

extern char **environ;

typedef int (*execve_fn_t)(const char *filename, char *const argv[], char *const envp[]);

static execve_fn_t real_execve_fn = NULL;
typedef struct {
    char interpreter[PATH_MAX];
    char arg[PATH_MAX];
    bool has_arg;
} shebang_info_t;

static void init_real_execve(void) {
    if (!real_execve_fn) {
        real_execve_fn = (execve_fn_t)dlsym(RTLD_NEXT, "execve");
    }
}

static bool starts_with(const char *value, const char *prefix) {
    if (!value || !prefix) {
        return false;
    }
    size_t prefix_len = strlen(prefix);
    return strncmp(value, prefix, prefix_len) == 0;
}

static bool is_linker_path(const char *path) {
    return path && (strcmp(path, "/system/bin/linker64") == 0 || strcmp(path, "/system/bin/linker") == 0);
}

static const char *select_system_linker(void) {
    if (access("/system/bin/linker64", X_OK) == 0) {
        return "/system/bin/linker64";
    }
    return "/system/bin/linker";
}

static bool map_legacy_termux_usr_path(const char *path, char *out, size_t out_size) {
    if (!path || !out || out_size == 0) {
        return false;
    }

    const char *prefix = getenv("PREFIX");
    if (!prefix || prefix[0] == '\0') {
        return false;
    }

    const char *suffix = NULL;
    const char *legacy_data = "/data/data/com.termux/files/usr";
    const char *legacy_user = "/data/user/0/com.termux/files/usr";
    if (starts_with(path, legacy_data)) {
        suffix = path + strlen(legacy_data);
    } else if (starts_with(path, legacy_user)) {
        suffix = path + strlen(legacy_user);
    }

    if (!suffix) {
        return false;
    }

    int n = snprintf(out, out_size, "%s%s", prefix, suffix);
    return n > 0 && (size_t)n < out_size;
}

static bool path_in_prefix(const char *path) {
    if (!path || path[0] != '/') {
        return false;
    }

    char remapped[PATH_MAX];
    const char *check_path = path;
    if (map_legacy_termux_usr_path(path, remapped, sizeof(remapped))) {
        check_path = remapped;
    }

    const char *rootfs = getenv("TERMUX__ROOTFS");
    if (rootfs && rootfs[0] != '\0' && starts_with(check_path, rootfs)) {
        return true;
    }

    const char *prefix = getenv("PREFIX");
    if (prefix && prefix[0] != '\0' && starts_with(check_path, prefix)) {
        return true;
    }

    return strstr(check_path, "/files/prefix/") != NULL;
}

static bool is_elf_binary(const char *path) {
    unsigned char magic[4] = {0};
    int fd = open(path, O_RDONLY | O_CLOEXEC);
    if (fd < 0) {
        return false;
    }

    ssize_t n = read(fd, magic, sizeof(magic));
    close(fd);
    return n == (ssize_t)sizeof(magic) && magic[0] == 0x7f && magic[1] == 'E' && magic[2] == 'L' && magic[3] == 'F';
}

static bool parse_shebang(const char *path, shebang_info_t *out) {
    if (!out) {
        return false;
    }
    memset(out, 0, sizeof(*out));

    int fd = open(path, O_RDONLY | O_CLOEXEC);
    if (fd < 0) {
        return false;
    }

    char buf[512];
    ssize_t n = read(fd, buf, sizeof(buf) - 1);
    close(fd);
    if (n <= 2) {
        return false;
    }

    buf[n] = '\0';
    if (!(buf[0] == '#' && buf[1] == '!')) {
        return false;
    }

    char *p = buf + 2;
    while (*p == ' ' || *p == '\t') {
        p++;
    }
    if (*p != '/') {
        return false;
    }

    char *end = p;
    while (*end && *end != ' ' && *end != '\t' && *end != '\n' && *end != '\r') {
        end++;
    }
    size_t len = (size_t)(end - p);
    if (len == 0 || len >= sizeof(out->interpreter)) {
        return false;
    }
    memcpy(out->interpreter, p, len);
    out->interpreter[len] = '\0';

    while (*end == ' ' || *end == '\t') {
        end++;
    }
    if (*end && *end != '\n' && *end != '\r') {
        char *arg_end = end;
        while (*arg_end && *arg_end != '\n' && *arg_end != '\r') {
            arg_end++;
        }
        size_t arg_len = (size_t)(arg_end - end);
        if (arg_len > 0 && arg_len < sizeof(out->arg)) {
            memcpy(out->arg, end, arg_len);
            out->arg[arg_len] = '\0';
            out->has_arg = true;
        }
    }

    return true;
}

static bool should_wrap_elf(const char *filename) {
    if (!filename || filename[0] != '/') {
        return false;
    }
    if (is_linker_path(filename)) {
        return false;
    }
    if (starts_with(filename, "/system/") || starts_with(filename, "/apex/")) {
        return false;
    }
    if (!path_in_prefix(filename)) {
        return false;
    }
    return is_elf_binary(filename);
}

static int execve_via_linker(const char *filename, char *const argv[], char *const envp[]) {
    const char *linker = select_system_linker();
    size_t argc = 0;
    if (argv) {
        while (argv[argc]) {
            argc++;
        }
    }

    /* linker, target, argv[1..], NULL */
    char **new_argv = (char **)calloc(argc + 2, sizeof(char *));
    if (!new_argv) {
        errno = ENOMEM;
        return -1;
    }

    new_argv[0] = (char *)linker;
    new_argv[1] = (char *)filename;
    for (size_t i = 1; i < argc; i++) {
        new_argv[i + 1] = argv[i];
    }

    int rc = real_execve_fn(linker, new_argv, envp ? envp : environ);
    int saved_errno = errno;
    free(new_argv);
    errno = saved_errno;
    return rc;
}

static int exec_script_via_linker(
    const char *filename,
    const shebang_info_t *sb,
    char *const argv[],
    char *const envp[]
) {
    if (!sb || sb->interpreter[0] == '\0') {
        errno = ENOEXEC;
        return -1;
    }

    char remapped_interpreter[PATH_MAX];
    const char *interpreter = sb->interpreter;
    if (map_legacy_termux_usr_path(sb->interpreter, remapped_interpreter, sizeof(remapped_interpreter))) {
        interpreter = remapped_interpreter;
    }

    if (!path_in_prefix(interpreter)) {
        /* non-prefix interpreters (e.g. /system/bin/sh) can run normally */
        return real_execve_fn(filename, argv, envp ? envp : environ);
    }

    const char *linker = select_system_linker();
    size_t argc = 0;
    if (argv) {
        while (argv[argc]) {
            argc++;
        }
    }

    size_t fixed = sb->has_arg ? 4 : 3; /* linker, interp, [arg], script */
    char **new_argv = (char **)calloc(argc + fixed, sizeof(char *));
    if (!new_argv) {
        errno = ENOMEM;
        return -1;
    }

    size_t i = 0;
    new_argv[i++] = (char *)linker;
    new_argv[i++] = (char *)interpreter;
    if (sb->has_arg) {
        new_argv[i++] = (char *)sb->arg;
    }
    new_argv[i++] = (char *)filename;
    for (size_t j = 1; j < argc; j++) {
        new_argv[i++] = argv[j];
    }

    int rc = real_execve_fn(linker, new_argv, envp ? envp : environ);
    int saved_errno = errno;
    free(new_argv);
    errno = saved_errno;
    return rc;
}

int execve(const char *filename, char *const argv[], char *const envp[]) {
    init_real_execve();
    if (!real_execve_fn) {
        errno = ENOSYS;
        return -1;
    }

    char remapped_filename[PATH_MAX];
    const char *effective_filename = filename;
    if (map_legacy_termux_usr_path(filename, remapped_filename, sizeof(remapped_filename))) {
        effective_filename = remapped_filename;
    }

    if (should_wrap_elf(effective_filename)) {
        return execve_via_linker(effective_filename, argv, envp);
    }

    shebang_info_t sb;
    if (path_in_prefix(effective_filename) && parse_shebang(effective_filename, &sb)) {
        return exec_script_via_linker(effective_filename, &sb, argv, envp);
    }

    return real_execve_fn(effective_filename, argv, envp ? envp : environ);
}

static int search_path_and_exec(const char *file, char *const argv[], char *const envp[]) {
    if (!file || file[0] == '\0') {
        errno = ENOENT;
        return -1;
    }
    if (strchr(file, '/')) {
        return execve(file, argv, envp);
    }

    const char *path_env = getenv("PATH");
    if (!path_env || path_env[0] == '\0') {
        path_env = "/system/bin";
    }

    char *path_copy = strdup(path_env);
    if (!path_copy) {
        errno = ENOMEM;
        return -1;
    }

    int saved_errno = ENOENT;
    char *saveptr = NULL;
    for (char *token = strtok_r(path_copy, ":", &saveptr); token; token = strtok_r(NULL, ":", &saveptr)) {
        const char *dir = token[0] ? token : ".";
        char candidate[PATH_MAX];
        int n = snprintf(candidate, sizeof(candidate), "%s/%s", dir, file);
        if (n <= 0 || (size_t)n >= sizeof(candidate)) {
            continue;
        }

        execve(candidate, argv, envp);
        if (errno != ENOENT && errno != ENOTDIR) {
            saved_errno = errno;
        }
    }

    free(path_copy);
    errno = saved_errno;
    return -1;
}

int execv(const char *path, char *const argv[]) {
    return execve(path, argv, environ);
}

int execvp(const char *file, char *const argv[]) {
    return search_path_and_exec(file, argv, environ);
}

int execvpe(const char *file, char *const argv[], char *const envp[]) {
    return search_path_and_exec(file, argv, envp ? envp : environ);
}
