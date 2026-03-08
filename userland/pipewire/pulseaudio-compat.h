/*
 * VeridianOS -- pulseaudio-compat.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * PulseAudio compatibility API for VeridianOS.  Provides the subset of
 * the PulseAudio simple and async APIs that KDE Plasma, Qt Multimedia,
 * and common Linux applications expect, translating all calls to the
 * PipeWire daemon underneath.
 *
 * Covers:
 *   - Simple API (pa_simple) for basic playback/capture
 *   - Async context + stream API for volume control, sink enumeration
 *   - Sample format / channel map helpers
 */

#ifndef PULSEAUDIO_COMPAT_H
#define PULSEAUDIO_COMPAT_H

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Opaque types                                                              */
/* ========================================================================= */

struct pa_context;
struct pa_stream;
struct pa_mainloop;
struct pa_mainloop_api;
struct pa_simple;

/* ========================================================================= */
/* Sample format                                                             */
/* ========================================================================= */

enum pa_sample_format {
    PA_SAMPLE_U8        = 0,
    PA_SAMPLE_S16LE     = 3,
    PA_SAMPLE_S32LE     = 7,
    PA_SAMPLE_FLOAT32LE = 5,
    PA_SAMPLE_S24LE     = 11,
    PA_SAMPLE_INVALID   = -1
};

struct pa_sample_spec {
    enum pa_sample_format format;
    uint32_t              rate;
    uint8_t               channels;
};

/* ========================================================================= */
/* Channel map                                                               */
/* ========================================================================= */

#define PA_CHANNELS_MAX 8

enum pa_channel_position {
    PA_CHANNEL_POSITION_MONO           = 0,
    PA_CHANNEL_POSITION_FRONT_LEFT     = 1,
    PA_CHANNEL_POSITION_FRONT_RIGHT    = 2,
    PA_CHANNEL_POSITION_FRONT_CENTER   = 3,
    PA_CHANNEL_POSITION_LFE            = 4,
    PA_CHANNEL_POSITION_REAR_LEFT      = 5,
    PA_CHANNEL_POSITION_REAR_RIGHT     = 6,
    PA_CHANNEL_POSITION_SIDE_LEFT      = 7,
    PA_CHANNEL_POSITION_SIDE_RIGHT     = 8,
    PA_CHANNEL_POSITION_INVALID        = -1
};

struct pa_channel_map {
    uint8_t                  channels;
    enum pa_channel_position map[PA_CHANNELS_MAX];
};

struct pa_channel_map *pa_channel_map_init_auto(struct pa_channel_map *map,
                                                unsigned channels,
                                                int def);

/* ========================================================================= */
/* Buffer attributes (for latency control)                                   */
/* ========================================================================= */

struct pa_buffer_attr {
    uint32_t maxlength;   /**< Maximum buffer length in bytes */
    uint32_t tlength;     /**< Target buffer length (playback) */
    uint32_t prebuf;      /**< Pre-buffering in bytes */
    uint32_t minreq;      /**< Minimum request size */
    uint32_t fragsize;    /**< Fragment size (capture) */
};

/* ========================================================================= */
/* Context state                                                             */
/* ========================================================================= */

enum pa_context_state {
    PA_CONTEXT_UNCONNECTED  = 0,
    PA_CONTEXT_CONNECTING   = 1,
    PA_CONTEXT_AUTHORIZING  = 2,
    PA_CONTEXT_SETTING_NAME = 3,
    PA_CONTEXT_READY        = 4,
    PA_CONTEXT_FAILED       = 5,
    PA_CONTEXT_TERMINATED   = 6
};

/* ========================================================================= */
/* Stream state                                                              */
/* ========================================================================= */

enum pa_stream_state {
    PA_STREAM_UNCONNECTED  = 0,
    PA_STREAM_CREATING     = 1,
    PA_STREAM_READY        = 2,
    PA_STREAM_FAILED       = 3,
    PA_STREAM_TERMINATED   = 4
};

/* ========================================================================= */
/* Stream direction                                                          */
/* ========================================================================= */

enum pa_stream_direction {
    PA_STREAM_NODIRECTION  = 0,
    PA_STREAM_PLAYBACK     = 1,
    PA_STREAM_RECORD       = 2,
    PA_STREAM_UPLOAD        = 3
};

/* ========================================================================= */
/* Simple API                                                                */
/* ========================================================================= */

/**
 * Create a new playback or capture connection.
 *
 * @param server  Server name (NULL for default -- ignored on VeridianOS).
 * @param name    Application name.
 * @param dir     Stream direction (PA_STREAM_PLAYBACK or PA_STREAM_RECORD).
 * @param dev     Sink/source name (NULL for default).
 * @param stream_name  Human-readable stream name.
 * @param ss      Sample specification.
 * @param map     Channel map (NULL for default).
 * @param attr    Buffer attributes (NULL for defaults).
 * @param error   Set to error code on failure (may be NULL).
 * @return Handle on success, NULL on failure.
 */
struct pa_simple *pa_simple_new(const char *server,
                                const char *name,
                                enum pa_stream_direction dir,
                                const char *dev,
                                const char *stream_name,
                                const struct pa_sample_spec *ss,
                                const struct pa_channel_map *map,
                                const struct pa_buffer_attr *attr,
                                int *error);

/** Write audio data (blocking). Returns 0 on success, negative on error. */
int pa_simple_write(struct pa_simple *s,
                    const void *data,
                    size_t bytes,
                    int *error);

/** Read audio data (blocking). Returns 0 on success, negative on error. */
int pa_simple_read(struct pa_simple *s,
                   void *data,
                   size_t bytes,
                   int *error);

/** Drain the playback buffer. Returns 0 on success. */
int pa_simple_drain(struct pa_simple *s, int *error);

/** Free a simple connection. */
void pa_simple_free(struct pa_simple *s);

/** Get the playback/recording latency in microseconds. */
uint64_t pa_simple_get_latency(struct pa_simple *s, int *error);

/* ========================================================================= */
/* Async API (subset)                                                        */
/* ========================================================================= */

typedef void (*pa_context_notify_cb_t)(struct pa_context *c, void *userdata);
typedef void (*pa_stream_notify_cb_t)(struct pa_stream *s, void *userdata);

/** Main loop */
struct pa_mainloop     *pa_mainloop_new(void);
struct pa_mainloop_api *pa_mainloop_get_api(struct pa_mainloop *m);
int                     pa_mainloop_run(struct pa_mainloop *m, int *retval);
void                    pa_mainloop_free(struct pa_mainloop *m);

/** Context */
struct pa_context *pa_context_new(struct pa_mainloop_api *api,
                                  const char *name);
int                pa_context_connect(struct pa_context *c,
                                      const char *server,
                                      int flags,
                                      void *spawn_api);
void               pa_context_disconnect(struct pa_context *c);
enum pa_context_state pa_context_get_state(const struct pa_context *c);
void               pa_context_set_state_callback(struct pa_context *c,
                                                  pa_context_notify_cb_t cb,
                                                  void *userdata);
void               pa_context_unref(struct pa_context *c);

/** Stream */
struct pa_stream *pa_stream_new(struct pa_context *c,
                                const char *name,
                                const struct pa_sample_spec *ss,
                                const struct pa_channel_map *map);

int  pa_stream_connect_playback(struct pa_stream *s,
                                const char *dev,
                                const struct pa_buffer_attr *attr,
                                int flags,
                                void *volume,
                                struct pa_stream *sync_stream);

int  pa_stream_connect_record(struct pa_stream *s,
                              const char *dev,
                              const struct pa_buffer_attr *attr,
                              int flags);

int  pa_stream_write(struct pa_stream *s,
                     const void *data,
                     size_t nbytes,
                     void (*free_cb)(void *),
                     int64_t offset,
                     int seek);

int  pa_stream_cork(struct pa_stream *s, int b, void *cb, void *userdata);

enum pa_stream_state pa_stream_get_state(const struct pa_stream *s);

void pa_stream_set_state_callback(struct pa_stream *s,
                                  pa_stream_notify_cb_t cb,
                                  void *userdata);

void pa_stream_unref(struct pa_stream *s);

/* ========================================================================= */
/* Utility                                                                   */
/* ========================================================================= */

/** Return a human-readable error string for an error code. */
const char *pa_strerror(int error);

/** PA error codes */
#define PA_OK                0
#define PA_ERR_ACCESS        1
#define PA_ERR_COMMAND       2
#define PA_ERR_INVALID       3
#define PA_ERR_EXIST         4
#define PA_ERR_NOENTITY      5
#define PA_ERR_CONNECTIONREFUSED 6
#define PA_ERR_PROTOCOL      7
#define PA_ERR_TIMEOUT       8
#define PA_ERR_NOTSUPPORTED  10
#define PA_ERR_UNKNOWN       11
#define PA_ERR_NODATA        12
#define PA_ERR_IO            14

/** Volume constants (linear 16-bit scale, 0-65536) */
#define PA_VOLUME_MUTED   ((uint32_t)0x00000000U)
#define PA_VOLUME_NORM    ((uint32_t)0x00010000U)
#define PA_VOLUME_MAX     ((uint32_t)0x0001FFFFU)

#ifdef __cplusplus
}  /* extern "C" */
#endif

#endif /* PULSEAUDIO_COMPAT_H */
