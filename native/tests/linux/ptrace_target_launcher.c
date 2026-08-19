#define _GNU_SOURCE

#include <errno.h>
#include <stdio.h>
#include <sys/prctl.h>
#include <unistd.h>

int main(int argc, char **argv) {
    if (argc < 2) {
        fprintf(stderr, "target command is required\n");
        return 2;
    }
    if (prctl(PR_SET_PTRACER, PR_SET_PTRACER_ANY, 0, 0, 0) != 0) {
        perror("prctl(PR_SET_PTRACER_ANY)");
        return 1;
    }
    execvp(argv[1], argv + 1);
    perror("execvp");
    return errno == ENOENT ? 127 : 126;
}
