/*
 * VeridianOS -- pw-alsa-bridge.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Bridge layer between PipeWire streams and the VeridianOS kernel ALSA
 * subsystem.  Translates PipeWire buffer semantics into ALSA PCM
 * read/write operations on /dev/snd/pcmC{card}D{device}{p|c} devices.
 *
 * Provides integer-only 16.16 fixed-point resampling for rate conversion
 * (e.g. 44100 -> 48000 Hz) without requiring an FPU.
 */

#ifndef PW_ALSA_BRIDGE_H
#define PW_ALSA_BRIDGE_H

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Types                                                                     */
/* ========================================================================= */

/** Opaque handle to an ALSA bridge instance */
struct AlsaBridge;

/** Sample format (mirrors ALSA / PipeWire format IDs) */
enum alsa_bridge_format {
    ALSA_BRIDGE_FORMAT_U8     = 0,
    ALSA_BRIDGE_FORMAT_S16_LE = 1,
    ALSA_BRIDGE_FORMAT_S32_LE = 2
};

/* ========================================================================= */
/* Lifecycle                                                                 */
/* ========================================================================= */

/**
 * Create a new ALSA bridge instance.
 * Returns NULL on allocation failure.
 */
struct AlsaBridge *alsa_bridge_new(void);

/**
 * Open a kernel ALSA PCM device.
 *
 * @param bridge  Bridge instance.
 * @param device  Device path, e.g. "/dev/snd/pcmC0D0p" (playback)
 *                or "/dev/snd/pcmC0D0c" (capture).
 * @param format  Sample format.
 * @param rate    Sample rate in Hz (8000-192000).
 * @param channels Number of interleaved channels (1-8).
 * @return 0 on success, negative errno on failure.
 */
int alsa_bridge_open(struct AlsaBridge *bridge,
                     const char *device,
                     enum alsa_bridge_format format,
                     uint32_t rate,
                     uint32_t channels);

/**
 * Close the currently open ALSA device.
 * Safe to call even if no device is open.
 */
void alsa_bridge_close(struct AlsaBridge *bridge);

/**
 * Destroy the bridge and free all resources.
 */
void alsa_bridge_destroy(struct AlsaBridge *bridge);

/* ========================================================================= */
/* I/O                                                                       */
/* ========================================================================= */

/**
 * Write interleaved audio frames to the playback device.
 *
 * @param bridge Bridge instance (must be open for playback).
 * @param buffer Pointer to interleaved sample data.
 * @param frames Number of frames to write.
 * @return Number of frames actually written, or negative errno.
 */
int32_t alsa_bridge_write(struct AlsaBridge *bridge,
                          const void *buffer,
                          uint32_t frames);

/**
 * Read interleaved audio frames from the capture device.
 *
 * @param bridge Bridge instance (must be open for capture).
 * @param buffer Destination buffer for interleaved sample data.
 * @param frames Maximum number of frames to read.
 * @return Number of frames actually read, or negative errno.
 */
int32_t alsa_bridge_read(struct AlsaBridge *bridge,
                         void *buffer,
                         uint32_t frames);

/* ========================================================================= */
/* Configuration                                                             */
/* ========================================================================= */

/**
 * Query the current end-to-end latency in microseconds.
 *
 * @return Latency in microseconds (integer), or 0 if unknown.
 */
uint32_t alsa_bridge_get_latency(const struct AlsaBridge *bridge);

/**
 * Set the ALSA period/buffer size.
 *
 * @param bridge Bridge instance.
 * @param period_frames Desired period size in frames.
 * @param buffer_frames Desired buffer size in frames (>= period_frames).
 * @return 0 on success, negative errno on failure.
 */
int alsa_bridge_set_buffer_size(struct AlsaBridge *bridge,
                                uint32_t period_frames,
                                uint32_t buffer_frames);

#ifdef __cplusplus
}  /* extern "C" */
#endif

#endif /* PW_ALSA_BRIDGE_H */
