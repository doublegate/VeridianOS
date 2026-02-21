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
 * Limitations:
 *   - No thread cancellation (pthread_cancel is not implemented).
 *   - No robust mutexes or priority inheritance.
 *   - No thread-local storage keys (pthread_key_create et al. are stubs).
 *   - Condition variable timedwait does not handle clock selection.
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
    tcb->retval = rv;
    veridian_syscall1(SYS_THREAD_EXIT, (long)rv);
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

void pthread_exit(void *retval)
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
