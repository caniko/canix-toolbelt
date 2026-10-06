/* Test-only fsync EIO injection for disposable controller children. */
#include <errno.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <unistd.h>

int fsync(int fd) {
    struct stat metadata;
    const char *kind = getenv("CANIX_ADMISSION_FSYNC_FAULT");
    if (kind != NULL && fstat(fd, &metadata) == 0 &&
        ((strcmp(kind, "file") == 0 && S_ISREG(metadata.st_mode)) ||
         (strcmp(kind, "directory") == 0 && S_ISDIR(metadata.st_mode)))) {
        errno = EIO;
        return -1;
    }
    return (int)syscall(SYS_fsync, fd);
}
