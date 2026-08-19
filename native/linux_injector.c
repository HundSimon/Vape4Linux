#define _GNU_SOURCE

#include <dlfcn.h>
#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/ptrace.h>
#include <sys/types.h>
#include <sys/user.h>
#include <sys/wait.h>
#include <unistd.h>

#if !defined(__x86_64__)
#error "Vape421LinuxNativeInjector supports x86_64 only"
#endif

#define REMOTE_MEMORY_SIZE (1024UL * 1024UL)
#define MAX_TARGET_THREADS 8192

typedef struct attached_process {
    pid_t pid;
    pid_t control_tid;
    pid_t tids[MAX_TARGET_THREADS];
    size_t tid_count;
    struct user_regs_struct original_registers;
    unsigned long original_instruction;
    int attached;
    int instruction_saved;
} attached_process;

static void usage(const char *program) {
    fprintf(stderr, "Usage: %s <pid> <agent.so> <payload.jar>\n", program);
}

static int regular_file(const char *input, char output[PATH_MAX]) {
    return realpath(input, output) != NULL && access(output, R_OK) == 0;
}

static int wait_for_stop(pid_t pid, int *signal_number) {
    int status;
    if (waitpid(pid, &status, __WALL) != pid || !WIFSTOPPED(status)) {
        return 0;
    }
    *signal_number = WSTOPSIG(status);
    return 1;
}

static int has_tid(const attached_process *process, pid_t tid) {
    size_t index;
    for (index = 0; index < process->tid_count; ++index) {
        if (process->tids[index] == tid) return 1;
    }
    return 0;
}

static int attach_one_thread(attached_process *process, pid_t tid) {
    int signal_number;
    if (has_tid(process, tid)) return 1;
    if (process->tid_count >= MAX_TARGET_THREADS) {
        fprintf(stderr, "Target has more than %d threads.\n", MAX_TARGET_THREADS);
        return 0;
    }
    if (ptrace(PTRACE_ATTACH, tid, NULL, NULL) != 0) {
        if (errno == ESRCH) return 1;
        if (errno == EPERM) {
            fprintf(stderr,
                    "ptrace attach was denied. With Yama ptrace_scope=1, "
                    "run this force mode through sudo or grant CAP_SYS_PTRACE.\n");
        } else {
            perror("ptrace(PTRACE_ATTACH)");
        }
        return 0;
    }
    process->tids[process->tid_count++] = tid;
    process->attached = 1;
    if (!wait_for_stop(tid, &signal_number)) {
        perror("wait for target thread");
        return 0;
    }
    return 1;
}

static int attach_process(attached_process *process, pid_t pid) {
    char task_path[64];
    int added;
    memset(process, 0, sizeof(*process));
    process->pid = pid;
    snprintf(task_path, sizeof(task_path), "/proc/%ld/task", (long)pid);
    do {
        DIR *directory = opendir(task_path);
        struct dirent *entry;
        added = 0;
        if (directory == NULL) {
            perror("open target task directory");
            return 0;
        }
        while ((entry = readdir(directory)) != NULL) {
            char *end = NULL;
            long parsed;
            pid_t tid;
            if (entry->d_name[0] == '.') continue;
            errno = 0;
            parsed = strtol(entry->d_name, &end, 10);
            if (errno != 0 || end == entry->d_name || *end != '\0'
                    || parsed <= 1) {
                continue;
            }
            tid = (pid_t)parsed;
            if ((long)tid != parsed || has_tid(process, tid)) continue;
            if (!attach_one_thread(process, tid)) {
                closedir(directory);
                return 0;
            }
            added = 1;
        }
        closedir(directory);
    } while (added);
    if (process->tid_count == 0) return 0;
    process->control_tid = has_tid(process, pid) ? pid : process->tids[0];
    if (ptrace(PTRACE_GETREGS, process->control_tid, NULL,
                    &process->original_registers) != 0) {
        perror("capture target registers");
        return 0;
    }
    errno = 0;
    process->original_instruction = (unsigned long)ptrace(
            PTRACE_PEEKTEXT, process->control_tid,
            (void *)process->original_registers.rip, NULL);
    if (process->original_instruction == (unsigned long)-1 && errno != 0) {
        perror("read target instruction");
        return 0;
    }
    process->instruction_saved = 1;
    return 1;
}

static void restore_and_detach(attached_process *process) {
    size_t index;
    if (!process->attached) return;
    if (process->instruction_saved) {
        ptrace(PTRACE_POKETEXT, process->control_tid,
                (void *)process->original_registers.rip,
                (void *)process->original_instruction);
        ptrace(PTRACE_SETREGS, process->control_tid, NULL,
                &process->original_registers);
    }
    for (index = 0; index < process->tid_count; ++index) {
        ptrace(PTRACE_DETACH, process->tids[index], NULL, NULL);
    }
    process->tid_count = 0;
    process->attached = 0;
}

static int resume_other_threads(attached_process *process) {
    size_t index;
    int found_control = 0;
    for (index = 0; index < process->tid_count; ++index) {
        pid_t tid = process->tids[index];
        if (tid == process->control_tid) {
            found_control = 1;
            continue;
        }
        if (ptrace(PTRACE_DETACH, tid, NULL, NULL) != 0) {
            perror("detach secondary target thread");
            return 0;
        }
    }
    if (!found_control) return 0;
    process->tids[0] = process->control_tid;
    process->tid_count = 1;
    return 1;
}

static int write_remote(pid_t pid, uintptr_t address,
        const void *buffer, size_t length) {
    const unsigned char *bytes = (const unsigned char *)buffer;
    size_t offset = 0;
    while (offset < length) {
        unsigned long word = 0;
        size_t chunk = length - offset;
        if (chunk > sizeof(word)) chunk = sizeof(word);
        if (chunk != sizeof(word)) {
            errno = 0;
            word = (unsigned long)ptrace(PTRACE_PEEKDATA, pid,
                    (void *)(address + offset), NULL);
            if (word == (unsigned long)-1 && errno != 0) return 0;
        }
        memcpy(&word, bytes + offset, chunk);
        if (ptrace(PTRACE_POKEDATA, pid, (void *)(address + offset),
                (void *)word) != 0) {
            return 0;
        }
        offset += chunk;
    }
    return 1;
}

static long remote_syscall(attached_process *process, long number,
        unsigned long first, unsigned long second, unsigned long third,
        unsigned long fourth, unsigned long fifth, unsigned long sixth) {
    struct user_regs_struct registers = process->original_registers;
    unsigned long instruction = process->original_instruction;
    int signal_number;

    /* syscall; int3 -- the remaining bytes retain the original instruction. */
    instruction = (instruction & ~0x00ffffffUL) | 0x00cc050fUL;
    if (ptrace(PTRACE_POKETEXT, process->control_tid,
                (void *)registers.rip, (void *)instruction) != 0) {
        return -1;
    }
    registers.rax = (unsigned long)number;
    registers.rdi = first;
    registers.rsi = second;
    registers.rdx = third;
    registers.r10 = fourth;
    registers.r8 = fifth;
    registers.r9 = sixth;
    if (ptrace(PTRACE_SETREGS, process->control_tid, NULL, &registers) != 0
            || ptrace(PTRACE_CONT, process->control_tid, NULL, NULL) != 0
            || !wait_for_stop(process->control_tid, &signal_number)
            || signal_number != SIGTRAP
            || ptrace(PTRACE_GETREGS, process->control_tid, NULL, &registers) != 0) {
        return -1;
    }
    if (ptrace(PTRACE_POKETEXT, process->control_tid,
                (void *)process->original_registers.rip,
                (void *)process->original_instruction) != 0
            || ptrace(PTRACE_SETREGS, process->control_tid, NULL,
                &process->original_registers) != 0) {
        return -1;
    }
    return (long)registers.rax;
}

static long remote_syscall_at(attached_process *process, uintptr_t trampoline,
        long number, unsigned long first, unsigned long second,
        unsigned long third, unsigned long fourth, unsigned long fifth,
        unsigned long sixth) {
    struct user_regs_struct registers = process->original_registers;
    int signal_number;
    registers.rip = trampoline;
    registers.rax = (unsigned long)number;
    registers.rdi = first;
    registers.rsi = second;
    registers.rdx = third;
    registers.r10 = fourth;
    registers.r8 = fifth;
    registers.r9 = sixth;
    if (ptrace(PTRACE_SETREGS, process->control_tid, NULL, &registers) != 0
            || ptrace(PTRACE_CONT, process->control_tid, NULL, NULL) != 0
            || !wait_for_stop(process->control_tid, &signal_number)
            || signal_number != SIGTRAP
            || ptrace(PTRACE_GETREGS, process->control_tid, NULL,
                &registers) != 0
            || registers.rip != trampoline + 3) {
        return -1;
    }
    return (long)registers.rax;
}

static int remote_call(attached_process *process, uintptr_t trampoline,
        uintptr_t function,
        uintptr_t remote_stack_top, unsigned long first, unsigned long second,
        unsigned long *result) {
    struct user_regs_struct registers = process->original_registers;
    uintptr_t stack_pointer = remote_stack_top & ~(uintptr_t)0xf;
    int signal_number;

    registers.rip = trampoline;
    registers.rsp = stack_pointer;
    registers.rax = function;
    registers.rdi = first;
    registers.rsi = second;
    if (ptrace(PTRACE_SETREGS, process->control_tid, NULL, &registers) != 0
            || ptrace(PTRACE_CONT, process->control_tid, NULL, NULL) != 0
            || !wait_for_stop(process->control_tid, &signal_number)
            || ptrace(PTRACE_GETREGS, process->control_tid, NULL, &registers) != 0) {
        return 0;
    }
    if (signal_number != SIGTRAP
            || registers.rip != trampoline + 3) {
        fprintf(stderr,
                "remote call stopped unexpectedly (signal=%d, rip=0x%llx)\n",
                signal_number, (unsigned long long)registers.rip);
        return 0;
    }
    *result = registers.rax;
    return ptrace(PTRACE_SETREGS, process->control_tid, NULL,
                &process->original_registers) == 0;
}

static uintptr_t module_base(pid_t pid, const char *module_path) {
    char maps_path[64];
    char expected[PATH_MAX];
    char line[PATH_MAX + 160];
    FILE *maps;
    uintptr_t best = 0;
    if (realpath(module_path, expected) == NULL) return 0;
    snprintf(maps_path, sizeof(maps_path), "/proc/%ld/maps", (long)pid);
    maps = fopen(maps_path, "r");
    if (maps == NULL) return 0;
    while (fgets(line, sizeof(line), maps) != NULL) {
        unsigned long long start;
        unsigned long long end;
        unsigned long long offset;
        unsigned long inode;
        char permissions[5];
        char device[16];
        char path[PATH_MAX];
        if (sscanf(line, "%llx-%llx %4s %llx %15s %lu %4095[^\n]",
                &start, &end, permissions, &offset, device, &inode, path) == 7) {
            char *name = path;
            uintptr_t base;
            (void)end;
            (void)permissions;
            (void)device;
            (void)inode;
            while (*name == ' ') ++name;
            if (strcmp(name, expected) != 0) continue;
            base = (uintptr_t)(start - offset);
            if (best == 0 || base < best) best = base;
        }
    }
    fclose(maps);
    return best;
}

static int remote_symbol(pid_t pid, void *local_symbol,
        uintptr_t *remote_address) {
    Dl_info information;
    uintptr_t remote_module;
    if (dladdr(local_symbol, &information) == 0
            || information.dli_fbase == NULL
            || information.dli_fname == NULL) {
        return 0;
    }
    remote_module = module_base(pid, information.dli_fname);
    if (remote_module == 0) return 0;
    *remote_address = remote_module
            + ((uintptr_t)local_symbol - (uintptr_t)information.dli_fbase);
    return 1;
}

static int parse_pid(const char *value, pid_t *pid) {
    char *end = NULL;
    long parsed;
    errno = 0;
    parsed = strtol(value, &end, 10);
    if (errno != 0 || end == value || *end != '\0' || parsed <= 1) return 0;
    *pid = (pid_t)parsed;
    return (long)*pid == parsed;
}

int main(int argc, char **argv) {
    pid_t pid;
    char agent_path[PATH_MAX];
    char payload_path[PATH_MAX];
    attached_process process;
    void *local_agent = NULL;
    void *local_bootstrap;
    void *local_dlopen;
    uintptr_t remote_dlopen;
    uintptr_t remote_agent_base;
    uintptr_t remote_bootstrap;
    uintptr_t remote_memory = 0;
    uintptr_t remote_agent_path;
    uintptr_t remote_payload_path;
    uintptr_t remote_stack_top;
    uintptr_t remote_call_trampoline;
    uintptr_t remote_syscall_trampoline;
    Dl_info bootstrap_information;
    unsigned long call_result;
    long syscall_result;
    int remote_trampoline_executable = 0;
    int exit_code = 1;

    memset(&process, 0, sizeof(process));
    if (argc != 4 || !parse_pid(argv[1], &pid)) {
        usage(argv[0]);
        return 2;
    }
    if (!regular_file(argv[2], agent_path)
            || !regular_file(argv[3], payload_path)) {
        fprintf(stderr, "Agent or payload is missing or unreadable.\n");
        return 2;
    }
    local_dlopen = dlsym(RTLD_DEFAULT, "dlopen");
    local_agent = dlopen(agent_path, RTLD_NOW | RTLD_LOCAL);
    local_bootstrap = local_agent == NULL ? NULL
            : dlsym(local_agent, "Vape421_ForceBootstrap");
    if (local_dlopen == NULL || local_bootstrap == NULL
            || dladdr(local_bootstrap, &bootstrap_information) == 0) {
        fprintf(stderr, "Could not resolve injector symbols: %s\n", dlerror());
        goto cleanup;
    }
    if (!attach_process(&process, pid)) goto cleanup;
    if (!remote_symbol(pid, local_dlopen, &remote_dlopen)) {
        fprintf(stderr, "Could not resolve target dlopen address.\n");
        goto cleanup;
    }
    syscall_result = remote_syscall(&process, 9 /* mmap */, 0,
            REMOTE_MEMORY_SIZE, PROT_READ | PROT_WRITE,
            MAP_PRIVATE | MAP_ANONYMOUS, (unsigned long)-1, 0);
    if (syscall_result < 0 && syscall_result >= -4095) {
        fprintf(stderr, "Remote mmap failed: %s\n", strerror((int)-syscall_result));
        goto cleanup;
    }
    remote_memory = (uintptr_t)syscall_result;
    remote_call_trampoline = remote_memory;
    remote_syscall_trampoline = remote_memory + 0x10;
    remote_agent_path = remote_memory + 0x1000;
    remote_payload_path = remote_memory + 0x3000;
    remote_stack_top = remote_memory + REMOTE_MEMORY_SIZE;
    {
        const unsigned char call_code[] = {0xff, 0xd0, 0xcc};
        const unsigned char syscall_code[] = {0x0f, 0x05, 0xcc};
        if (!write_remote(process.control_tid, remote_call_trampoline,
                    call_code, sizeof(call_code))
                || !write_remote(process.control_tid, remote_syscall_trampoline,
                    syscall_code, sizeof(syscall_code))) {
            perror("write target trampoline");
            goto cleanup;
        }
    }
    if (!write_remote(process.control_tid, remote_agent_path,
                agent_path, strlen(agent_path) + 1)
            || !write_remote(process.control_tid, remote_payload_path,
                payload_path, strlen(payload_path) + 1)) {
        perror("write target memory");
        goto cleanup;
    }
    syscall_result = remote_syscall(&process, 10 /* mprotect */,
            remote_memory, (unsigned long)sysconf(_SC_PAGESIZE),
            PROT_READ | PROT_EXEC, 0, 0, 0);
    if (syscall_result != 0) {
        fprintf(stderr, "Remote mprotect failed: %s\n",
                syscall_result < 0 && syscall_result >= -4095
                    ? strerror((int)-syscall_result) : "unexpected result");
        goto cleanup;
    }
    remote_trampoline_executable = 1;
    if (!resume_other_threads(&process)) goto cleanup;
    if (!remote_call(&process, remote_call_trampoline, remote_dlopen,
                remote_stack_top,
                remote_agent_path, RTLD_NOW | RTLD_GLOBAL, &call_result)
            || call_result == 0) {
        fprintf(stderr, "Remote dlopen failed.\n");
        goto cleanup;
    }
    remote_agent_base = module_base(pid, agent_path);
    if (remote_agent_base == 0) {
        fprintf(stderr, "Agent loaded but its target mapping was not found.\n");
        goto cleanup;
    }
    remote_bootstrap = remote_agent_base
            + ((uintptr_t)local_bootstrap
                - (uintptr_t)bootstrap_information.dli_fbase);
    if (!remote_call(&process, remote_call_trampoline, remote_bootstrap,
                remote_stack_top,
                remote_payload_path, 0, &call_result)
            || call_result != 1) {
        fprintf(stderr, "Target native bootstrap did not start.\n");
        goto cleanup;
    }
    printf("Loaded %s and started its native bootstrap in PID %ld\n",
            agent_path, (long)pid);
    exit_code = 0;

cleanup:
    if (process.attached && remote_memory != 0) {
        if (remote_trampoline_executable) {
            remote_syscall_at(&process, remote_syscall_trampoline,
                    11 /* munmap */, remote_memory,
                    REMOTE_MEMORY_SIZE, 0, 0, 0, 0);
        } else {
            remote_syscall(&process, 11 /* munmap */, remote_memory,
                    REMOTE_MEMORY_SIZE, 0, 0, 0, 0);
        }
    }
    restore_and_detach(&process);
    if (local_agent != NULL) dlclose(local_agent);
    return exit_code;
}
