/*
 * VeridianOS -- pw-alsa-bridge.cpp
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Bridge between PipeWire buffer semantics and the VeridianOS kernel
 * ALSA subsystem.  Opens kernel PCM devices at /dev/snd/pcmC{n}D{n}{p|c},
 * translates PipeWire buffers to ALSA frames, and provides integer-only
 * 16.16 fixed-point resampling for rate conversion.
 *
 * All arithmetic is integer or fixed-point -- no floating point is used.
 */

#include "pw-alsa-bridge.h"

#include <stdlib.h>
#include <string.h>
#include <stdio.h>
#include <errno.h>
#include <fcntl.h>
#include <unistd.h>
#include <sys/ioctl.h>

/* ========================================================================= */
/* ALSA ioctl definitions (must match kernel/src/audio/alsa.rs)              */
/* ========================================================================= */

#define SNDRV_PCM_IOCTL_HW_PARAMS  (0x4100 + 0x11)
#define SNDRV_PCM_IOCTL_SW_PARAMS  (0x4100 + 0x13)
#define SNDRV_PCM_IOCTL_PREPARE    (0x4100 + 0x40)
#define SNDRV_PCM_IOCTL_START      (0x4100 + 0x42)
#define SNDRV_PCM_IOCTL_STOP       (0x4100 + 0x43)
#define SNDRV_PCM_IOCTL_STATUS     (0x4100 + 0x20)

/* ========================================================================= */
/* HW params structure passed to kernel via ioctl                            */
/* ========================================================================= */

struct alsa_ioctl_hw_params {
    uint32_t sample_rate;
    uint8_t  channels;
    uint8_t  format;       /* 0=U8, 1=S16LE, 2=S32LE */
    uint8_t  _pad[2];
    uint32_t buffer_size;  /* in frames */
    uint32_t period_size;  /* in frames */
};

struct alsa_ioctl_sw_params {
    uint32_t avail_min;
    uint32_t start_threshold;
    uint32_t stop_threshold;
};

struct alsa_ioctl_status {
    uint32_t state;          /* PcmState enum value */
    uint64_t frames_played;
    uint64_t frames_captured;
    uint32_t avail;          /* frames available */
};

/* ========================================================================= */
/* Fixed-point 16.16 resampler                                               */
/* ========================================================================= */

#define FP_SHIFT  16
#define FP_ONE    (1 << FP_SHIFT)
#define FP_HALF   (1 << (FP_SHIFT - 1))

/*
 * Resample S16LE audio from src_rate to dst_rate using linear interpolation
 * with 16.16 fixed-point arithmetic.
 *
 * Returns the number of output frames written.
 */
static uint32_t resample_s16_fixed(const int16_t *src, uint32_t src_frames,
                                   int16_t *dst, uint32_t dst_max_frames,
                                   uint32_t src_rate, uint32_t dst_rate,
                                   uint32_t channels) {
    if (src_rate == dst_rate || src_frames == 0 || channels == 0) {
        /* No resampling needed -- copy directly */
        uint32_t copy_frames = (src_frames < dst_max_frames)
                               ? src_frames : dst_max_frames;
        memcpy(dst, src, copy_frames * channels * sizeof(int16_t));
        return copy_frames;
    }

    /* Fixed-point ratio: src_rate / dst_rate in 16.16 */
    uint32_t ratio_fp = ((uint64_t)src_rate << FP_SHIFT) / dst_rate;

    uint32_t out_frames = 0;
    uint32_t pos_fp = 0; /* 16.16 position in source */

    while (out_frames < dst_max_frames) {
        uint32_t src_idx = pos_fp >> FP_SHIFT;
        if (src_idx + 1 >= src_frames) break;

        /* Fractional part for interpolation */
        uint32_t frac = pos_fp & (FP_ONE - 1);

        for (uint32_t ch = 0; ch < channels; ch++) {
            int32_t s0 = src[src_idx * channels + ch];
            int32_t s1 = src[(src_idx + 1) * channels + ch];

            /* Linear interpolation: s0 + (s1 - s0) * frac / FP_ONE */
            int32_t interp = s0 + (((s1 - s0) * (int32_t)frac + FP_HALF) >> FP_SHIFT);

            /* Clamp to int16 range */
            if (interp > 32767) interp = 32767;
            if (interp < -32768) interp = -32768;

            dst[out_frames * channels + ch] = (int16_t)interp;
        }

        out_frames++;
        pos_fp += ratio_fp;
    }

    return out_frames;
}

/*
 * Resample S32LE audio using the same 16.16 fixed-point approach.
 */
static uint32_t resample_s32_fixed(const int32_t *src, uint32_t src_frames,
                                   int32_t *dst, uint32_t dst_max_frames,
                                   uint32_t src_rate, uint32_t dst_rate,
                                   uint32_t channels) {
    if (src_rate == dst_rate || src_frames == 0 || channels == 0) {
        uint32_t copy_frames = (src_frames < dst_max_frames)
                               ? src_frames : dst_max_frames;
        memcpy(dst, src, copy_frames * channels * sizeof(int32_t));
        return copy_frames;
    }

    uint32_t ratio_fp = ((uint64_t)src_rate << FP_SHIFT) / dst_rate;
    uint32_t out_frames = 0;
    uint32_t pos_fp = 0;

    while (out_frames < dst_max_frames) {
        uint32_t src_idx = pos_fp >> FP_SHIFT;
        if (src_idx + 1 >= src_frames) break;

        uint32_t frac = pos_fp & (FP_ONE - 1);

        for (uint32_t ch = 0; ch < channels; ch++) {
            int64_t s0 = src[src_idx * channels + ch];
            int64_t s1 = src[(src_idx + 1) * channels + ch];
            int64_t interp = s0 + (((s1 - s0) * (int64_t)frac + FP_HALF) >> FP_SHIFT);

            /* Clamp to int32 range */
            if (interp > 2147483647LL) interp = 2147483647LL;
            if (interp < -2147483648LL) interp = -2147483648LL;

            dst[out_frames * channels + ch] = (int32_t)interp;
        }

        out_frames++;
        pos_fp += ratio_fp;
    }

    return out_frames;
}

/* ========================================================================= */
/* Bridge implementation                                                     */
/* ========================================================================= */

struct AlsaBridge {
    int                      fd;           /* Kernel PCM device fd */
    int                      is_open;
    int                      is_playback;  /* 1 = playback, 0 = capture */

    /* Current configuration */
    enum alsa_bridge_format  format;
    uint32_t                 rate;
    uint32_t                 channels;
    uint32_t                 period_frames;
    uint32_t                 buffer_frames;

    /* Derived values */
    uint32_t                 bytes_per_sample;
    uint32_t                 frame_size;   /* channels * bytes_per_sample */

    /* Latency tracking (integer microseconds) */
    uint32_t                 latency_us;

    /* Resampling buffer (used when device rate differs from requested) */
    uint32_t                 device_rate;
    uint8_t                 *resample_buf;
    uint32_t                 resample_buf_size;
};

struct AlsaBridge *alsa_bridge_new(void) {
    struct AlsaBridge *b =
        (struct AlsaBridge *)calloc(1, sizeof(struct AlsaBridge));
    if (!b) return NULL;
    b->fd = -1;
    b->period_frames = 1024;
    b->buffer_frames = 4096;
    return b;
}

int alsa_bridge_open(struct AlsaBridge *bridge,
                     const char *device,
                     enum alsa_bridge_format format,
                     uint32_t rate,
                     uint32_t channels) {
    if (!bridge || !device) return -EINVAL;
    if (bridge->is_open) return -EBUSY;

    /* Validate parameters */
    if (channels == 0 || channels > 8) return -EINVAL;
    if (rate < 8000 || rate > 192000) return -EINVAL;

    /* Determine direction from device path */
    size_t len = strlen(device);
    if (len == 0) return -EINVAL;
    bridge->is_playback = (device[len - 1] == 'p') ? 1 : 0;

    /* Open the device */
    int flags = bridge->is_playback ? O_WRONLY : O_RDONLY;
    bridge->fd = open(device, flags);
    if (bridge->fd < 0) {
        fprintf(stderr, "[alsa-bridge] Failed to open '%s': errno=%d\n",
                device, errno);
        return -errno;
    }

    /* Set format parameters */
    bridge->format = format;
    bridge->rate = rate;
    bridge->channels = channels;

    switch (format) {
        case ALSA_BRIDGE_FORMAT_U8:     bridge->bytes_per_sample = 1; break;
        case ALSA_BRIDGE_FORMAT_S16_LE: bridge->bytes_per_sample = 2; break;
        case ALSA_BRIDGE_FORMAT_S32_LE: bridge->bytes_per_sample = 4; break;
        default:                        bridge->bytes_per_sample = 2; break;
    }
    bridge->frame_size = bridge->channels * bridge->bytes_per_sample;

    /* Configure hardware parameters via ioctl */
    struct alsa_ioctl_hw_params hw;
    memset(&hw, 0, sizeof(hw));
    hw.sample_rate = rate;
    hw.channels    = (uint8_t)channels;
    hw.format      = (uint8_t)format;
    hw.buffer_size = bridge->buffer_frames;
    hw.period_size = bridge->period_frames;

    int ret = ioctl(bridge->fd, SNDRV_PCM_IOCTL_HW_PARAMS, &hw);
    if (ret < 0) {
        fprintf(stderr, "[alsa-bridge] HW_PARAMS ioctl failed: errno=%d\n",
                errno);
        /* Continue anyway -- the kernel may not support all ioctls yet */
    }

    /* Configure software parameters */
    struct alsa_ioctl_sw_params sw;
    memset(&sw, 0, sizeof(sw));
    sw.avail_min       = bridge->period_frames;
    sw.start_threshold = bridge->buffer_frames;
    sw.stop_threshold  = bridge->buffer_frames;

    ret = ioctl(bridge->fd, SNDRV_PCM_IOCTL_SW_PARAMS, &sw);
    if (ret < 0) {
        fprintf(stderr, "[alsa-bridge] SW_PARAMS ioctl failed: errno=%d\n",
                errno);
    }

    /* Prepare the device */
    ret = ioctl(bridge->fd, SNDRV_PCM_IOCTL_PREPARE, NULL);
    if (ret < 0) {
        fprintf(stderr, "[alsa-bridge] PREPARE ioctl failed: errno=%d\n",
                errno);
    }

    /* Calculate latency:
     * latency_us = buffer_frames * 1_000_000 / rate (integer division) */
    bridge->latency_us = (uint32_t)(
        ((uint64_t)bridge->buffer_frames * 1000000ULL) / rate
    );

    bridge->device_rate = rate;
    bridge->is_open = 1;

    fprintf(stderr, "[alsa-bridge] Opened '%s' (%s, fmt=%d, rate=%u, ch=%u, "
            "latency=%u us)\n",
            device,
            bridge->is_playback ? "playback" : "capture",
            format, rate, channels, bridge->latency_us);
    return 0;
}

void alsa_bridge_close(struct AlsaBridge *bridge) {
    if (!bridge || !bridge->is_open) return;

    /* Stop the device */
    ioctl(bridge->fd, SNDRV_PCM_IOCTL_STOP, NULL);

    close(bridge->fd);
    bridge->fd = -1;
    bridge->is_open = 0;

    /* Free resampling buffer */
    if (bridge->resample_buf) {
        free(bridge->resample_buf);
        bridge->resample_buf = NULL;
        bridge->resample_buf_size = 0;
    }

    fprintf(stderr, "[alsa-bridge] Closed device\n");
}

void alsa_bridge_destroy(struct AlsaBridge *bridge) {
    if (!bridge) return;
    if (bridge->is_open) alsa_bridge_close(bridge);
    free(bridge);
}

int32_t alsa_bridge_write(struct AlsaBridge *bridge,
                          const void *buffer,
                          uint32_t frames) {
    if (!bridge || !bridge->is_open || !bridge->is_playback) return -EINVAL;
    if (!buffer || frames == 0) return 0;

    const uint8_t *data = (const uint8_t *)buffer;
    uint32_t total_bytes = frames * bridge->frame_size;

    /* If resampling is needed (device_rate != requested rate), do it here */
    if (bridge->device_rate != bridge->rate) {
        /* Estimate output frame count */
        uint32_t out_max = (uint32_t)(
            ((uint64_t)frames * bridge->device_rate + bridge->rate - 1)
            / bridge->rate
        );
        uint32_t out_bytes = out_max * bridge->frame_size;

        /* Ensure resample buffer is large enough */
        if (bridge->resample_buf_size < out_bytes) {
            free(bridge->resample_buf);
            bridge->resample_buf = (uint8_t *)malloc(out_bytes);
            if (!bridge->resample_buf) {
                bridge->resample_buf_size = 0;
                return -ENOMEM;
            }
            bridge->resample_buf_size = out_bytes;
        }

        uint32_t out_frames;
        if (bridge->format == ALSA_BRIDGE_FORMAT_S16_LE) {
            out_frames = resample_s16_fixed(
                (const int16_t *)data, frames,
                (int16_t *)bridge->resample_buf, out_max,
                bridge->rate, bridge->device_rate,
                bridge->channels);
        } else if (bridge->format == ALSA_BRIDGE_FORMAT_S32_LE) {
            out_frames = resample_s32_fixed(
                (const int32_t *)data, frames,
                (int32_t *)bridge->resample_buf, out_max,
                bridge->rate, bridge->device_rate,
                bridge->channels);
        } else {
            /* U8: no resampling support, write directly */
            out_frames = frames;
            data = (const uint8_t *)buffer;
            goto direct_write;
        }

        data = bridge->resample_buf;
        total_bytes = out_frames * bridge->frame_size;
        frames = out_frames;
    }

direct_write:
    /* Start the device if not already running */
    ioctl(bridge->fd, SNDRV_PCM_IOCTL_START, NULL);

    /* Write to the kernel device */
    ssize_t written = write(bridge->fd, data, total_bytes);
    if (written < 0) {
        return (int32_t)(-errno);
    }

    uint32_t frames_written = (bridge->frame_size > 0)
                              ? (uint32_t)written / bridge->frame_size
                              : 0;
    return (int32_t)frames_written;
}

int32_t alsa_bridge_read(struct AlsaBridge *bridge,
                         void *buffer,
                         uint32_t frames) {
    if (!bridge || !bridge->is_open || bridge->is_playback) return -EINVAL;
    if (!buffer || frames == 0) return 0;

    uint32_t total_bytes = frames * bridge->frame_size;

    /* Start capture if not already running */
    ioctl(bridge->fd, SNDRV_PCM_IOCTL_START, NULL);

    ssize_t bytes_read = read(bridge->fd, buffer, total_bytes);
    if (bytes_read < 0) {
        return (int32_t)(-errno);
    }

    uint32_t frames_read = (bridge->frame_size > 0)
                           ? (uint32_t)bytes_read / bridge->frame_size
                           : 0;
    return (int32_t)frames_read;
}

uint32_t alsa_bridge_get_latency(const struct AlsaBridge *bridge) {
    if (!bridge || !bridge->is_open) return 0;
    return bridge->latency_us;
}

int alsa_bridge_set_buffer_size(struct AlsaBridge *bridge,
                                uint32_t period_frames,
                                uint32_t buffer_frames) {
    if (!bridge) return -EINVAL;
    if (period_frames == 0 || buffer_frames == 0) return -EINVAL;
    if (period_frames > buffer_frames) return -EINVAL;

    bridge->period_frames = period_frames;
    bridge->buffer_frames = buffer_frames;

    /* If device is open, reconfigure via ioctl */
    if (bridge->is_open) {
        struct alsa_ioctl_hw_params hw;
        memset(&hw, 0, sizeof(hw));
        hw.sample_rate = bridge->rate;
        hw.channels    = (uint8_t)bridge->channels;
        hw.format      = (uint8_t)bridge->format;
        hw.buffer_size = buffer_frames;
        hw.period_size = period_frames;

        int ret = ioctl(bridge->fd, SNDRV_PCM_IOCTL_HW_PARAMS, &hw);
        if (ret < 0) return -errno;

        /* Recalculate latency */
        bridge->latency_us = (uint32_t)(
            ((uint64_t)buffer_frames * 1000000ULL) / bridge->rate
        );
    }

    return 0;
}
