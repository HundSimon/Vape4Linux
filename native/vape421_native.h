#ifndef VAPE421_NATIVE_H
#define VAPE421_NATIVE_H

#ifdef _WIN32
#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#include <windows.h>
#else
#include <pthread.h>
#include <stddef.h>
#include <stdlib.h>

typedef void *HMODULE;
typedef long LONG;
typedef size_t SIZE_T;
typedef pthread_rwlock_t SRWLOCK;

#define SRWLOCK_INIT PTHREAD_RWLOCK_INITIALIZER
#define HEAP_ZERO_MEMORY 1

static inline void AcquireSRWLockExclusive(SRWLOCK *lock) {
    pthread_rwlock_wrlock(lock);
}

static inline void ReleaseSRWLockExclusive(SRWLOCK *lock) {
    pthread_rwlock_unlock(lock);
}

static inline void AcquireSRWLockShared(SRWLOCK *lock) {
    pthread_rwlock_rdlock(lock);
}

static inline void ReleaseSRWLockShared(SRWLOCK *lock) {
    pthread_rwlock_unlock(lock);
}

static inline LONG InterlockedExchange(volatile LONG *target, LONG value) {
    return __atomic_exchange_n(target, value, __ATOMIC_SEQ_CST);
}

static inline LONG InterlockedCompareExchange(
        volatile LONG *target, LONG value, LONG expected) {
    __atomic_compare_exchange_n(target, &expected, value, 0,
            __ATOMIC_SEQ_CST, __ATOMIC_SEQ_CST);
    return expected;
}

static inline void *GetProcessHeap(void) {
    return NULL;
}

static inline void *HeapAlloc(void *heap, unsigned long flags, SIZE_T size) {
    (void)heap;
    return flags == HEAP_ZERO_MEMORY ? calloc(1, size) : malloc(size);
}

static inline int HeapFree(void *heap, unsigned long flags, void *memory) {
    (void)heap;
    (void)flags;
    free(memory);
    return 1;
}
#endif
#include <jni.h>
#include <jvmti.h>

#ifdef __cplusplus
extern "C" {
#endif

extern JavaVM *g_vm;
extern jvmtiEnv *g_jvmti;
extern HMODULE g_module;

void vape_log(const wchar_t *format, ...);
void vape_log_pending_exception(JNIEnv *env, const wchar_t *context);
jint vape_initialize_jvmti(JavaVM *vm);
jint vape_register_native_bridge(JNIEnv *env, jclass bridge_class);
void vape_release_native_bridge(JNIEnv *env);

#ifdef __cplusplus
}
#endif

#endif
