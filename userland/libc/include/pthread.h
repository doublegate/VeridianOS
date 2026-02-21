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

#ifdef __cplusplus
}
#endif

#endif /* VERIDIAN_PTHREAD_H */
