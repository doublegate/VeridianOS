/*
 * VeridianOS -- pulseaudio-compat.cpp
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * PulseAudio compatibility layer for VeridianOS.  Translates PulseAudio
 * simple and async API calls to the PipeWire daemon implementation,
 * allowing unmodified PA-linked applications and KDE Plasma audio
 * controls to function.
 *
 * Format mapping: PA_SAMPLE_* -> SPA_AUDIO_FORMAT_*.
 * Volume mapping: PA 0-65536 linear scale <-> PipeWire/ALSA 0-100.
 */

#include "pulseaudio-compat.h"
#include "pipewire-veridian.h"

#include <stdlib.h>
#include <string.h>
#include <stdio.h>
#include <errno.h>
#include <stdarg.h>

/* ========================================================================= */
/* Internal types                                                            */
/* ========================================================================= */

struct pa_simple {
    struct pw_stream          *stream;
    enum pa_stream_direction   direction;
    struct pa_sample_spec      spec;
    struct pa_channel_map      channel_map;
    struct pa_buffer_attr      attr;
    int                        connected;
    uint32_t                   bytes_per_sample;
    uint32_t                   frame_size;
};

#define PA_MAX_CONTEXTS 8
#define PA_MAX_STREAMS  16

struct pa_context {
    int                     ref_count;
    enum pa_context_state   state;
    char                    name[64];
    struct pw_context      *pw_ctx;
    struct pw_core         *pw_core;
    pa_context_notify_cb_t  state_cb;
    void                   *state_cb_data;
};

struct pa_stream {
    int                     ref_count;
    enum pa_stream_state    state;
    char                    name[64];
    struct pa_sample_spec   spec;
    struct pa_channel_map   channel_map;
    struct pa_context      *context;
    struct pw_stream       *pw_stream;
    pa_stream_notify_cb_t   state_cb;
    void                   *state_cb_data;
};

struct pa_mainloop {
    struct pw_main_loop *pw_loop;
};

struct pa_mainloop_api {
    struct pa_mainloop *mainloop;
};

/* ========================================================================= */
/* Format translation                                                        */
/* ========================================================================= */

static enum spa_audio_format pa_format_to_spa(enum pa_sample_format fmt) {
    switch (fmt) {
        case PA_SAMPLE_U8:        return SPA_AUDIO_FORMAT_U8;
        case PA_SAMPLE_S16LE:     return SPA_AUDIO_FORMAT_S16_LE;
        case PA_SAMPLE_S32LE:     return SPA_AUDIO_FORMAT_S32_LE;
        case PA_SAMPLE_FLOAT32LE: return SPA_AUDIO_FORMAT_F32_LE;
        case PA_SAMPLE_S24LE:     return SPA_AUDIO_FORMAT_S24_LE;
        default:                  return SPA_AUDIO_FORMAT_S16_LE;
    }
}

static uint32_t pa_format_bytes(enum pa_sample_format fmt) {
    switch (fmt) {
        case PA_SAMPLE_U8:        return 1;
        case PA_SAMPLE_S16LE:     return 2;
        case PA_SAMPLE_S24LE:     return 3;
        case PA_SAMPLE_S32LE:     return 4;
        case PA_SAMPLE_FLOAT32LE: return 4;
        default:                  return 2;
    }
}

/* ========================================================================= */
/* Channel map                                                               */
/* ========================================================================= */

struct pa_channel_map *pa_channel_map_init_auto(struct pa_channel_map *map,
                                                unsigned channels,
                                                int def) {
    (void)def;
    if (!map) return NULL;

    memset(map, 0, sizeof(*map));
    map->channels = (uint8_t)(channels > PA_CHANNELS_MAX
                              ? PA_CHANNELS_MAX : channels);

    switch (map->channels) {
        case 1:
            map->map[0] = PA_CHANNEL_POSITION_MONO;
            break;
        case 2:
            map->map[0] = PA_CHANNEL_POSITION_FRONT_LEFT;
            map->map[1] = PA_CHANNEL_POSITION_FRONT_RIGHT;
            break;
        case 6:
            map->map[0] = PA_CHANNEL_POSITION_FRONT_LEFT;
            map->map[1] = PA_CHANNEL_POSITION_FRONT_RIGHT;
            map->map[2] = PA_CHANNEL_POSITION_FRONT_CENTER;
            map->map[3] = PA_CHANNEL_POSITION_LFE;
            map->map[4] = PA_CHANNEL_POSITION_REAR_LEFT;
            map->map[5] = PA_CHANNEL_POSITION_REAR_RIGHT;
            break;
        case 8:
            map->map[0] = PA_CHANNEL_POSITION_FRONT_LEFT;
            map->map[1] = PA_CHANNEL_POSITION_FRONT_RIGHT;
            map->map[2] = PA_CHANNEL_POSITION_FRONT_CENTER;
            map->map[3] = PA_CHANNEL_POSITION_LFE;
            map->map[4] = PA_CHANNEL_POSITION_REAR_LEFT;
            map->map[5] = PA_CHANNEL_POSITION_REAR_RIGHT;
            map->map[6] = PA_CHANNEL_POSITION_SIDE_LEFT;
            map->map[7] = PA_CHANNEL_POSITION_SIDE_RIGHT;
            break;
        default:
            for (unsigned i = 0; i < map->channels; i++) {
                map->map[i] = (enum pa_channel_position)(
                    PA_CHANNEL_POSITION_FRONT_LEFT + (int)i);
            }
            break;
    }
    return map;
}

/* ========================================================================= */
/* Error strings                                                             */
/* ========================================================================= */

static const char *pa_error_strings[] = {
    "OK",                       /* 0 */
    "Access denied",            /* 1 */
    "Bad command",              /* 2 */
    "Invalid argument",         /* 3 */
    "Entity exists",            /* 4 */
    "No such entity",           /* 5 */
    "Connection refused",       /* 6 */
    "Protocol error",           /* 7 */
    "Timeout",                  /* 8 */
    "Unknown error (9)",        /* 9 */
    "Not supported",            /* 10 */
    "Unknown error",            /* 11 */
    "No data",                  /* 12 */
    "Unknown error (13)",       /* 13 */
    "I/O error",                /* 14 */
};

#define PA_ERROR_COUNT (sizeof(pa_error_strings) / sizeof(pa_error_strings[0]))

const char *pa_strerror(int error) {
    if (error < 0) error = -error;
    if ((unsigned)error >= PA_ERROR_COUNT) return "Unknown error";
    return pa_error_strings[error];
}

/* ========================================================================= */
/* Simple API                                                                */
/* ========================================================================= */

struct pa_simple *pa_simple_new(const char *server,
                                const char *name,
                                enum pa_stream_direction dir,
                                const char *dev,
                                const char *stream_name,
                                const struct pa_sample_spec *ss,
                                const struct pa_channel_map *map,
                                const struct pa_buffer_attr *attr,
                                int *error) {
    (void)server;
    (void)dev;

    if (!ss) {
        if (error) *error = PA_ERR_INVALID;
        return NULL;
    }

    /* Ensure PipeWire is initialised */
    pw_init(NULL, NULL);

    struct pa_simple *s = (struct pa_simple *)calloc(1, sizeof(struct pa_simple));
    if (!s) {
        if (error) *error = PA_ERR_UNKNOWN;
        return NULL;
    }

    s->direction = dir;
    s->spec = *ss;
    s->bytes_per_sample = pa_format_bytes(ss->format);
    s->frame_size = s->bytes_per_sample * ss->channels;

    /* Set up channel map */
    if (map) {
        s->channel_map = *map;
    } else {
        pa_channel_map_init_auto(&s->channel_map, ss->channels, 0);
    }

    /* Buffer attributes */
    if (attr) {
        s->attr = *attr;
    } else {
        /* Default: ~50ms buffer at the given rate */
        uint32_t frames_50ms = (ss->rate * 50) / 1000;
        uint32_t buf_bytes = frames_50ms * s->frame_size;
        s->attr.maxlength = buf_bytes * 2;
        s->attr.tlength   = buf_bytes;
        s->attr.prebuf    = buf_bytes;
        s->attr.minreq    = s->frame_size * 256;
        s->attr.fragsize  = buf_bytes;
    }

    /* Create PipeWire context and stream */
    struct pw_context *ctx = pw_context_new(NULL, NULL, 0);
    struct pw_core *core = pw_context_connect(ctx, NULL, 0);

    const char *sname = stream_name ? stream_name : (name ? name : "pa_simple");
    s->stream = pw_stream_new(core, sname, NULL);
    if (!s->stream) {
        free(s);
        pw_context_destroy(ctx);
        if (error) *error = PA_ERR_UNKNOWN;
        return NULL;
    }

    /* Build format params */
    struct spa_audio_info_raw raw;
    memset(&raw, 0, sizeof(raw));
    raw.format   = pa_format_to_spa(ss->format);
    raw.rate     = ss->rate;
    raw.channels = ss->channels;

    /* Map PA channel positions to SPA */
    for (uint8_t i = 0; i < ss->channels && i < SPA_AUDIO_MAX_CHANNELS; i++) {
        raw.position[i] = (enum spa_audio_channel)(
            SPA_AUDIO_CHANNEL_FL + (int)s->channel_map.map[i]);
    }

    /* Connect the stream */
    enum pw_direction pw_dir = (dir == PA_STREAM_PLAYBACK)
                               ? PW_DIRECTION_OUTPUT
                               : PW_DIRECTION_INPUT;

    const void *params[] = { &raw };
    int ret = pw_stream_connect(s->stream, pw_dir, PW_ID_ANY,
                                PW_STREAM_FLAG_AUTOCONNECT,
                                params, 1);
    if (ret < 0) {
        pw_stream_destroy(s->stream);
        free(s);
        pw_context_destroy(ctx);
        if (error) *error = PA_ERR_CONNECTIONREFUSED;
        return NULL;
    }

    s->connected = 1;

    fprintf(stderr, "[pa-compat] pa_simple_new: '%s' (%s, fmt=%d, rate=%u, "
            "ch=%u)\n",
            sname,
            dir == PA_STREAM_PLAYBACK ? "playback" : "record",
            ss->format, ss->rate, ss->channels);

    if (error) *error = PA_OK;
    return s;
}

int pa_simple_write(struct pa_simple *s,
                    const void *data,
                    size_t bytes,
                    int *error) {
    if (!s || !data || bytes == 0) {
        if (error) *error = PA_ERR_INVALID;
        return -1;
    }

    if (s->direction != PA_STREAM_PLAYBACK) {
        if (error) *error = PA_ERR_NOTSUPPORTED;
        return -1;
    }

    /* Dequeue a buffer, fill it, queue it back */
    size_t remaining = bytes;
    const uint8_t *src = (const uint8_t *)data;

    while (remaining > 0) {
        struct pw_buffer *buf = pw_stream_dequeue_buffer(s->stream);
        if (!buf) {
            /* No buffers available -- in a real impl we'd block */
            if (error) *error = PA_ERR_IO;
            return -1;
        }

        struct spa_data *d = &buf->buffer->datas[0];
        uint32_t chunk = (remaining < d->maxsize)
                         ? (uint32_t)remaining : d->maxsize;

        memcpy(d->data, src, chunk);
        d->chunk_offset = 0;
        d->chunk_size   = chunk;

        pw_stream_queue_buffer(s->stream, buf);

        src       += chunk;
        remaining -= chunk;
    }

    if (error) *error = PA_OK;
    return 0;
}

int pa_simple_read(struct pa_simple *s,
                   void *data,
                   size_t bytes,
                   int *error) {
    if (!s || !data || bytes == 0) {
        if (error) *error = PA_ERR_INVALID;
        return -1;
    }

    if (s->direction != PA_STREAM_RECORD) {
        if (error) *error = PA_ERR_NOTSUPPORTED;
        return -1;
    }

    struct pw_buffer *buf = pw_stream_dequeue_buffer(s->stream);
    if (!buf) {
        if (error) *error = PA_ERR_NODATA;
        return -1;
    }

    struct spa_data *d = &buf->buffer->datas[0];
    uint32_t avail = d->chunk_size;
    uint32_t copy = (bytes < avail) ? (uint32_t)bytes : avail;

    memcpy(data, (uint8_t *)d->data + d->chunk_offset, copy);
    pw_stream_queue_buffer(s->stream, buf);

    if (error) *error = PA_OK;
    return 0;
}

int pa_simple_drain(struct pa_simple *s, int *error) {
    if (!s) {
        if (error) *error = PA_ERR_INVALID;
        return -1;
    }
    /* Drain is a no-op for now; buffers are written synchronously */
    if (error) *error = PA_OK;
    return 0;
}

void pa_simple_free(struct pa_simple *s) {
    if (!s) return;
    if (s->stream) {
        pw_stream_destroy(s->stream);
    }
    free(s);
}

uint64_t pa_simple_get_latency(struct pa_simple *s, int *error) {
    if (!s) {
        if (error) *error = PA_ERR_INVALID;
        return 0;
    }

    /* Estimate latency from buffer attributes:
     * latency_us = tlength * 1_000_000 / (rate * frame_size) */
    uint32_t denominator = s->spec.rate * s->frame_size;
    if (denominator == 0) {
        if (error) *error = PA_OK;
        return 0;
    }

    uint64_t latency_us = ((uint64_t)s->attr.tlength * 1000000ULL) / denominator;
    if (error) *error = PA_OK;
    return latency_us;
}

/* ========================================================================= */
/* Async API -- Mainloop                                                     */
/* ========================================================================= */

static struct pa_mainloop_api g_pa_api;

struct pa_mainloop *pa_mainloop_new(void) {
    struct pa_mainloop *m =
        (struct pa_mainloop *)calloc(1, sizeof(struct pa_mainloop));
    if (!m) return NULL;

    pw_init(NULL, NULL);
    m->pw_loop = pw_main_loop_new(NULL);
    return m;
}

struct pa_mainloop_api *pa_mainloop_get_api(struct pa_mainloop *m) {
    if (!m) return NULL;
    g_pa_api.mainloop = m;
    return &g_pa_api;
}

int pa_mainloop_run(struct pa_mainloop *m, int *retval) {
    if (!m || !m->pw_loop) {
        if (retval) *retval = -1;
        return -1;
    }
    int ret = pw_main_loop_run(m->pw_loop);
    if (retval) *retval = ret;
    return ret;
}

void pa_mainloop_free(struct pa_mainloop *m) {
    if (!m) return;
    if (m->pw_loop) pw_main_loop_destroy(m->pw_loop);
    free(m);
}

/* ========================================================================= */
/* Async API -- Context                                                      */
/* ========================================================================= */

struct pa_context *pa_context_new(struct pa_mainloop_api *api,
                                  const char *name) {
    (void)api;

    struct pa_context *c =
        (struct pa_context *)calloc(1, sizeof(struct pa_context));
    if (!c) return NULL;

    c->ref_count = 1;
    c->state = PA_CONTEXT_UNCONNECTED;
    if (name) strncpy(c->name, name, sizeof(c->name) - 1);

    return c;
}

int pa_context_connect(struct pa_context *c,
                       const char *server,
                       int flags,
                       void *spawn_api) {
    (void)server;
    (void)flags;
    (void)spawn_api;

    if (!c) return -PA_ERR_INVALID;

    pw_init(NULL, NULL);

    c->state = PA_CONTEXT_CONNECTING;
    if (c->state_cb) c->state_cb(c, c->state_cb_data);

    /* Create PipeWire context and connect */
    c->pw_ctx = pw_context_new(NULL, NULL, 0);
    if (!c->pw_ctx) {
        c->state = PA_CONTEXT_FAILED;
        if (c->state_cb) c->state_cb(c, c->state_cb_data);
        return -PA_ERR_CONNECTIONREFUSED;
    }

    c->pw_core = pw_context_connect(c->pw_ctx, NULL, 0);
    if (!c->pw_core) {
        pw_context_destroy(c->pw_ctx);
        c->pw_ctx = NULL;
        c->state = PA_CONTEXT_FAILED;
        if (c->state_cb) c->state_cb(c, c->state_cb_data);
        return -PA_ERR_CONNECTIONREFUSED;
    }

    c->state = PA_CONTEXT_READY;
    if (c->state_cb) c->state_cb(c, c->state_cb_data);

    fprintf(stderr, "[pa-compat] Context '%s' connected\n", c->name);
    return 0;
}

void pa_context_disconnect(struct pa_context *c) {
    if (!c) return;

    if (c->pw_ctx) {
        pw_context_destroy(c->pw_ctx);
        c->pw_ctx = NULL;
        c->pw_core = NULL;
    }

    c->state = PA_CONTEXT_TERMINATED;
    if (c->state_cb) c->state_cb(c, c->state_cb_data);
}

enum pa_context_state pa_context_get_state(const struct pa_context *c) {
    if (!c) return PA_CONTEXT_FAILED;
    return c->state;
}

void pa_context_set_state_callback(struct pa_context *c,
                                   pa_context_notify_cb_t cb,
                                   void *userdata) {
    if (!c) return;
    c->state_cb = cb;
    c->state_cb_data = userdata;
}

void pa_context_unref(struct pa_context *c) {
    if (!c) return;
    c->ref_count--;
    if (c->ref_count <= 0) {
        pa_context_disconnect(c);
        free(c);
    }
}

/* ========================================================================= */
/* Async API -- Stream                                                       */
/* ========================================================================= */

struct pa_stream *pa_stream_new(struct pa_context *c,
                                const char *name,
                                const struct pa_sample_spec *ss,
                                const struct pa_channel_map *map) {
    if (!c || !ss) return NULL;
    if (c->state != PA_CONTEXT_READY) return NULL;

    struct pa_stream *s =
        (struct pa_stream *)calloc(1, sizeof(struct pa_stream));
    if (!s) return NULL;

    s->ref_count = 1;
    s->state = PA_STREAM_UNCONNECTED;
    s->context = c;
    s->spec = *ss;
    if (name) strncpy(s->name, name, sizeof(s->name) - 1);

    if (map) {
        s->channel_map = *map;
    } else {
        pa_channel_map_init_auto(&s->channel_map, ss->channels, 0);
    }

    /* Create underlying PipeWire stream */
    s->pw_stream = pw_stream_new(c->pw_core, name, NULL);
    if (!s->pw_stream) {
        free(s);
        return NULL;
    }

    return s;
}

int pa_stream_connect_playback(struct pa_stream *s,
                               const char *dev,
                               const struct pa_buffer_attr *attr,
                               int flags,
                               void *volume,
                               struct pa_stream *sync_stream) {
    (void)dev;
    (void)attr;
    (void)flags;
    (void)volume;
    (void)sync_stream;

    if (!s || !s->pw_stream) return -PA_ERR_INVALID;

    struct spa_audio_info_raw raw;
    memset(&raw, 0, sizeof(raw));
    raw.format   = pa_format_to_spa(s->spec.format);
    raw.rate     = s->spec.rate;
    raw.channels = s->spec.channels;

    const void *params[] = { &raw };
    int ret = pw_stream_connect(s->pw_stream, PW_DIRECTION_OUTPUT,
                                PW_ID_ANY, PW_STREAM_FLAG_AUTOCONNECT,
                                params, 1);
    if (ret < 0) {
        s->state = PA_STREAM_FAILED;
        if (s->state_cb) s->state_cb(s, s->state_cb_data);
        return -PA_ERR_IO;
    }

    s->state = PA_STREAM_READY;
    if (s->state_cb) s->state_cb(s, s->state_cb_data);
    return 0;
}

int pa_stream_connect_record(struct pa_stream *s,
                             const char *dev,
                             const struct pa_buffer_attr *attr,
                             int flags) {
    (void)dev;
    (void)attr;
    (void)flags;

    if (!s || !s->pw_stream) return -PA_ERR_INVALID;

    struct spa_audio_info_raw raw;
    memset(&raw, 0, sizeof(raw));
    raw.format   = pa_format_to_spa(s->spec.format);
    raw.rate     = s->spec.rate;
    raw.channels = s->spec.channels;

    const void *params[] = { &raw };
    int ret = pw_stream_connect(s->pw_stream, PW_DIRECTION_INPUT,
                                PW_ID_ANY, PW_STREAM_FLAG_AUTOCONNECT,
                                params, 1);
    if (ret < 0) {
        s->state = PA_STREAM_FAILED;
        if (s->state_cb) s->state_cb(s, s->state_cb_data);
        return -PA_ERR_IO;
    }

    s->state = PA_STREAM_READY;
    if (s->state_cb) s->state_cb(s, s->state_cb_data);
    return 0;
}

int pa_stream_write(struct pa_stream *s,
                    const void *data,
                    size_t nbytes,
                    void (*free_cb)(void *),
                    int64_t offset,
                    int seek) {
    (void)free_cb;
    (void)offset;
    (void)seek;

    if (!s || !s->pw_stream || !data || nbytes == 0) return -PA_ERR_INVALID;

    struct pw_buffer *buf = pw_stream_dequeue_buffer(s->pw_stream);
    if (!buf) return -PA_ERR_IO;

    struct spa_data *d = &buf->buffer->datas[0];
    uint32_t chunk = (nbytes < d->maxsize) ? (uint32_t)nbytes : d->maxsize;

    memcpy(d->data, data, chunk);
    d->chunk_offset = 0;
    d->chunk_size   = chunk;

    pw_stream_queue_buffer(s->pw_stream, buf);
    return 0;
}

int pa_stream_cork(struct pa_stream *s, int b, void *cb, void *userdata) {
    (void)cb;
    (void)userdata;

    if (!s) return -PA_ERR_INVALID;

    /* Cork (pause) or uncork (resume) -- currently a no-op */
    fprintf(stderr, "[pa-compat] Stream '%s' %s\n",
            s->name, b ? "corked" : "uncorked");
    return 0;
}

enum pa_stream_state pa_stream_get_state(const struct pa_stream *s) {
    if (!s) return PA_STREAM_FAILED;
    return s->state;
}

void pa_stream_set_state_callback(struct pa_stream *s,
                                  pa_stream_notify_cb_t cb,
                                  void *userdata) {
    if (!s) return;
    s->state_cb = cb;
    s->state_cb_data = userdata;
}

void pa_stream_unref(struct pa_stream *s) {
    if (!s) return;
    s->ref_count--;
    if (s->ref_count <= 0) {
        if (s->pw_stream) pw_stream_destroy(s->pw_stream);
        free(s);
    }
}
