/*
 * VeridianOS -- pipewire-veridian.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * PipeWire audio daemon API shim for VeridianOS.  Provides the core
 * PipeWire types, enumerations, and function prototypes required by
 * applications and toolkits (Qt 6 multimedia, KDE Plasma audio applet)
 * to interact with the audio graph.
 *
 * The implementation bridges PipeWire semantics to the kernel ALSA
 * subsystem via pw-alsa-bridge, exposing a node/port/link graph model
 * on top of kernel PCM devices.
 */

#ifndef PIPEWIRE_VERIDIAN_H
#define PIPEWIRE_VERIDIAN_H

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Forward declarations                                                      */
/* ========================================================================= */

struct pw_main_loop;
struct pw_context;
struct pw_core;
struct pw_stream;
struct pw_node;
struct pw_port;
struct pw_link;
struct pw_properties;

/* ========================================================================= */
/* Enumerations                                                              */
/* ========================================================================= */

/** Stream state machine */
enum pw_stream_state {
    PW_STREAM_STATE_UNCONNECTED = 0,
    PW_STREAM_STATE_CONNECTING  = 1,
    PW_STREAM_STATE_PAUSED      = 2,
    PW_STREAM_STATE_STREAMING   = 3,
    PW_STREAM_STATE_ERROR       = -1
};

/** Port / stream direction */
enum pw_direction {
    PW_DIRECTION_INPUT  = 0,
    PW_DIRECTION_OUTPUT = 1
};

/** Node state (mirrors kernel PCM state where applicable) */
enum pw_node_state {
    PW_NODE_STATE_ERROR     = -1,
    PW_NODE_STATE_CREATING  = 0,
    PW_NODE_STATE_SUSPENDED = 1,
    PW_NODE_STATE_IDLE      = 2,
    PW_NODE_STATE_RUNNING   = 3
};

/* ========================================================================= */
/* SPA audio format negotiation                                              */
/* ========================================================================= */

/** Audio sample format identifiers (subset) */
enum spa_audio_format {
    SPA_AUDIO_FORMAT_UNKNOWN  = 0,
    SPA_AUDIO_FORMAT_U8       = 1,
    SPA_AUDIO_FORMAT_S16_LE   = 2,
    SPA_AUDIO_FORMAT_S32_LE   = 3,
    SPA_AUDIO_FORMAT_F32_LE   = 4,   /* mapped to 16.16 fixed-point internally */
    SPA_AUDIO_FORMAT_S24_LE   = 5
};

/** Channel position identifiers */
enum spa_audio_channel {
    SPA_AUDIO_CHANNEL_UNKNOWN = 0,
    SPA_AUDIO_CHANNEL_MONO    = 1,
    SPA_AUDIO_CHANNEL_FL      = 2,   /* Front Left */
    SPA_AUDIO_CHANNEL_FR      = 3,   /* Front Right */
    SPA_AUDIO_CHANNEL_FC      = 4,   /* Front Center */
    SPA_AUDIO_CHANNEL_LFE     = 5,
    SPA_AUDIO_CHANNEL_RL      = 6,   /* Rear Left */
    SPA_AUDIO_CHANNEL_RR      = 7,   /* Rear Right */
    SPA_AUDIO_CHANNEL_SL      = 8,   /* Side Left */
    SPA_AUDIO_CHANNEL_SR      = 9    /* Side Right */
};

#define SPA_AUDIO_MAX_CHANNELS 8

/** Raw audio format descriptor */
struct spa_audio_info_raw {
    enum spa_audio_format format;
    uint32_t rate;
    uint32_t channels;
    enum spa_audio_channel position[SPA_AUDIO_MAX_CHANNELS];
};

/* ========================================================================= */
/* Buffer types                                                              */
/* ========================================================================= */

/** Data chunk within a SPA buffer */
struct spa_data {
    void    *data;          /**< Pointer to mapped memory */
    uint32_t maxsize;       /**< Maximum size in bytes */
    uint32_t chunk_offset;  /**< Offset of valid data */
    uint32_t chunk_size;    /**< Size of valid data */
    uint32_t chunk_stride;  /**< Stride between frames */
};

/** SPA buffer -- one or more data planes */
struct spa_buffer {
    uint32_t         n_datas;
    struct spa_data *datas;
};

/** PipeWire buffer wrapper */
struct pw_buffer {
    struct spa_buffer *buffer;
    uint64_t           size;     /**< Negotiated buffer size in bytes */
    void              *user_data;
};

/* ========================================================================= */
/* Stream events                                                             */
/* ========================================================================= */

/** Callbacks delivered to stream owners */
struct pw_stream_events {
    /** Protocol version (set to 0) */
    uint32_t version;

    /** Stream state changed */
    void (*state_changed)(void *data,
                          enum pw_stream_state old_state,
                          enum pw_stream_state new_state,
                          const char *error);

    /** A buffer is ready to process */
    void (*process)(void *data);

    /** Format parameters changed (may be NULL) */
    void (*param_changed)(void *data,
                          uint32_t id,
                          const void *param);
};

/* ========================================================================= */
/* Stream flags                                                              */
/* ========================================================================= */

enum pw_stream_flags {
    PW_STREAM_FLAG_NONE       = 0,
    PW_STREAM_FLAG_AUTOCONNECT = (1 << 0),
    PW_STREAM_FLAG_MAP_BUFFERS = (1 << 1),
    PW_STREAM_FLAG_RT_PROCESS  = (1 << 2)
};

/* ========================================================================= */
/* Properties (key-value bag)                                                */
/* ========================================================================= */

struct pw_properties *pw_properties_new(const char *key, ...);
const char           *pw_properties_get(const struct pw_properties *props,
                                        const char *key);
int                   pw_properties_set(struct pw_properties *props,
                                        const char *key,
                                        const char *value);
void                  pw_properties_free(struct pw_properties *props);

/* ========================================================================= */
/* Initialization                                                            */
/* ========================================================================= */

/** Initialise the PipeWire library. Call once at startup. */
void pw_init(int *argc, char ***argv);

/** Clean up the PipeWire library. */
void pw_deinit(void);

/* ========================================================================= */
/* Main loop                                                                 */
/* ========================================================================= */

struct pw_main_loop *pw_main_loop_new(const struct pw_properties *props);
int                  pw_main_loop_run(struct pw_main_loop *loop);
int                  pw_main_loop_quit(struct pw_main_loop *loop);
void                 pw_main_loop_destroy(struct pw_main_loop *loop);

/** Obtain the loop implementation for pw_context_new(). */
void *pw_main_loop_get_loop(struct pw_main_loop *loop);

/* ========================================================================= */
/* Context / Core                                                            */
/* ========================================================================= */

struct pw_context *pw_context_new(void *loop,
                                  struct pw_properties *props,
                                  size_t user_data_size);
void               pw_context_destroy(struct pw_context *ctx);

struct pw_core *pw_context_connect(struct pw_context *ctx,
                                   struct pw_properties *props,
                                   size_t user_data_size);

/* ========================================================================= */
/* Stream                                                                    */
/* ========================================================================= */

struct pw_stream *pw_stream_new(struct pw_core *core,
                                const char *name,
                                struct pw_properties *props);

int pw_stream_connect(struct pw_stream *stream,
                      enum pw_direction direction,
                      uint32_t target_id,
                      enum pw_stream_flags flags,
                      const void **params,
                      uint32_t n_params);

int  pw_stream_disconnect(struct pw_stream *stream);
void pw_stream_destroy(struct pw_stream *stream);

struct pw_buffer *pw_stream_dequeue_buffer(struct pw_stream *stream);
int               pw_stream_queue_buffer(struct pw_stream *stream,
                                         struct pw_buffer *buf);

enum pw_stream_state pw_stream_get_state(struct pw_stream *stream,
                                         const char **error);

void pw_stream_add_listener(struct pw_stream *stream,
                            void *listener_data,
                            const struct pw_stream_events *events,
                            void *data);

/** Target ID that means "don't care / let the session manager choose" */
#define PW_ID_ANY ((uint32_t)0xFFFFFFFF)

#ifdef __cplusplus
}  /* extern "C" */
#endif

#endif /* PIPEWIRE_VERIDIAN_H */
