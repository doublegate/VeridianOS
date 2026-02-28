/*
 * VeridianOS libc -- pthread.h
 *
 * Minimal, strictly functional pthread subset backed by SYS_THREAD_CLONE
 * and SYS_FUTEX. Designed to match POSIX semantics closely enough for
 * libc, GCC, and build tooling.
 */

#ifndef VERIDIAN_PTHREAD_H
#define VERIDIAN_PTHREAD_H

#include <stddef.h>
#include <stdint.h>
#include <time.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef unsigned long pthread_t;

typedef struct {
    size_t stack_size;
    int detach_state; /* 0=joinable, 1=detached */
} pthread_attr_t;

typedef struct {
    int state; /* 0 unlocked, 1 locked */
} pthread_mutex_t;

typedef struct {
    int state; /* 0 unlocked, 1 locked with waiter(s) */
} pthread_mutexattr_t;

typedef struct {
    int futex;
} pthread_cond_t;

typedef struct {
    int dummy;
} pthread_condattr_t;

typedef struct {
    int done;
    void *retval;
} pthread_once_t;

/*
 * pthread_rwlock_t -- readers-writer lock
 *
 * readers: number of active readers (positive means read-locked)
 * writer:  1 when a writer holds the lock, 0 otherwise
 * pending_writers: count of writers blocked in futex_wait; used so
 *   new readers yield to waiting writers (writer preference).
 *
 * State word layout (packed into a single int for futex):
 *   We keep readers and writer counts separately for clarity, and use
 *   futex_wait on &writer when readers block for a writer, and on &readers
 *   when a writer blocks for readers to drain.
 */
typedef struct {
    int readers;         /* active reader count; futex addr for writer blocking */
    int writer;          /* 1 if a writer holds the lock; futex addr for reader blocking */
    int pending_writers; /* writers waiting; biases reader acquisition */
} pthread_rwlock_t;

typedef struct {
    int dummy;
} pthread_rwlockattr_t;

#define PTHREAD_RWLOCK_INITIALIZER { .readers = 0, .writer = 0, .pending_writers = 0 }

/*
 * TLS keys -- up to PTHREAD_KEYS_MAX per process.
 *
 * pthread_key_t is a plain int index into the global key table.
 * Each thread stores its own per-key value in a fixed-size array carried
 * in its TCB (or, for the main thread and threads that have not yet set any
 * key, in a globally indexed table keyed by TID via a flat lookup).
 *
 * Implementation note: rather than extending the TCB (which would require
 * changing the clone setup), we use a per-process global table of
 * (destructor, values[MAX_THREADS]) entries allocated lazily.  For
 * self-hosting workloads the number of threads is small so this is fine.
 */
#define PTHREAD_KEYS_MAX 128

typedef int pthread_key_t;

/*
 * pthread_barrier_t -- cyclic barrier.
 *
 * count:    total threads that must call pthread_barrier_wait before any
 *           proceeds.
 * waiting:  number of threads currently blocked in the barrier; futex addr.
 * phase:    incremented each time the barrier is released (cyclic support).
 */
typedef struct {
    int count;
    int waiting; /* futex wait address */
    int phase;   /* generation counter to handle spurious wakeups */
} pthread_barrier_t;

typedef struct {
    int dummy;
} pthread_barrierattr_t;

#define PTHREAD_BARRIER_SERIAL_THREAD (-1)

/*
 * pthread_spinlock_t -- simple atomic spinlock.
 *
 * 0 = unlocked, 1 = locked.  No futex: busy-waits.
 */
typedef int pthread_spinlock_t;

#define PTHREAD_MUTEX_INITIALIZER { .state = 0 }
#define PTHREAD_COND_INITIALIZER  { .futex = 0 }
#define PTHREAD_ONCE_INIT         { .done = 0, .retval = NULL }

int pthread_create(pthread_t *thread,
                   const pthread_attr_t *attr,
                   void *(*start_routine)(void *),
                   void *arg);
int pthread_join(pthread_t thread, void **retval);
int pthread_detach(pthread_t thread);
void pthread_exit(void *retval) __attribute__((noreturn));
pthread_t pthread_self(void);

int pthread_mutex_init(pthread_mutex_t *mutex, const pthread_mutexattr_t *attr);
int pthread_mutex_destroy(pthread_mutex_t *mutex);
int pthread_mutex_lock(pthread_mutex_t *mutex);
int pthread_mutex_trylock(pthread_mutex_t *mutex);
int pthread_mutex_unlock(pthread_mutex_t *mutex);

int pthread_cond_init(pthread_cond_t *cond, const pthread_condattr_t *attr);
int pthread_cond_destroy(pthread_cond_t *cond);
int pthread_cond_wait(pthread_cond_t *cond, pthread_mutex_t *mutex);
int pthread_cond_timedwait(pthread_cond_t *cond, pthread_mutex_t *mutex,
                           const struct timespec *abstime);
int pthread_cond_signal(pthread_cond_t *cond);
int pthread_cond_broadcast(pthread_cond_t *cond);

int pthread_once(pthread_once_t *once_control, void (*init_routine)(void));
int pthread_setcancelstate(int state, int *oldstate); /* stub: no cancellation */
int pthread_yield(void);

/* Read-write locks */
int pthread_rwlock_init(pthread_rwlock_t *rwlock, const pthread_rwlockattr_t *attr);
int pthread_rwlock_destroy(pthread_rwlock_t *rwlock);
int pthread_rwlock_rdlock(pthread_rwlock_t *rwlock);
int pthread_rwlock_tryrdlock(pthread_rwlock_t *rwlock);
int pthread_rwlock_wrlock(pthread_rwlock_t *rwlock);
int pthread_rwlock_trywrlock(pthread_rwlock_t *rwlock);
int pthread_rwlock_unlock(pthread_rwlock_t *rwlock);

/* TLS keys */
int pthread_key_create(pthread_key_t *key, void (*destructor)(void *));
int pthread_key_delete(pthread_key_t key);
void *pthread_getspecific(pthread_key_t key);
int pthread_setspecific(pthread_key_t key, const void *value);

/* Barriers */
int pthread_barrier_init(pthread_barrier_t *barrier,
                         const pthread_barrierattr_t *attr,
                         unsigned int count);
int pthread_barrier_destroy(pthread_barrier_t *barrier);
int pthread_barrier_wait(pthread_barrier_t *barrier);

/* Spinlocks */
int pthread_spin_init(pthread_spinlock_t *lock, int pshared);
int pthread_spin_destroy(pthread_spinlock_t *lock);
int pthread_spin_lock(pthread_spinlock_t *lock);
int pthread_spin_trylock(pthread_spinlock_t *lock);
int pthread_spin_unlock(pthread_spinlock_t *lock);

#ifdef __cplusplus
}
#endif

#endif /* VERIDIAN_PTHREAD_H */
