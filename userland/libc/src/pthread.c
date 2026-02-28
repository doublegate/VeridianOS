/*
 * VeridianOS libc -- pthread.c
 *
 * Minimal POSIX pthreads implementation for VeridianOS self-hosting.
 *
 * This provides a subset of POSIX threading primitives sufficient for
 * cross-compiling toolchains (GCC, binutils) and build systems (make, ninja)
 * on VeridianOS. It does NOT aim for full POSIX compliance -- only the
 * primitives actually used by self-hosting toolchain code are implemented.
 *
 * Threading model:
 *   - Threads are created via SYS_THREAD_CLONE (Linux clone-style semantics).
 *   - Synchronization uses SYS_FUTEX (futex_wait / futex_wake / futex_requeue).
 *   - Thread joining uses SYS_THREAD_JOIN with CLONE_CHILD_CLEARTID for
 *     robust exit notification.
 *   - No kernel-side helper threads are required.
 *
 * Primitives implemented:
 *   - Mutex (futex-backed, two-state)
 *   - Condition variable (futex-backed sequence counter)
 *   - Once (atomic CAS + futex for secondary waiters)
 *   - Read-write lock (reader count + writer flag + futex)
 *   - TLS keys (global key table, per-thread value arrays, destructors)
 *   - Barrier (atomic counter + futex, cyclic via phase counter)
 *   - Spinlock (atomic CAS busy-wait, no futex)
 *
 * Limitations:
 *   - No thread cancellation (pthread_cancel is not implemented).
 *   - No robust mutexes or priority inheritance.
 *   - Condition variable timedwait does not handle clock selection.
 *   - TLS key destructors are invoked at pthread_exit() only, not process exit.
 *   - PTHREAD_KEYS_MAX = 128; PTHREAD_THREADS_MAX for TLS storage = 256.
 */

#include <pthread.h>
#include <errno.h>
#include <stdbool.h>
#include <stdlib.h>
#include <stdint.h>
#include <string.h>
#include <time.h>
#include <limits.h>

#include <veridian/syscall.h>

/* Imported raw syscall helpers */
long veridian_thread_clone(unsigned long flags, void *newsp,
                           int *parent_tidptr, int *child_tidptr, void *tls);
int futex_wait(int *uaddr, int expected, const struct timespec *timeout);
int futex_wake(int *uaddr, int count);
int futex_requeue(int *uaddr, int wake_count, int *uaddr2, int requeue_count);
pid_t gettid(void);
int arch_prctl(int code, unsigned long addr);

/* Futex correctness gating ------------------------------------------------ */
static int futex_ready = 0;

static inline int ensure_futex_ready(void)
{
    if (__atomic_load_n(&futex_ready, __ATOMIC_ACQUIRE))
        return 0;

    int probe = 1;
    /* Expect EWOULDBLOCK when value mismatches. */
    int r = futex_wait(&probe, 0, NULL);
    if (!(r == -1 && errno == EWOULDBLOCK)) {
        return errno ? errno : EIO;
    }

    /* Wake on an address with no waiters should return 0. */
    r = futex_wake(&probe, 1);
    if (r < 0) {
        return errno ? errno : EIO;
    }

    __atomic_store_n(&futex_ready, 1, __ATOMIC_RELEASE);
    return 0;
}

/* Architecture helpers --------------------------------------------------- */
static inline void set_thread_pointer(void *ptr)
{
#if defined(__x86_64__)
    arch_prctl(ARCH_SET_FS, (unsigned long)ptr);
#elif defined(__aarch64__)
    __asm__ volatile("msr tpidr_el0, %0" :: "r"(ptr));
#elif defined(__riscv)
    __asm__ volatile("mv tp, %0" :: "r"(ptr));
#else
#error "Unsupported arch"
#endif
}

static inline void *get_thread_pointer(void)
{
#if defined(__x86_64__)
    return __builtin_thread_pointer();
#elif defined(__aarch64__)
    return __builtin_thread_pointer();
#elif defined(__riscv)
    return __builtin_thread_pointer();
#else
    return NULL;
#endif
}

/* Thread control block --------------------------------------------------- */
struct pthread_control {
    pthread_t tid;
    void *stack;       /* aligned usable stack base */
    void *stack_alloc; /* original pointer for free */
    size_t stack_size;
    int detached;
    int exit_futex; /* used by kernel CLONE_CHILD_CLEARTID */
    void *retval;
    void *(*start)(void *);
    void *arg;
};

static struct pthread_control main_tcb;
static int main_tcb_ready = 0;

static void ensure_main_tcb(void)
{
    if (main_tcb_ready)
        return;
    main_tcb.tid = (pthread_t)gettid();
    main_tcb.stack = NULL;
    main_tcb.stack_size = 0;
    main_tcb.detached = 0;
    main_tcb.exit_futex = 0;
    main_tcb.retval = NULL;
    main_tcb.start = NULL;
    main_tcb.arg = NULL;
    set_thread_pointer(&main_tcb);
    main_tcb_ready = 1;
}

/* Spinlock for internal lists.
 *
 * Uses an atomic exchange for the fast path, falling back to futex_wait when
 * contended. The futex return value is checked: EINTR (spurious wakeup or
 * signal) causes a retry, and EWOULDBLOCK (value changed) is expected and
 * also retries. Any other error breaks out to retry the CAS -- the worst
 * case is spinning, which is acceptable for a short critical section.
 */
static int global_lock = 0;
static inline void lock_global(void)
{
    while (__atomic_exchange_n(&global_lock, 1, __ATOMIC_ACQUIRE)) {
        int r = futex_wait(&global_lock, 1, NULL);
        if (r == -1 && errno != EINTR && errno != EWOULDBLOCK) {
            /* Unexpected futex error -- fall through to retry the CAS.
             * For a short critical section this degenerates to a spinlock,
             * which is acceptable. */
        }
    }
}
static inline void unlock_global(void)
{
    __atomic_store_n(&global_lock, 0, __ATOMIC_RELEASE);
    int r = futex_wake(&global_lock, 1);
    (void)r; /* Wake failure is benign: worst case a waiter spins once. */
}

/* Simple registry of live TCBs */
struct tcb_node {
    struct pthread_control *tcb;
    struct tcb_node *next;
};
static struct tcb_node *tcb_head = NULL;

static void register_tcb(struct pthread_control *tcb)
{
    lock_global();
    struct tcb_node *n = malloc(sizeof(*n));
    if (n) {
        n->tcb = tcb;
        n->next = tcb_head;
        tcb_head = n;
    }
    unlock_global();
}

static struct pthread_control *lookup_tcb(pthread_t tid)
{
    lock_global();
    struct tcb_node *n = tcb_head;
    while (n) {
        if (n->tcb->tid == tid) {
            struct pthread_control *t = n->tcb;
            unlock_global();
            return t;
        }
        n = n->next;
    }
    unlock_global();
    return NULL;
}

static void unregister_tcb(pthread_t tid)
{
    lock_global();
    struct tcb_node **cur = &tcb_head;
    while (*cur) {
        if ((*cur)->tcb->tid == tid) {
            struct tcb_node *dead = *cur;
            *cur = dead->next;
            free(dead->tcb->stack_alloc);
            free(dead);
            break;
        }
        cur = &((*cur)->next);
    }
    unlock_global();
}

/* Mutex ------------------------------------------------------------------ */
int pthread_mutex_init(pthread_mutex_t *mutex, const pthread_mutexattr_t *attr)
{
    (void)attr;
    int err = ensure_futex_ready();
    if (err) {
        return err;
    }
    mutex->state = 0;
    return 0;
}

int pthread_mutex_destroy(pthread_mutex_t *mutex)
{
    (void)mutex;
    return 0;
}

/*
 * pthread_mutex_lock -- acquire a mutex, blocking if necessary.
 *
 * Futex-based mutex implementation:
 *   state == 0: unlocked
 *   state == 1: locked
 *
 *   Fast path: atomic CAS 0 -> 1. If the mutex is uncontended this is a
 *   single atomic instruction with no syscall overhead.
 *
 *   Slow path: if the CAS fails (mutex is held), we call futex_wait() to
 *   sleep until the holder calls futex_wake() in pthread_mutex_unlock().
 *   On wakeup we retry the CAS -- spurious wakeups and races between
 *   multiple waiters are handled by the retry loop.
 */
int pthread_mutex_lock(pthread_mutex_t *mutex)
{
    int err = ensure_futex_ready();
    if (err) {
        return err;
    }

    /* Fast path: uncontended lock */
    int expected = 0;
    if (__atomic_compare_exchange_n(&mutex->state, &expected, 1, false,
                                    __ATOMIC_ACQUIRE, __ATOMIC_RELAXED)) {
        return 0;
    }
    /* Slow path: contended -- sleep on the futex until the lock is released */
    while (1) {
        expected = 0;
        if (__atomic_compare_exchange_n(&mutex->state, &expected, 1, false,
                                        __ATOMIC_ACQUIRE, __ATOMIC_RELAXED)) {
            return 0;
        }
        futex_wait(&mutex->state, 1, NULL);
    }
}

int pthread_mutex_trylock(pthread_mutex_t *mutex)
{
    int err = ensure_futex_ready();
    if (err) {
        return err;
    }

    int expected = 0;
    if (__atomic_compare_exchange_n(&mutex->state, &expected, 1, false,
                                    __ATOMIC_ACQUIRE, __ATOMIC_RELAXED)) {
        return 0;
    }
    return EBUSY;
}

/*
 * pthread_mutex_unlock -- release a mutex and wake one waiter.
 *
 * Atomically stores 0 (unlocked) with release ordering to ensure all
 * preceding critical section writes are visible before the lock appears
 * free. Then calls futex_wake() to unblock at most one waiter sleeping
 * in pthread_mutex_lock().
 *
 * We always call futex_wake() even if there might be no waiters, because
 * tracking the "contended" state would require a 3-state protocol. The
 * wake syscall on an empty wait queue is cheap (returns 0 immediately).
 */
int pthread_mutex_unlock(pthread_mutex_t *mutex)
{
    int err = ensure_futex_ready();
    if (err) {
        return err;
    }

    __atomic_store_n(&mutex->state, 0, __ATOMIC_RELEASE);
    futex_wake(&mutex->state, 1);
    return 0;
}

/* Condition variable ----------------------------------------------------- */
int pthread_cond_init(pthread_cond_t *cond, const pthread_condattr_t *attr)
{
    (void)attr;
    int err = ensure_futex_ready();
    if (err) {
        return err;
    }
    cond->futex = 0;
    return 0;
}

int pthread_cond_destroy(pthread_cond_t *cond)
{
    (void)cond;
    return 0;
}

int pthread_cond_wait(pthread_cond_t *cond, pthread_mutex_t *mutex)
{
    int err = ensure_futex_ready();
    if (err) {
        return err;
    }

    int seq = __atomic_load_n(&cond->futex, __ATOMIC_SEQ_CST);
    pthread_mutex_unlock(mutex);
    futex_wait(&cond->futex, seq, NULL);
    pthread_mutex_lock(mutex);
    return 0;
}

int pthread_cond_timedwait(pthread_cond_t *cond, pthread_mutex_t *mutex,
                           const struct timespec *abstime)
{
    int err = ensure_futex_ready();
    if (err) {
        return err;
    }

    int seq = __atomic_load_n(&cond->futex, __ATOMIC_SEQ_CST);
    pthread_mutex_unlock(mutex);
    futex_wait(&cond->futex, seq, abstime);
    pthread_mutex_lock(mutex);
    return 0;
}

int pthread_cond_signal(pthread_cond_t *cond)
{
    __atomic_fetch_add(&cond->futex, 1, __ATOMIC_SEQ_CST);
    futex_wake(&cond->futex, 1);
    return 0;
}

int pthread_cond_broadcast(pthread_cond_t *cond)
{
    __atomic_fetch_add(&cond->futex, 1, __ATOMIC_SEQ_CST);
    futex_wake(&cond->futex, INT32_MAX);
    return 0;
}

/* Once ------------------------------------------------------------------- */
int pthread_once(pthread_once_t *once_control, void (*init_routine)(void))
{
    if (__atomic_load_n(&once_control->done, __ATOMIC_ACQUIRE))
        return 0;
    int expected = 0;
    if (__atomic_compare_exchange_n(&once_control->done, &expected, 1, false,
                                    __ATOMIC_ACQ_REL, __ATOMIC_RELAXED)) {
        init_routine();
        return 0;
    }
    while (!__atomic_load_n(&once_control->done, __ATOMIC_ACQUIRE)) {
        futex_wait(&once_control->done, 1, NULL);
    }
    return 0;
}

/* Threads ---------------------------------------------------------------- */
struct start_args {
    struct pthread_control *tcb;
    void *(*start)(void *);
    void *arg;
};

static void child_trampoline(struct pthread_control *tcb)
{
    set_thread_pointer(tcb);
    tcb->tid = (pthread_t)gettid();
    void *rv = tcb->start(tcb->arg);
    /* Run TLS destructors then exit.  pthread_exit() is the public symbol
     * defined at the bottom of this file; call it directly. */
    pthread_exit(rv);
    __builtin_unreachable();
}

/*
 * pthread_create -- create a new thread.
 *
 * Stack allocation strategy:
 *   We malloc(stack_size + 16) to guarantee 16-byte alignment for the ABI
 *   (x86_64 System V, AArch64 AAPCS64). The raw malloc'd pointer is saved
 *   in tcb->stack_alloc for later free(); the aligned pointer is used as
 *   the usable stack base.
 *
 * TCB (Thread Control Block) setup:
 *   A heap-allocated pthread_control struct holds per-thread metadata: the
 *   stack pointers, detach state, return value, start routine, and the
 *   exit_futex used by CLONE_CHILD_CLEARTID for join notification.
 *
 * Clone syscall usage:
 *   We call veridian_thread_clone() with CLONE_VM | CLONE_FS | CLONE_FILES |
 *   CLONE_SIGHAND | CLONE_THREAD (shared address space, file table, and
 *   signal handlers) plus CLONE_SETTLS (set the thread pointer to the TCB),
 *   CLONE_CHILD_CLEARTID (kernel zeroes exit_futex on exit and does a futex
 *   wake, enabling join), CLONE_CHILD_SETTID and CLONE_PARENT_SETTID (kernel
 *   writes the new TID into the TCB).
 *
 * The TCB is registered in the global TCB list only AFTER the clone syscall
 * succeeds, so that a failed clone never leaves a dangling registry entry.
 */
int pthread_create(pthread_t *thread,
                   const pthread_attr_t *attr,
                   void *(*start_routine)(void *),
                   void *arg)
{
    int err = ensure_futex_ready();
    if (err) {
        return err;
    }
    ensure_main_tcb();

    size_t stack_size = (attr && attr->stack_size) ? attr->stack_size : (1024 * 1024);

    /* Allocate stack_size + 16 bytes so we can manually align to a 16-byte
     * boundary. Save the raw pointer for free(). */
    void *raw = malloc(stack_size + 16);
    if (!raw) {
        return ENOMEM;
    }
    uintptr_t aligned = ((uintptr_t)raw + 15) & ~(uintptr_t)0xF;
    void *stack = (void *)aligned;

    struct pthread_control *tcb = calloc(1, sizeof(*tcb));
    if (!tcb) {
        free(raw);  /* free the original malloc'd pointer, not the aligned one */
        return ENOMEM;
    }
    tcb->stack = stack;
    tcb->stack_alloc = raw;
    tcb->stack_size = stack_size;
    tcb->detached = (attr && attr->detach_state) ? 1 : 0;
    tcb->start = start_routine;
    tcb->arg = arg;
    tcb->exit_futex = 0;

    /* Set up the child stack: push start_routine and arg at the top so the
     * kernel entry trampoline can pop them. Stack grows downward. */
    void **child_stack_top = (void **)((uint8_t *)stack + stack_size);
    *(--child_stack_top) = arg;
    *(--child_stack_top) = (void *)start_routine;
    unsigned long flags = CLONE_VM | CLONE_FS | CLONE_FILES | CLONE_SIGHAND |
                          CLONE_THREAD | CLONE_SETTLS |
                          CLONE_CHILD_CLEARTID | CLONE_CHILD_SETTID | CLONE_PARENT_SETTID;

    /* NOTE: register_tcb() is intentionally NOT called here -- it is called
     * below, only after clone succeeds, to avoid leaving a dangling TCB
     * registry entry on failure. */
    long ret = veridian_thread_clone(flags,
                                     child_stack_top,
                                     (int *)&tcb->tid,
                                     (int *)&tcb->tid,
                                     tcb);
    if (ret < 0) {
        int clone_err = errno;
        free(raw);
        free(tcb);
        return clone_err ? clone_err : (int)-ret;
    }

    if (ret == 0) {
        /* Child path -- never returns */
        child_trampoline(tcb);
    }

    /* Parent path: clone succeeded, now register the TCB. */
    tcb->tid = (pthread_t)ret;
    register_tcb(tcb);
    if (thread) {
        *thread = (pthread_t)ret;
    }
    return 0;
}

/*
 * pthread_join -- wait for a thread to terminate and retrieve its return value.
 *
 * Futex-based wait mechanism:
 *   The target thread was created with CLONE_CHILD_CLEARTID, so when it exits
 *   the kernel atomically zeroes its exit_futex and performs a futex wake on
 *   that address. SYS_THREAD_JOIN blocks the caller until that wake occurs
 *   (or returns immediately if the thread has already exited).
 *
 *   After the join syscall returns, we read the return value from the TCB
 *   (which the child wrote before calling SYS_THREAD_EXIT), then unregister
 *   and free the TCB and its stack allocation.
 */
int pthread_join(pthread_t thread, void **retval)
{
    ensure_main_tcb();
    struct pthread_control *tcb = lookup_tcb(thread);
    if (!tcb) {
        return ESRCH;
    }
    if (tcb->detached) {
        return EINVAL;
    }

    long rv = veridian_syscall2(SYS_THREAD_JOIN, thread, retval ? retval : 0);
    if (rv < 0) {
        return (int)-rv;
    }
    if (retval) {
        *retval = tcb->retval;
    }
    unregister_tcb(thread);
    return 0;
}

int pthread_detach(pthread_t thread)
{
    struct pthread_control *tcb = lookup_tcb(thread);
    if (!tcb) {
        return ESRCH;
    }
    tcb->detached = 1;
    return 0;
}

pthread_t pthread_self(void)
{
    ensure_main_tcb();
    struct pthread_control *tcb = (struct pthread_control *)get_thread_pointer();
    if (tcb) {
        return tcb->tid;
    }
    return (pthread_t)gettid();
}

/*
 * pthread_exit_raw -- low-level thread exit without destructor handling.
 *
 * The public pthread_exit() is defined later in this file after
 * run_tls_destructors() is available.  child_trampoline() calls this
 * raw version directly because it is a forward reference; the compiler
 * sees only one definition of the public symbol.
 */
static void __attribute__((noreturn)) pthread_exit_raw(void *retval)
{
    struct pthread_control *tcb = (struct pthread_control *)get_thread_pointer();
    if (tcb) {
        tcb->retval = retval;
    }
    veridian_syscall1(SYS_THREAD_EXIT, (long)retval);
    __builtin_unreachable();
}

int pthread_setcancelstate(int state, int *oldstate)
{
    if (oldstate) {
        *oldstate = 0;
    }
    (void)state;
    return 0;
}

int pthread_yield(void)
{
    veridian_syscall0(SYS_PROCESS_YIELD);
    return 0;
}

/* =========================================================================
 * Read-write lock
 * =========================================================================
 *
 * State:
 *   rwlock->readers       -- active reader count; we futex_wait on this when
 *                            a writer needs all readers to drain.
 *   rwlock->writer        -- 1 when a writer holds the lock, 0 otherwise;
 *                            readers futex_wait here until the writer releases.
 *   rwlock->pending_writers -- number of threads waiting to write; a positive
 *                            value causes new readers to spin/wait, giving
 *                            writers priority to prevent writer starvation.
 *
 * Invariants:
 *   (a) writer == 1  =>  readers == 0
 *   (b) readers > 0  =>  writer == 0
 *
 * Read-lock acquisition:
 *   1. If writer==1 or pending_writers>0, block on &writer until it clears.
 *   2. Atomically increment readers.
 *   3. Re-check writer: if a writer snuck in between steps 1 and 2, undo
 *      the increment and retry (avoids ABA window).
 *
 * Write-lock acquisition:
 *   1. Increment pending_writers (signals intent, biases readers).
 *   2. CAS writer: 0 -> 1.  If it fails, futex_wait on &writer until 0.
 *   3. Wait for all active readers to drain: futex_wait on &readers until 0.
 *   4. Decrement pending_writers (the lock is fully ours now).
 *
 * Unlock:
 *   If writer==1: clear it and wake all waiters (readers + any writer).
 *   If readers>0: decrement; if it reaches 0, wake a pending writer.
 */
int pthread_rwlock_init(pthread_rwlock_t *rwlock, const pthread_rwlockattr_t *attr)
{
    (void)attr;
    int err = ensure_futex_ready();
    if (err) {
        return err;
    }
    __atomic_store_n(&rwlock->readers,         0, __ATOMIC_RELEASE);
    __atomic_store_n(&rwlock->writer,          0, __ATOMIC_RELEASE);
    __atomic_store_n(&rwlock->pending_writers, 0, __ATOMIC_RELEASE);
    return 0;
}

int pthread_rwlock_destroy(pthread_rwlock_t *rwlock)
{
    /* Behaviour is undefined if the lock is held at destroy time. */
    (void)rwlock;
    return 0;
}

int pthread_rwlock_rdlock(pthread_rwlock_t *rwlock)
{
    int err = ensure_futex_ready();
    if (err) {
        return err;
    }

    while (1) {
        /* Block while a writer holds the lock or writers are pending. */
        while (__atomic_load_n(&rwlock->writer,          __ATOMIC_ACQUIRE) ||
               __atomic_load_n(&rwlock->pending_writers, __ATOMIC_ACQUIRE)) {
            futex_wait(&rwlock->writer, 1, NULL);
        }

        /* Claim one reader slot. */
        __atomic_fetch_add(&rwlock->readers, 1, __ATOMIC_ACQUIRE);

        /* Verify a writer did not slip in between the check and the increment.
         * If it did, undo and retry -- we must not hold a reader slot while a
         * writer is active. */
        if (!__atomic_load_n(&rwlock->writer, __ATOMIC_ACQUIRE)) {
            return 0; /* success */
        }
        __atomic_fetch_sub(&rwlock->readers, 1, __ATOMIC_RELEASE);
        /* Wake a potential writer waiting for readers==0 if we transiently
         * incremented then decremented. */
        if (__atomic_load_n(&rwlock->readers, __ATOMIC_RELAXED) == 0) {
            futex_wake(&rwlock->readers, 1);
        }
    }
}

int pthread_rwlock_tryrdlock(pthread_rwlock_t *rwlock)
{
    /* Fail immediately if a writer holds or is waiting for the lock. */
    if (__atomic_load_n(&rwlock->writer,          __ATOMIC_ACQUIRE) ||
        __atomic_load_n(&rwlock->pending_writers, __ATOMIC_ACQUIRE)) {
        return EBUSY;
    }
    __atomic_fetch_add(&rwlock->readers, 1, __ATOMIC_ACQUIRE);
    if (!__atomic_load_n(&rwlock->writer, __ATOMIC_ACQUIRE)) {
        return 0;
    }
    /* Writer snuck in -- undo. */
    __atomic_fetch_sub(&rwlock->readers, 1, __ATOMIC_RELEASE);
    if (__atomic_load_n(&rwlock->readers, __ATOMIC_RELAXED) == 0) {
        futex_wake(&rwlock->readers, 1);
    }
    return EBUSY;
}

int pthread_rwlock_wrlock(pthread_rwlock_t *rwlock)
{
    int err = ensure_futex_ready();
    if (err) {
        return err;
    }

    /* Signal intent so new readers yield to us. */
    __atomic_fetch_add(&rwlock->pending_writers, 1, __ATOMIC_ACQUIRE);

    /* Acquire the writer flag. */
    while (1) {
        int expected = 0;
        if (__atomic_compare_exchange_n(&rwlock->writer, &expected, 1, false,
                                        __ATOMIC_ACQUIRE, __ATOMIC_RELAXED)) {
            break;
        }
        /* Another writer holds it -- sleep. */
        futex_wait(&rwlock->writer, 1, NULL);
    }

    /* Wait for all active readers to finish. */
    while (__atomic_load_n(&rwlock->readers, __ATOMIC_ACQUIRE) != 0) {
        futex_wait(&rwlock->readers, __atomic_load_n(&rwlock->readers, __ATOMIC_RELAXED), NULL);
    }

    /* We now hold the exclusive write lock. */
    __atomic_fetch_sub(&rwlock->pending_writers, 1, __ATOMIC_RELEASE);
    return 0;
}

int pthread_rwlock_trywrlock(pthread_rwlock_t *rwlock)
{
    /* Fail immediately if anyone holds the lock. */
    if (__atomic_load_n(&rwlock->readers, __ATOMIC_ACQUIRE) != 0) {
        return EBUSY;
    }
    int expected = 0;
    if (!__atomic_compare_exchange_n(&rwlock->writer, &expected, 1, false,
                                     __ATOMIC_ACQUIRE, __ATOMIC_RELAXED)) {
        return EBUSY;
    }
    /* Double-check no readers crept in between. */
    if (__atomic_load_n(&rwlock->readers, __ATOMIC_ACQUIRE) != 0) {
        __atomic_store_n(&rwlock->writer, 0, __ATOMIC_RELEASE);
        futex_wake(&rwlock->writer, INT32_MAX);
        return EBUSY;
    }
    return 0;
}

int pthread_rwlock_unlock(pthread_rwlock_t *rwlock)
{
    if (__atomic_load_n(&rwlock->writer, __ATOMIC_ACQUIRE)) {
        /* Write-unlock: clear writer and wake all blocked threads so that
         * pending readers or the next writer can proceed. */
        __atomic_store_n(&rwlock->writer, 0, __ATOMIC_RELEASE);
        futex_wake(&rwlock->writer, INT32_MAX);
    } else {
        /* Read-unlock: decrement reader count; if we were the last reader and
         * a writer is pending, wake it. */
        int prev = __atomic_fetch_sub(&rwlock->readers, 1, __ATOMIC_RELEASE);
        if (prev == 1 &&
            __atomic_load_n(&rwlock->pending_writers, __ATOMIC_ACQUIRE)) {
            futex_wake(&rwlock->readers, 1);
        }
    }
    return 0;
}

/* =========================================================================
 * TLS keys (pthread_key_create / delete / getspecific / setspecific)
 * =========================================================================
 *
 * Design:
 *   A global key table of PTHREAD_KEYS_MAX entries holds the destructor
 *   function for each key (NULL destructor = slot available when combined
 *   with the in_use flag).
 *
 *   Per-thread values are stored in a flat 2D array:
 *     tls_values[key][thread_slot]
 *
 *   Thread slots are assigned sequentially and recorded in the TCB via a
 *   global tid-to-slot table (tls_tid_slot[]).  The main thread is slot 0.
 *
 *   Memory for value arrays is allocated lazily (first setspecific call for
 *   a given key allocates the values column).
 *
 *   This is not O(1) per-thread in the worst case but is sufficient for
 *   toolchain use where PTHREAD_KEYS_MAX calls are rare and thread counts
 *   are small.
 *
 * TLS destructor invocation:
 *   pthread_exit() iterates all active keys and calls the destructor for
 *   any non-NULL value belonging to the exiting thread.  Destructor calls
 *   may set new values; we repeat up to PTHREAD_DESTRUCTOR_ITERATIONS times
 *   (POSIX requires at least 4).
 */

#define PTHREAD_THREADS_MAX         256
#define PTHREAD_DESTRUCTOR_ITERATIONS 4

struct tls_key_entry {
    int      in_use;
    void   (*destructor)(void *);
    void    *values[PTHREAD_THREADS_MAX]; /* indexed by thread slot */
};

/* Protected by key_lock (atomic spinlock -- no futex needed here). */
static struct tls_key_entry tls_keys[PTHREAD_KEYS_MAX];
static int key_lock = 0; /* 0=free, 1=held */

/* TID-to-slot mapping.  Slot 0 is reserved for the main thread. */
static pthread_t tls_tid_map[PTHREAD_THREADS_MAX];
static int       tls_slots_used = 0;

static inline void key_lock_acquire(void)
{
    while (__atomic_exchange_n(&key_lock, 1, __ATOMIC_ACQUIRE))
        ; /* spin -- key table operations are brief */
}
static inline void key_lock_release(void)
{
    __atomic_store_n(&key_lock, 0, __ATOMIC_RELEASE);
}

/*
 * Return the slot index for the calling thread, allocating one if this is
 * the first TLS call from this thread.  Returns -1 if PTHREAD_THREADS_MAX
 * is exhausted.
 */
static int get_thread_slot(void)
{
    pthread_t self = pthread_self();
    key_lock_acquire();
    /* Linear scan -- table is small. */
    for (int i = 0; i < tls_slots_used; i++) {
        if (tls_tid_map[i] == self) {
            key_lock_release();
            return i;
        }
    }
    /* Allocate new slot. */
    if (tls_slots_used >= PTHREAD_THREADS_MAX) {
        key_lock_release();
        return -1;
    }
    int slot = tls_slots_used++;
    tls_tid_map[slot] = self;
    key_lock_release();
    return slot;
}

int pthread_key_create(pthread_key_t *key, void (*destructor)(void *))
{
    if (!key) {
        return EINVAL;
    }
    key_lock_acquire();
    for (int k = 0; k < PTHREAD_KEYS_MAX; k++) {
        if (!tls_keys[k].in_use) {
            tls_keys[k].in_use     = 1;
            tls_keys[k].destructor = destructor;
            /* Zero out any stale values from a previous incarnation. */
            for (int t = 0; t < PTHREAD_THREADS_MAX; t++) {
                tls_keys[k].values[t] = NULL;
            }
            key_lock_release();
            *key = (pthread_key_t)k;
            return 0;
        }
    }
    key_lock_release();
    return EAGAIN; /* POSIX: EAGAIN when no free keys */
}

int pthread_key_delete(pthread_key_t key)
{
    if (key < 0 || key >= PTHREAD_KEYS_MAX) {
        return EINVAL;
    }
    key_lock_acquire();
    if (!tls_keys[key].in_use) {
        key_lock_release();
        return EINVAL;
    }
    /* POSIX: does NOT invoke destructors on delete; values simply vanish. */
    tls_keys[key].in_use     = 0;
    tls_keys[key].destructor = NULL;
    for (int t = 0; t < PTHREAD_THREADS_MAX; t++) {
        tls_keys[key].values[t] = NULL;
    }
    key_lock_release();
    return 0;
}

void *pthread_getspecific(pthread_key_t key)
{
    if (key < 0 || key >= PTHREAD_KEYS_MAX) {
        return NULL;
    }
    key_lock_acquire();
    if (!tls_keys[key].in_use) {
        key_lock_release();
        return NULL;
    }
    key_lock_release();

    int slot = get_thread_slot();
    if (slot < 0) {
        return NULL;
    }
    /* Reading values[slot] is safe without the lock once we have the slot,
     * because only this thread writes its own slot. */
    return tls_keys[key].values[slot];
}

int pthread_setspecific(pthread_key_t key, const void *value)
{
    if (key < 0 || key >= PTHREAD_KEYS_MAX) {
        return EINVAL;
    }
    key_lock_acquire();
    if (!tls_keys[key].in_use) {
        key_lock_release();
        return EINVAL;
    }
    key_lock_release();

    int slot = get_thread_slot();
    if (slot < 0) {
        return ENOMEM;
    }
    /* Only this thread writes its own slot -- no lock needed for the write
     * itself, but we need acquire/release to be visible to future getspecific
     * calls from the same thread. */
    __atomic_store_n(&tls_keys[key].values[slot], (void *)(uintptr_t)value,
                     __ATOMIC_RELEASE);
    return 0;
}

/*
 * Run TLS destructors for the calling thread on exit.
 * Called from pthread_exit() before the thread_exit syscall.
 */
static void run_tls_destructors(void)
{
    int slot = get_thread_slot();
    if (slot < 0) {
        return; /* No TLS values recorded for this thread. */
    }

    /* Up to PTHREAD_DESTRUCTOR_ITERATIONS rounds, as required by POSIX. */
    for (int round = 0; round < PTHREAD_DESTRUCTOR_ITERATIONS; round++) {
        int any = 0;
        key_lock_acquire();
        /* Snapshot: collect (value, destructor) pairs to call outside lock. */
        struct { void *val; void (*dtor)(void *); } pending[PTHREAD_KEYS_MAX];
        int n = 0;
        for (int k = 0; k < PTHREAD_KEYS_MAX; k++) {
            if (tls_keys[k].in_use && tls_keys[k].destructor &&
                tls_keys[k].values[slot]) {
                pending[n].val  = tls_keys[k].values[slot];
                pending[n].dtor = tls_keys[k].destructor;
                tls_keys[k].values[slot] = NULL; /* clear before calling dtor */
                n++;
                any = 1;
            }
        }
        key_lock_release();

        for (int i = 0; i < n; i++) {
            pending[i].dtor(pending[i].val);
        }
        if (!any) {
            break; /* No more non-NULL values; done. */
        }
    }
}

/* =========================================================================
 * Barrier
 * =========================================================================
 *
 * A cyclic barrier: all threads call pthread_barrier_wait(); the last one
 * to arrive releases all waiting threads and returns PTHREAD_BARRIER_SERIAL_THREAD.
 * All other callers return 0.
 *
 * State:
 *   barrier->count   -- total threads required (set at init, never changes)
 *   barrier->waiting -- number of threads currently blocked; futex address
 *   barrier->phase   -- generation counter; prevents released threads from
 *                       racing back into the next cycle and seeing a stale
 *                       futex value
 *
 * Protocol:
 *   1. Atomically increment waiting.
 *   2. If waiting < count: read current phase, then futex_wait on &waiting
 *      using the phase as the "expected changed" marker -- actually we wait
 *      on &phase until it advances (wakes happen via futex_wake on &phase).
 *   3. The last thread (waiting == count): reset waiting to 0, advance phase,
 *      broadcast-wake on &phase, return PTHREAD_BARRIER_SERIAL_THREAD.
 */
int pthread_barrier_init(pthread_barrier_t *barrier,
                         const pthread_barrierattr_t *attr,
                         unsigned int count)
{
    (void)attr;
    if (!barrier || count == 0) {
        return EINVAL;
    }
    int err = ensure_futex_ready();
    if (err) {
        return err;
    }
    barrier->count   = (int)count;
    barrier->waiting = 0;
    barrier->phase   = 0;
    return 0;
}

int pthread_barrier_destroy(pthread_barrier_t *barrier)
{
    /* Undefined if threads are still waiting. */
    (void)barrier;
    return 0;
}

int pthread_barrier_wait(pthread_barrier_t *barrier)
{
    int err = ensure_futex_ready();
    if (err) {
        return err;
    }

    /* Snapshot the phase before incrementing the waiter count so we can
     * detect the broadcast even if we are preempted between the increment
     * and the futex_wait call. */
    int my_phase = __atomic_load_n(&barrier->phase, __ATOMIC_ACQUIRE);

    int arrived = __atomic_add_fetch(&barrier->waiting, 1, __ATOMIC_ACQ_REL);
    if (arrived == barrier->count) {
        /* We are the last thread. Reset state and wake everyone else. */
        __atomic_store_n(&barrier->waiting, 0, __ATOMIC_RELEASE);
        __atomic_fetch_add(&barrier->phase, 1, __ATOMIC_RELEASE);
        futex_wake(&barrier->phase, INT32_MAX);
        return PTHREAD_BARRIER_SERIAL_THREAD;
    }

    /* Wait until the phase advances (last thread increments it and wakes us).
     * Retry on spurious wakeups by re-checking the phase. */
    while (__atomic_load_n(&barrier->phase, __ATOMIC_ACQUIRE) == my_phase) {
        futex_wait(&barrier->phase, my_phase, NULL);
    }
    return 0;
}

/* =========================================================================
 * Spinlock
 * =========================================================================
 *
 * A pure atomic CAS busy-wait lock.  No futex; no sleeping.  Suitable for
 * very short critical sections.  pshared is accepted for API compatibility
 * but ignored -- our memory model already supports cross-process shared
 * memory spinlocks because we use __ATOMIC_* builtins.
 *
 * State: 0 = unlocked, 1 = locked.
 *
 * pthread_spin_lock busy-waits using a test-and-test-and-set (TTAS) pattern:
 *   - First check the flag with a cheap load (avoids cache-line hammering).
 *   - Only attempt CAS when the flag appears to be 0.
 * On x86 a PAUSE hint (via __builtin_ia32_pause) reduces speculative
 * execution overhead in the spin loop.
 */
int pthread_spin_init(pthread_spinlock_t *lock, int pshared)
{
    (void)pshared;
    __atomic_store_n(lock, 0, __ATOMIC_RELEASE);
    return 0;
}

int pthread_spin_destroy(pthread_spinlock_t *lock)
{
    (void)lock;
    return 0;
}

int pthread_spin_lock(pthread_spinlock_t *lock)
{
    while (1) {
        /* TTAS fast path: cheap load before expensive CAS. */
        if (!__atomic_load_n(lock, __ATOMIC_RELAXED)) {
            int expected = 0;
            if (__atomic_compare_exchange_n(lock, &expected, 1, false,
                                            __ATOMIC_ACQUIRE, __ATOMIC_RELAXED)) {
                return 0;
            }
        }
        /* Hint to the CPU that we are in a spin loop.  On x86 this emits
         * PAUSE; on other architectures it compiles to nothing, which is
         * still correct. */
#if defined(__x86_64__)
        __asm__ volatile("pause" ::: "memory");
#elif defined(__aarch64__)
        __asm__ volatile("yield" ::: "memory");
#else
        __asm__ volatile("" ::: "memory"); /* compiler fence */
#endif
    }
}

int pthread_spin_trylock(pthread_spinlock_t *lock)
{
    int expected = 0;
    if (__atomic_compare_exchange_n(lock, &expected, 1, false,
                                    __ATOMIC_ACQUIRE, __ATOMIC_RELAXED)) {
        return 0;
    }
    return EBUSY;
}

int pthread_spin_unlock(pthread_spinlock_t *lock)
{
    __atomic_store_n(lock, 0, __ATOMIC_RELEASE);
    return 0;
}

/* =========================================================================
 * pthread_exit -- public API
 * =========================================================================
 *
 * Runs TLS destructors for all active keys belonging to this thread, then
 * calls pthread_exit_raw() which stores the return value in the TCB and
 * issues SYS_THREAD_EXIT.  The TCB cleanup (freeing stack, unregistering
 * from the TCB list) is handled by pthread_join() in the joining thread.
 *
 * Destructor ordering follows POSIX: up to PTHREAD_DESTRUCTOR_ITERATIONS
 * rounds; each round calls all non-NULL destructors and clears the value
 * before calling (so a destructor that sets a new value gets another round).
 */
void pthread_exit(void *retval)
{
    run_tls_destructors();
    pthread_exit_raw(retval);
}
