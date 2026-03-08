/*
 * VeridianOS -- pipewire-veridian.cpp
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * PipeWire audio daemon implementation for VeridianOS.  Manages a graph
 * of audio nodes (sources, sinks), ports, and links on top of the kernel
 * ALSA subsystem via the pw-alsa-bridge layer.
 *
 * Key design decisions:
 *   - Single-threaded event loop (no pthreads required at this stage)
 *   - All mixing/resampling uses integer or 16.16 fixed-point math
 *   - Node graph stored in static arrays (bounded memory, no heap churn)
 *   - D-Bus registration at org.freedesktop.impl.portal.PipeWire is
 *     stubbed; actual D-Bus is provided by the VeridianOS dbus shim
 */

#include "pipewire-veridian.h"
#include "pw-alsa-bridge.h"

#include <stdlib.h>
#include <string.h>
#include <stdio.h>
#include <errno.h>

/* ========================================================================= */
/* Internal constants                                                        */
/* ========================================================================= */

#define PW_MAX_NODES        64
#define PW_MAX_PORTS        128
#define PW_MAX_LINKS        64
#define PW_MAX_STREAMS      32
#define PW_MAX_PROPERTIES   32
#define PW_MAX_BUFFERS      4
#define PW_BUFFER_SIZE      4096   /* default buffer size in bytes */

/* Fixed-point 16.16 helpers */
#define FP_SHIFT  16
#define FP_ONE    (1 << FP_SHIFT)
#define FP_HALF   (1 << (FP_SHIFT - 1))

/* ========================================================================= */
/* Property bag implementation                                               */
/* ========================================================================= */

struct pw_property_entry {
    char key[64];
    char value[128];
    int  used;
};

struct pw_properties {
    struct pw_property_entry entries[PW_MAX_PROPERTIES];
    int count;
};

struct pw_properties *pw_properties_new(const char *key, ...) {
    struct pw_properties *props =
        (struct pw_properties *)calloc(1, sizeof(struct pw_properties));
    if (!props) return NULL;

    if (key) {
        /* Parse first key; varargs pairs handled below */
        va_list ap;
        va_start(ap, key);
        const char *k = key;
        while (k && props->count < PW_MAX_PROPERTIES) {
            const char *v = va_arg(ap, const char *);
            if (!v) break;
            strncpy(props->entries[props->count].key, k,
                    sizeof(props->entries[0].key) - 1);
            strncpy(props->entries[props->count].value, v,
                    sizeof(props->entries[0].value) - 1);
            props->entries[props->count].used = 1;
            props->count++;
            k = va_arg(ap, const char *);
        }
        va_end(ap);
    }
    return props;
}

const char *pw_properties_get(const struct pw_properties *props,
                              const char *key) {
    if (!props || !key) return NULL;
    for (int i = 0; i < props->count; i++) {
        if (props->entries[i].used &&
            strcmp(props->entries[i].key, key) == 0) {
            return props->entries[i].value;
        }
    }
    return NULL;
}

int pw_properties_set(struct pw_properties *props,
                      const char *key, const char *value) {
    if (!props || !key) return -EINVAL;

    /* Update existing entry */
    for (int i = 0; i < props->count; i++) {
        if (props->entries[i].used &&
            strcmp(props->entries[i].key, key) == 0) {
            if (value) {
                strncpy(props->entries[i].value, value,
                        sizeof(props->entries[0].value) - 1);
            } else {
                props->entries[i].used = 0;
            }
            return 0;
        }
    }

    /* Add new entry */
    if (!value) return 0;
    if (props->count >= PW_MAX_PROPERTIES) return -ENOMEM;
    strncpy(props->entries[props->count].key, key,
            sizeof(props->entries[0].key) - 1);
    strncpy(props->entries[props->count].value, value,
            sizeof(props->entries[0].value) - 1);
    props->entries[props->count].used = 1;
    props->count++;
    return 0;
}

void pw_properties_free(struct pw_properties *props) {
    free(props);
}

/* ========================================================================= */
/* Node graph types                                                          */
/* ========================================================================= */

struct pw_node_internal {
    uint32_t          id;
    char              name[64];
    enum pw_node_state state;
    enum pw_direction direction;
    int               used;
};

struct pw_port_internal {
    uint32_t          id;
    uint32_t          node_id;
    enum pw_direction direction;
    int               used;
};

struct pw_link_internal {
    uint32_t id;
    uint32_t output_port_id;
    uint32_t input_port_id;
    int      used;
};

/* ========================================================================= */
/* Stream internals                                                          */
/* ========================================================================= */

struct pw_stream_internal {
    uint32_t                id;
    char                    name[64];
    enum pw_stream_state    state;
    enum pw_direction       direction;
    struct spa_audio_info_raw format;

    /* Buffers */
    struct pw_buffer        buffers[PW_MAX_BUFFERS];
    struct spa_buffer       spa_bufs[PW_MAX_BUFFERS];
    struct spa_data         spa_datas[PW_MAX_BUFFERS];
    uint8_t                 buf_mem[PW_MAX_BUFFERS][PW_BUFFER_SIZE];
    int                     buf_queued[PW_MAX_BUFFERS]; /* 1 = queued */
    int                     n_buffers;

    /* ALSA bridge (playback/capture backend) */
    struct AlsaBridge      *bridge;
    int                     bridge_open;

    /* Events */
    struct pw_stream_events events;
    void                   *events_data;

    /* Associated node ID */
    uint32_t                node_id;

    int                     used;
};

/* ========================================================================= */
/* Global daemon state                                                       */
/* ========================================================================= */

struct pw_daemon_state {
    int initialised;
    int running;

    /* Node graph */
    struct pw_node_internal nodes[PW_MAX_NODES];
    uint32_t next_node_id;
    int      node_count;

    struct pw_port_internal ports[PW_MAX_PORTS];
    uint32_t next_port_id;
    int      port_count;

    struct pw_link_internal links[PW_MAX_LINKS];
    uint32_t next_link_id;
    int      link_count;

    /* Streams */
    struct pw_stream_internal streams[PW_MAX_STREAMS];
    uint32_t next_stream_id;
    int      stream_count;
};

static struct pw_daemon_state g_state;

/* ========================================================================= */
/* Internal helpers                                                          */
/* ========================================================================= */

static struct pw_node_internal *find_node(uint32_t id) {
    for (int i = 0; i < PW_MAX_NODES; i++) {
        if (g_state.nodes[i].used && g_state.nodes[i].id == id)
            return &g_state.nodes[i];
    }
    return NULL;
}

static uint32_t create_node(const char *name, enum pw_direction dir) {
    for (int i = 0; i < PW_MAX_NODES; i++) {
        if (!g_state.nodes[i].used) {
            g_state.nodes[i].id = g_state.next_node_id++;
            strncpy(g_state.nodes[i].name, name,
                    sizeof(g_state.nodes[i].name) - 1);
            g_state.nodes[i].state = PW_NODE_STATE_IDLE;
            g_state.nodes[i].direction = dir;
            g_state.nodes[i].used = 1;
            g_state.node_count++;
            return g_state.nodes[i].id;
        }
    }
    return (uint32_t)-1;
}

static uint32_t create_port(uint32_t node_id, enum pw_direction dir) {
    for (int i = 0; i < PW_MAX_PORTS; i++) {
        if (!g_state.ports[i].used) {
            g_state.ports[i].id = g_state.next_port_id++;
            g_state.ports[i].node_id = node_id;
            g_state.ports[i].direction = dir;
            g_state.ports[i].used = 1;
            g_state.port_count++;
            return g_state.ports[i].id;
        }
    }
    return (uint32_t)-1;
}

static uint32_t create_link(uint32_t output_port, uint32_t input_port) {
    for (int i = 0; i < PW_MAX_LINKS; i++) {
        if (!g_state.links[i].used) {
            g_state.links[i].id = g_state.next_link_id++;
            g_state.links[i].output_port_id = output_port;
            g_state.links[i].input_port_id = input_port;
            g_state.links[i].used = 1;
            g_state.link_count++;
            return g_state.links[i].id;
        }
    }
    return (uint32_t)-1;
}

static struct pw_stream_internal *find_stream(uint32_t id) {
    for (int i = 0; i < PW_MAX_STREAMS; i++) {
        if (g_state.streams[i].used && g_state.streams[i].id == id)
            return &g_state.streams[i];
    }
    return NULL;
}

static void stream_set_state(struct pw_stream_internal *s,
                             enum pw_stream_state new_state,
                             const char *error) {
    if (!s) return;
    enum pw_stream_state old = s->state;
    s->state = new_state;
    if (s->events.state_changed) {
        s->events.state_changed(s->events_data, old, new_state, error);
    }
}

/* Set up buffers for a stream */
static void stream_init_buffers(struct pw_stream_internal *s) {
    s->n_buffers = PW_MAX_BUFFERS;
    for (int i = 0; i < PW_MAX_BUFFERS; i++) {
        s->spa_datas[i].data       = s->buf_mem[i];
        s->spa_datas[i].maxsize    = PW_BUFFER_SIZE;
        s->spa_datas[i].chunk_offset = 0;
        s->spa_datas[i].chunk_size   = 0;
        s->spa_datas[i].chunk_stride = 0;

        s->spa_bufs[i].n_datas = 1;
        s->spa_bufs[i].datas   = &s->spa_datas[i];

        s->buffers[i].buffer    = &s->spa_bufs[i];
        s->buffers[i].size      = PW_BUFFER_SIZE;
        s->buffers[i].user_data = NULL;

        s->buf_queued[i] = 0; /* not queued -- available for dequeue */
    }
}

/* Open the ALSA backend for a stream */
static int stream_open_alsa(struct pw_stream_internal *s) {
    if (s->bridge_open) return 0;

    s->bridge = alsa_bridge_new();
    if (!s->bridge) return -ENOMEM;

    /* Map PipeWire format to ALSA bridge format */
    enum alsa_bridge_format fmt;
    switch (s->format.format) {
        case SPA_AUDIO_FORMAT_U8:     fmt = ALSA_BRIDGE_FORMAT_U8;     break;
        case SPA_AUDIO_FORMAT_S16_LE: fmt = ALSA_BRIDGE_FORMAT_S16_LE; break;
        case SPA_AUDIO_FORMAT_S32_LE: fmt = ALSA_BRIDGE_FORMAT_S32_LE; break;
        default:                      fmt = ALSA_BRIDGE_FORMAT_S16_LE; break;
    }

    /* Select device path based on direction */
    const char *dev = (s->direction == PW_DIRECTION_OUTPUT)
                      ? "/dev/snd/pcmC0D0p"
                      : "/dev/snd/pcmC0D0c";

    uint32_t rate = s->format.rate ? s->format.rate : 48000;
    uint32_t ch   = s->format.channels ? s->format.channels : 2;

    int ret = alsa_bridge_open(s->bridge, dev, fmt, rate, ch);
    if (ret < 0) {
        alsa_bridge_destroy(s->bridge);
        s->bridge = NULL;
        return ret;
    }

    s->bridge_open = 1;
    return 0;
}

/* ========================================================================= */
/* Initialization                                                            */
/* ========================================================================= */

void pw_init(int *argc, char ***argv) {
    (void)argc;
    (void)argv;

    if (g_state.initialised) return;

    memset(&g_state, 0, sizeof(g_state));
    g_state.initialised = 1;
    g_state.next_node_id   = 1;
    g_state.next_port_id   = 1;
    g_state.next_link_id   = 1;
    g_state.next_stream_id = 1;

    /* Create default sink node ("Built-in Audio Analog Stereo") */
    uint32_t sink_id = create_node("alsa_output.pci-0000_00_1b.0.analog-stereo",
                                   PW_DIRECTION_INPUT);
    if (sink_id != (uint32_t)-1) {
        create_port(sink_id, PW_DIRECTION_INPUT);  /* FL */
        create_port(sink_id, PW_DIRECTION_INPUT);  /* FR */
    }

    /* Create default source node ("Built-in Audio Analog Stereo") */
    uint32_t source_id = create_node("alsa_input.pci-0000_00_1b.0.analog-stereo",
                                     PW_DIRECTION_OUTPUT);
    if (source_id != (uint32_t)-1) {
        create_port(source_id, PW_DIRECTION_OUTPUT);  /* FL */
        create_port(source_id, PW_DIRECTION_OUTPUT);  /* FR */
    }

    fprintf(stderr, "[pipewire] Initialised PipeWire daemon (VeridianOS)\n");
    fprintf(stderr, "[pipewire]   Default sink:   node %u\n", sink_id);
    fprintf(stderr, "[pipewire]   Default source: node %u\n", source_id);
}

void pw_deinit(void) {
    if (!g_state.initialised) return;

    /* Close all streams */
    for (int i = 0; i < PW_MAX_STREAMS; i++) {
        if (g_state.streams[i].used && g_state.streams[i].bridge_open) {
            alsa_bridge_close(g_state.streams[i].bridge);
            alsa_bridge_destroy(g_state.streams[i].bridge);
            g_state.streams[i].bridge = NULL;
            g_state.streams[i].bridge_open = 0;
        }
    }

    memset(&g_state, 0, sizeof(g_state));
    fprintf(stderr, "[pipewire] Shut down PipeWire daemon\n");
}

/* ========================================================================= */
/* Main loop                                                                 */
/* ========================================================================= */

struct pw_main_loop {
    int running;
    int quit_requested;
};

struct pw_main_loop *pw_main_loop_new(const struct pw_properties *props) {
    (void)props;
    struct pw_main_loop *loop =
        (struct pw_main_loop *)calloc(1, sizeof(struct pw_main_loop));
    return loop;
}

int pw_main_loop_run(struct pw_main_loop *loop) {
    if (!loop) return -EINVAL;

    loop->running = 1;
    loop->quit_requested = 0;

    /* Simple polling loop: iterate through all active streams and
     * invoke their process callbacks when buffers are available. */
    while (!loop->quit_requested) {
        int activity = 0;
        for (int i = 0; i < PW_MAX_STREAMS; i++) {
            struct pw_stream_internal *s = &g_state.streams[i];
            if (!s->used || s->state != PW_STREAM_STATE_STREAMING)
                continue;

            /* If the stream is streaming and has a process callback,
             * invoke it to let the client fill/drain buffers. */
            if (s->events.process) {
                s->events.process(s->events_data);
                activity = 1;
            }
        }

        /* If no streams are active, yield to avoid busy-spinning.
         * On VeridianOS, this maps to sched_yield(). */
        if (!activity) {
            /* Stub: in a full implementation this would poll fds */
            break;
        }
    }

    loop->running = 0;
    return 0;
}

int pw_main_loop_quit(struct pw_main_loop *loop) {
    if (!loop) return -EINVAL;
    loop->quit_requested = 1;
    return 0;
}

void pw_main_loop_destroy(struct pw_main_loop *loop) {
    free(loop);
}

void *pw_main_loop_get_loop(struct pw_main_loop *loop) {
    return (void *)loop;
}

/* ========================================================================= */
/* Context / Core                                                            */
/* ========================================================================= */

struct pw_context {
    void *loop;
    int   connected;
};

struct pw_core {
    struct pw_context *context;
};

/* Single global core (VeridianOS runs one daemon per session) */
static struct pw_core g_core;

struct pw_context *pw_context_new(void *loop,
                                  struct pw_properties *props,
                                  size_t user_data_size) {
    (void)props;
    (void)user_data_size;

    struct pw_context *ctx =
        (struct pw_context *)calloc(1, sizeof(struct pw_context));
    if (!ctx) return NULL;
    ctx->loop = loop;
    return ctx;
}

void pw_context_destroy(struct pw_context *ctx) {
    free(ctx);
}

struct pw_core *pw_context_connect(struct pw_context *ctx,
                                   struct pw_properties *props,
                                   size_t user_data_size) {
    (void)props;
    (void)user_data_size;

    if (!ctx) return NULL;
    ctx->connected = 1;
    g_core.context = ctx;
    return &g_core;
}

/* ========================================================================= */
/* Stream API                                                                */
/* ========================================================================= */

/*
 * Externally, pw_stream is opaque.  Internally we return a pointer into
 * the g_state.streams[] array cast to struct pw_stream*.
 */

struct pw_stream *pw_stream_new(struct pw_core *core,
                                const char *name,
                                struct pw_properties *props) {
    (void)core;
    (void)props;

    if (!g_state.initialised) return NULL;

    /* Find a free stream slot */
    for (int i = 0; i < PW_MAX_STREAMS; i++) {
        if (!g_state.streams[i].used) {
            struct pw_stream_internal *s = &g_state.streams[i];
            memset(s, 0, sizeof(*s));
            s->id = g_state.next_stream_id++;
            if (name) {
                strncpy(s->name, name, sizeof(s->name) - 1);
            }
            s->state = PW_STREAM_STATE_UNCONNECTED;
            s->used = 1;
            g_state.stream_count++;

            fprintf(stderr, "[pipewire] Created stream '%s' (id=%u)\n",
                    s->name, s->id);
            return (struct pw_stream *)s;
        }
    }
    return NULL;
}

int pw_stream_connect(struct pw_stream *stream,
                      enum pw_direction direction,
                      uint32_t target_id,
                      enum pw_stream_flags flags,
                      const void **params,
                      uint32_t n_params) {
    (void)target_id;
    (void)flags;

    struct pw_stream_internal *s = (struct pw_stream_internal *)stream;
    if (!s || !s->used) return -EINVAL;

    s->direction = direction;

    /* Parse format parameters if provided */
    if (params && n_params > 0) {
        /* First param is expected to be spa_audio_info_raw */
        const struct spa_audio_info_raw *raw =
            (const struct spa_audio_info_raw *)params[0];
        if (raw) {
            s->format = *raw;
        }
    }

    /* Apply defaults if format not fully specified */
    if (s->format.format == SPA_AUDIO_FORMAT_UNKNOWN)
        s->format.format = SPA_AUDIO_FORMAT_S16_LE;
    if (s->format.rate == 0)
        s->format.rate = 48000;
    if (s->format.channels == 0) {
        s->format.channels = 2;
        s->format.position[0] = SPA_AUDIO_CHANNEL_FL;
        s->format.position[1] = SPA_AUDIO_CHANNEL_FR;
    }

    /* Create a node for this stream */
    s->node_id = create_node(s->name, direction);

    /* Transition: UNCONNECTED -> CONNECTING -> PAUSED */
    stream_set_state(s, PW_STREAM_STATE_CONNECTING, NULL);

    /* Initialise buffers */
    stream_init_buffers(s);

    /* Open ALSA backend */
    int ret = stream_open_alsa(s);
    if (ret < 0) {
        stream_set_state(s, PW_STREAM_STATE_ERROR,
                         "Failed to open ALSA device");
        return ret;
    }

    /* Auto-link to default sink/source if target is PW_ID_ANY */
    if (target_id == PW_ID_ANY) {
        /* Find the default sink or source node's port */
        for (int i = 0; i < PW_MAX_PORTS; i++) {
            if (!g_state.ports[i].used) continue;

            enum pw_direction needed = (direction == PW_DIRECTION_OUTPUT)
                                       ? PW_DIRECTION_INPUT
                                       : PW_DIRECTION_OUTPUT;
            if (g_state.ports[i].direction == needed) {
                /* Find a port on our stream's node */
                uint32_t our_port = create_port(s->node_id, direction);
                if (our_port != (uint32_t)-1) {
                    if (direction == PW_DIRECTION_OUTPUT) {
                        create_link(our_port, g_state.ports[i].id);
                    } else {
                        create_link(g_state.ports[i].id, our_port);
                    }
                }
                break;
            }
        }
    }

    stream_set_state(s, PW_STREAM_STATE_PAUSED, NULL);

    /* If auto-connect requested, start streaming immediately */
    if (flags & PW_STREAM_FLAG_AUTOCONNECT) {
        stream_set_state(s, PW_STREAM_STATE_STREAMING, NULL);
        struct pw_node_internal *node = find_node(s->node_id);
        if (node) node->state = PW_NODE_STATE_RUNNING;
    }

    fprintf(stderr, "[pipewire] Stream '%s' connected (dir=%d, fmt=%d, "
            "rate=%u, ch=%u)\n",
            s->name, direction, s->format.format,
            s->format.rate, s->format.channels);
    return 0;
}

int pw_stream_disconnect(struct pw_stream *stream) {
    struct pw_stream_internal *s = (struct pw_stream_internal *)stream;
    if (!s || !s->used) return -EINVAL;

    if (s->bridge_open) {
        alsa_bridge_close(s->bridge);
        alsa_bridge_destroy(s->bridge);
        s->bridge = NULL;
        s->bridge_open = 0;
    }

    stream_set_state(s, PW_STREAM_STATE_UNCONNECTED, NULL);
    return 0;
}

void pw_stream_destroy(struct pw_stream *stream) {
    struct pw_stream_internal *s = (struct pw_stream_internal *)stream;
    if (!s || !s->used) return;

    pw_stream_disconnect(stream);

    s->used = 0;
    g_state.stream_count--;

    fprintf(stderr, "[pipewire] Destroyed stream '%s' (id=%u)\n",
            s->name, s->id);
}

struct pw_buffer *pw_stream_dequeue_buffer(struct pw_stream *stream) {
    struct pw_stream_internal *s = (struct pw_stream_internal *)stream;
    if (!s || !s->used) return NULL;

    /* Find first non-queued buffer */
    for (int i = 0; i < s->n_buffers; i++) {
        if (!s->buf_queued[i]) {
            s->buf_queued[i] = 1;
            /* Reset chunk for fresh write */
            s->spa_datas[i].chunk_offset = 0;
            s->spa_datas[i].chunk_size   = 0;
            return &s->buffers[i];
        }
    }
    return NULL; /* All buffers are in flight */
}

int pw_stream_queue_buffer(struct pw_stream *stream,
                           struct pw_buffer *buf) {
    struct pw_stream_internal *s = (struct pw_stream_internal *)stream;
    if (!s || !s->used || !buf) return -EINVAL;

    /* Identify which buffer index this is */
    int idx = -1;
    for (int i = 0; i < s->n_buffers; i++) {
        if (&s->buffers[i] == buf) {
            idx = i;
            break;
        }
    }
    if (idx < 0) return -EINVAL;

    /* If playback, write the buffer data to ALSA */
    if (s->direction == PW_DIRECTION_OUTPUT && s->bridge_open) {
        struct spa_data *d = &s->spa_datas[idx];
        if (d->chunk_size > 0) {
            /* Calculate frame count: chunk_size / (channels * bytes_per_sample) */
            uint32_t bps = 2; /* default S16_LE */
            switch (s->format.format) {
                case SPA_AUDIO_FORMAT_U8:     bps = 1; break;
                case SPA_AUDIO_FORMAT_S16_LE: bps = 2; break;
                case SPA_AUDIO_FORMAT_S32_LE: bps = 4; break;
                default:                      bps = 2; break;
            }
            uint32_t ch = s->format.channels ? s->format.channels : 2;
            uint32_t frame_size = ch * bps;
            uint32_t frames = (frame_size > 0) ? d->chunk_size / frame_size : 0;

            if (frames > 0) {
                uint8_t *data = (uint8_t *)d->data + d->chunk_offset;
                alsa_bridge_write(s->bridge, data, frames);
            }
        }
    }

    /* Mark buffer as available again */
    s->buf_queued[idx] = 0;
    return 0;
}

enum pw_stream_state pw_stream_get_state(struct pw_stream *stream,
                                         const char **error) {
    struct pw_stream_internal *s = (struct pw_stream_internal *)stream;
    if (!s || !s->used) {
        if (error) *error = "invalid stream";
        return PW_STREAM_STATE_ERROR;
    }
    if (error) *error = NULL;
    return s->state;
}

void pw_stream_add_listener(struct pw_stream *stream,
                            void *listener_data,
                            const struct pw_stream_events *events,
                            void *data) {
    (void)listener_data;
    struct pw_stream_internal *s = (struct pw_stream_internal *)stream;
    if (!s || !s->used || !events) return;

    s->events = *events;
    s->events_data = data;
}

/* ========================================================================= */
/* D-Bus portal registration (stub)                                          */
/* ========================================================================= */

/*
 * In a full PipeWire daemon, D-Bus is used to register the portal
 * interface at org.freedesktop.impl.portal.PipeWire.  On VeridianOS,
 * D-Bus is provided by the dbus-veridian shim (Sprint 9.5).
 *
 * This stub logs the intent but does not perform actual D-Bus IPC.
 * Plasma audio applet connects to PipeWire directly via the C API.
 */
static void pw_register_dbus_portal(void) {
    fprintf(stderr, "[pipewire] D-Bus portal registration: "
            "org.freedesktop.impl.portal.PipeWire (stub)\n");
}

/* ========================================================================= */
/* Daemon entry point (for standalone operation)                             */
/* ========================================================================= */

/*
 * When built as a standalone daemon, main() initialises PipeWire,
 * registers the D-Bus portal, and runs the main loop.
 *
 * When used as a library (linked into a host process), callers use
 * pw_init() / pw_main_loop_new() / pw_main_loop_run() directly.
 */
#ifdef PW_STANDALONE_DAEMON

int main(int argc, char *argv[]) {
    pw_init(&argc, &argv);
    pw_register_dbus_portal();

    struct pw_main_loop *loop = pw_main_loop_new(NULL);
    if (!loop) {
        fprintf(stderr, "[pipewire] Failed to create main loop\n");
        pw_deinit();
        return 1;
    }

    fprintf(stderr, "[pipewire] Entering main loop...\n");
    pw_main_loop_run(loop);

    pw_main_loop_destroy(loop);
    pw_deinit();
    return 0;
}

#endif /* PW_STANDALONE_DAEMON */
