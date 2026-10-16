/* Test-only native probes: no wrapper, preload, or producer stdout is trusted
 * by the supervisor. These deliberately handle failures and exit zero. */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <pthread.h>
#include <sched.h>
#include <setjmp.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <unistd.h>

static sigjmp_buf fault;
static void bus_handler(int signal) {
    (void)signal;
    siglongjmp(fault, 1);
}

static void *exec_thread(void *argument) {
    (void)argument;
    execl("/bin/sh", "/bin/sh", "-c", "printf thread-exec", (char *)NULL);
    _exit(92);
}

int main(int argc, char **argv) {
    if (argc != 2) return 90;
    if (!strcmp(argv[1], "clone-untraced")) {
        long child = syscall(SYS_clone, CLONE_UNTRACED | SIGCHLD, 0, 0, 0, 0);
        if (child == 0) _exit(91);
        if (child > 0) { waitpid((pid_t)child, NULL, 0); return 91; }
        if (errno != EPERM) return 92;
    } else if (!strcmp(argv[1], "clone3")) {
        unsigned long long arguments[11] = {0};
        arguments[4] = SIGCHLD;
        long child = syscall(SYS_clone3, arguments, sizeof(arguments));
        if (child == 0) _exit(91);
        if (child > 0) { waitpid((pid_t)child, NULL, 0); return 91; }
        if (errno != ENOSYS) return 92;
    } else if (!strcmp(argv[1], "async-io")) {
        unsigned long context = 0;
        if (syscall(SYS_io_setup, 1, &context) != -1 || errno != ENOSYS) return 91;
        unsigned char parameters[256] = {0};
        if (syscall(SYS_io_uring_setup, 1, parameters) != -1 || errno != ENOSYS) return 92;
    } else if (!strcmp(argv[1], "mmap-quota")) {
        if (chdir(getenv("QUALIFICATION_WORK_DIR"))) return 91;
        int file = open("mapping", O_CREAT | O_RDWR, 0600);
        const size_t size = 300 * 1024 * 1024;
        if (file < 0 || ftruncate(file, (off_t)size)) return 92;
        volatile char *mapping = mmap(NULL, size, PROT_WRITE, MAP_SHARED, file, 0);
        if (mapping == MAP_FAILED) return 93;
        struct sigaction action = {.sa_handler = bus_handler};
        sigemptyset(&action.sa_mask);
        if (sigaction(SIGBUS, &action, NULL)) return 94;
        if (!sigsetjmp(fault, 1)) {
            for (size_t offset = 0; offset < size; offset += 4096) mapping[offset] = 1;
            return 95;
        }
        if (munmap((void *)mapping, size) || close(file) || unlink("mapping")) return 96;
    } else if (!strcmp(argv[1], "blocked-output-limit")) {
        sigset_t signals;
        sigemptyset(&signals);
        sigaddset(&signals, SIGXFSZ);
        if (sigprocmask(SIG_BLOCK, &signals, NULL)) return 91;
        if (lseek(STDOUT_FILENO, 512 * 1024 * 1024, SEEK_SET) < 0) return 92;
        if (write(STDOUT_FILENO, "x", 1) != -1 || errno != EFBIG) return 93;
        if (ftruncate(STDOUT_FILENO, 0) || lseek(STDOUT_FILENO, 0, SEEK_SET) < 0) return 94;
    } else if (!strcmp(argv[1], "thread-exec")) {
        pthread_t thread;
        if (pthread_create(&thread, NULL, exec_thread, NULL)) return 91;
        pthread_join(thread, NULL);
        return 92;
    } else if (!strcmp(argv[1], "syscalls")) {
        for (int i = 0; i < 10000; ++i) {
            if (syscall(SYS_getpid) <= 0) return 91;
        }
    } else if (!strcmp(argv[1], "x32-syscall")) {
        syscall(0x40000000 | SYS_getpid);
        return 91; /* The seccomp ABI guard must kill this attempt. */
    } else {
        return 90;
    }
    puts("probe-complete");
    return 0;
}
