# VeridianOS Audio Subsystem

**Version:** v0.9.0 (Phase 7 Wave 5)
**Status:** Planned -- mixer pipeline, ring buffer transport, VirtIO-Sound backend

---

## Overview

VeridianOS implements a kernel-resident audio subsystem under `kernel/src/audio/`.
The design follows the same microkernel principles as the rest of the system:
capability-gated access, zero-copy where possible, and user-space shell commands
for interactive control.

Audio data flows through a fixed pipeline:

1. **Client** -- user-space or kernel task opens an `AudioStream`, writes PCM samples.
2. **Ring Buffer** -- lock-free SPSC ring buffer transports samples from producer to
   the mixer without copying.
3. **Mixer** -- sums all active streams using fixed-point 16.16 integer arithmetic,
   applies per-channel and master volume, clamps to `i16` range.
4. **Output Pipeline** -- pulls mixed frames at the device sample rate, tracks
   underruns, and feeds the hardware driver.
5. **VirtIO-Sound** -- PCI device `0x1AF4:0x1059`, virtqueue-based PCM output to
   the host audio backend.

All arithmetic is integer-only (no floating point in kernel context).

---

## Architecture Diagram

```
+-----------------------------------------------------------------------+
|                          User Space / Shell                            |
|   +------------------+  +-------------------+  +-------------------+  |
|   | play song.wav    |  | volume 75         |  | media-player      |  |
|   +--------+---------+  +---------+---------+  +---------+---------+  |
|            |                      |                      |            |
+============|======================|======================|============+
             |   syscall boundary   |                      |
+============|======================|======================|============+
|            v                      v                      v            |
|   +--------+---------+  +---------+---------+                         |
|   | AudioStream      |  | Volume Control    |   audio/client.rs      |
|   | create/play/stop |  | set_master_volume |                         |
|   +--------+---------+  +---------+---------+                         |
|            |                      |                                   |
|            v                      |                                   |
|   +--------+------------------+   |                                   |
|   | SharedAudioBuffer         |   |          audio/buffer.rs          |
|   | Lock-free SPSC ring buf   |   |                                   |
|   +--------+------------------+   |                                   |
|            |                      |                                   |
|            v                      v                                   |
|   +--------+----------------------+---------+                         |
|   |              Audio Mixer                |  audio/mixer.rs         |
|   |  Fixed-point 16.16, multi-stream sum    |                         |
|   |  Per-channel + master volume, clamp     |                         |
|   +--------+--------------------------------+                         |
|            |                                                          |
|            v                                                          |
|   +--------+---------------------------+                              |
|   |        Output Pipeline             |   audio/pipeline.rs          |
|   |  Pull loop, underrun tracking,     |                              |
|   |  drain support                     |                              |
|   +--------+---------------------------+                              |
|            |                                                          |
|            v                                                          |
|   +--------+---------------------------+                              |
|   |     VirtIO-Sound Driver            |   audio/virtio_sound.rs      |
|   |  PCI 0x1AF4:0x1059, virtqueue PCM  |                              |
|   +--------+---------------------------+                              |
|            |                                                          |
+============|==========================================================+
             |  MMIO / physical memory
+============|==========================================================+
|   +--------v---------------------------+                              |
|   |     Host Audio Backend             |                              |
|   |  (PulseAudio / ALSA / CoreAudio)   |                              |
|   +------------------------------------+                              |
+-----------------------------------------------------------------------+
```

---

## Audio Mixer (`audio/mixer.rs`)

The mixer is the central component. It pulls samples from every active stream,
sums them into a single output buffer, and applies volume scaling.

### Fixed-Point 16.16 Arithmetic

All volume and mixing calculations use `i32` values with 16 fractional bits.
This avoids soft-float overhead in kernel mode while preserving sub-dB
precision across the 0--100% volume range.

```rust
/// Fixed-point 16.16 representation.
/// 0x0001_0000 = 1.0, 0x0000_8000 = 0.5, 0x0000_0000 = 0.0
type Fixed16 = i32;

const FIXED_ONE: Fixed16 = 1 << 16;  // 65536

/// Convert a percentage (0..=100) to Fixed16.
fn percent_to_fixed(pct: u8) -> Fixed16 {
    (pct as i32 * FIXED_ONE) / 100
}

/// Multiply a PCM sample by a Fixed16 volume.
fn apply_volume(sample: i16, vol: Fixed16) -> i16 {
    let wide = (sample as i32) * vol;
    let result = wide >> 16;
    result.clamp(i16::MIN as i32, i16::MAX as i32) as i16
}
```

### Multi-Stream Mixing

Each active stream contributes samples to a `i32` accumulator per channel.
After all streams are summed, the result is clamped with saturation arithmetic
to prevent wrap-around distortion:

```rust
fn mix_frame(streams: &[&RingBuffer], master_vol: Fixed16) -> (i16, i16) {
    let (mut acc_l, mut acc_r): (i32, i32) = (0, 0);

    for stream in streams {
        if let Some((l, r)) = stream.pop_frame() {
            acc_l += apply_volume(l, stream.volume()) as i32;
            acc_r += apply_volume(r, stream.volume()) as i32;
        }
    }

    // Master volume + saturation clamp
    acc_l = (acc_l * master_vol as i64 >> 16) as i32;
    acc_r = (acc_r * master_vol as i64 >> 16) as i32;

    (
        acc_l.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
        acc_r.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
    )
}
```

The mixer supports mono (duplicated to stereo) and stereo streams at 44100 Hz
or 48000 Hz sample rates. Sample rate conversion, if needed, uses linear
interpolation with fixed-point coefficients.

---

## Ring Buffer Transport (`audio/buffer.rs`)

Audio data moves from producers (client streams) to the mixer via a lock-free
single-producer, single-consumer (SPSC) ring buffer. This eliminates mutex
contention on the audio hot path.

### SharedAudioBuffer

```rust
/// Lock-free ring buffer for PCM audio transport.
/// Capacity is always a power of two for mask-based indexing.
pub struct SharedAudioBuffer {
    buf: Vec<i16>,          // power-of-two capacity
    mask: usize,            // capacity - 1
    write_idx: AtomicUsize, // producer position
    read_idx: AtomicUsize,  // consumer position
}
```

Key properties:

| Property | Value |
|----------|-------|
| Capacity | 8192 samples (default), configurable power of two |
| Latency | ~93 ms at 44100 Hz stereo (8192 / 2 / 44100) |
| Ordering | `Acquire` on read, `Release` on write |
| Overflow | Oldest samples dropped (producer advances) |
| Underrun | Silence (zeros) returned to mixer |

The `push_samples()` and `pop_frame()` methods never block. The producer
(client write path) detects a full buffer by comparing indices and either
drops samples or returns `Err(BufferFull)` depending on the stream's overflow
policy.

---

## Client API (`audio/client.rs`)

User-space processes and kernel tasks interact with the audio subsystem through
the `AudioStream` lifecycle:

### Stream Lifecycle

```
create() --> Idle
  |
play()   --> Playing  <--+
  |                      |
pause()  --> Paused   ---+
  |
stop()   --> Stopped
  |
destroy()--> (freed)
```

### Core API

```rust
/// Create a new audio stream.
/// Returns a stream ID for subsequent operations.
pub fn audio_stream_create(
    sample_rate: u32,   // 44100 or 48000
    channels: u8,       // 1 (mono) or 2 (stereo)
    format: SampleFormat, // S16LE
) -> Result<StreamId, AudioError>;

/// Write PCM samples to a stream's ring buffer.
pub fn audio_stream_write(
    id: StreamId,
    samples: &[i16],
) -> Result<usize, AudioError>;

/// Control playback state.
pub fn audio_stream_play(id: StreamId) -> Result<(), AudioError>;
pub fn audio_stream_pause(id: StreamId) -> Result<(), AudioError>;
pub fn audio_stream_stop(id: StreamId) -> Result<(), AudioError>;

/// Set per-stream volume (0..=100).
pub fn audio_stream_set_volume(id: StreamId, pct: u8) -> Result<(), AudioError>;

/// Release stream resources.
pub fn audio_stream_destroy(id: StreamId) -> Result<(), AudioError>;
```

Each stream holds a `SharedAudioBuffer` and metadata (sample rate, channel
count, volume). The mixer reads from all streams in `Playing` state during
each output cycle.

---

## WAV Parser (`audio/wav.rs`)

The WAV parser handles RIFF/WAVE files for the `play` shell command. It
validates the file structure and extracts raw PCM data without heap allocation
beyond the sample buffer.

### RIFF/WAVE Layout

```
Offset  Size  Field
------  ----  -----
0       4     "RIFF" magic
4       4     File size - 8 (little-endian)
8       4     "WAVE" format
12      4     "fmt " chunk ID
16      4     fmt chunk size (16 for PCM)
20      2     Audio format (1 = PCM)
22      2     Channel count
24      4     Sample rate
28      4     Byte rate (sample_rate * channels * bits/8)
32      2     Block align (channels * bits/8)
34      2     Bits per sample (8, 16, or 24)
36      4     "data" chunk ID
40      4     Data size
44      ...   PCM sample data
```

### Supported Formats

| Bits/Sample | Storage | Range | Conversion |
|-------------|---------|-------|------------|
| 8 | `u8` | 0..255 | `(sample as i16 - 128) << 8` |
| 16 | `i16` LE | -32768..32767 | Direct use |
| 24 | 3 bytes LE | -8388608..8388607 | `sample >> 8` (truncate to 16-bit) |

The parser skips non-"fmt " and non-"data" chunks (e.g., LIST, INFO) by
reading the chunk size and advancing the offset. This allows playback of
WAV files produced by common tools without requiring exact chunk ordering.

---

## Output Pipeline (`audio/pipeline.rs`)

The output pipeline runs a periodic processing loop that pulls mixed audio
from the mixer and feeds it to the hardware driver.

### Processing Loop

```rust
/// Called at the device's interrupt rate or by a timer tick.
fn pipeline_tick(pipeline: &mut OutputPipeline) {
    let frames_needed = pipeline.driver.available_frames();

    for _ in 0..frames_needed {
        let (l, r) = pipeline.mixer.mix_frame();
        pipeline.output_buf.push(l);
        pipeline.output_buf.push(r);
    }

    if pipeline.output_buf.len() >= pipeline.batch_size {
        pipeline.driver.submit_pcm(&pipeline.output_buf);
        pipeline.output_buf.clear();
    }
}
```

### Underrun Tracking

When the mixer returns silence because all streams are empty, the pipeline
increments an underrun counter. Persistent underruns (>10 consecutive silent
frames) trigger a log warning. The counter is exposed through the `audio_stats`
diagnostic interface.

### Drain Support

When the last active stream calls `stop()`, the pipeline enters drain mode:
it continues pulling from the mixer until all buffered samples have been
submitted to the hardware, then signals completion. This prevents audio
truncation at the end of playback.

---

## VirtIO-Sound Driver (`audio/virtio_sound.rs`)

The VirtIO-Sound driver provides paravirtualized audio output using the
VirtIO 1.2 sound device specification.

### Device Identification

| Field | Value |
|-------|-------|
| Vendor ID | `0x1AF4` (Red Hat / VirtIO) |
| Device ID | `0x1059` (sound device, transitional) |
| PCI class | `0x0401` (Multimedia audio controller) |

### Virtqueue Layout

| Queue | Index | Purpose |
|-------|-------|---------|
| controlq | 0 | Configuration, stream setup/teardown |
| eventq | 1 | Asynchronous device events |
| txq | 2 | PCM data output (host playback) |
| rxq | 3 | PCM data input (host capture, future) |

### Stream Configuration

The driver negotiates PCM parameters with the host during stream setup:

```rust
struct VirtioSndPcmSetParams {
    hdr: VirtioSndHdr,          // VIRTIO_SND_R_PCM_SET_PARAMS
    buffer_bytes: u32,          // total buffer size
    period_bytes: u32,          // interrupt interval
    features: u32,              // 0 for basic PCM
    channels: u8,               // 1 or 2
    format: u8,                 // VIRTIO_SND_PCM_FMT_S16 = 2
    rate: u8,                   // VIRTIO_SND_PCM_RATE_44100 = 8
    _padding: u8,
}
```

PCM output uses `txq`: the driver posts buffers containing interleaved `i16`
samples. The host consumes them at the negotiated sample rate and signals
completion through used-buffer notifications, at which point the driver reclaims
the buffer for reuse.

### QEMU Integration

QEMU exposes VirtIO-Sound with:

```bash
qemu-system-x86_64 ... \
    -device virtio-sound-pci,audiodev=snd0 \
    -audiodev pa,id=snd0    # or -audiodev alsa,id=snd0
```

The driver auto-detects the device during PCI enumeration in
`drivers::pci::scan()` and registers it with the audio pipeline.

---

## Shell Commands

### `play <file.wav>`

Parses the WAV file from the VFS, creates an `AudioStream` with matching
parameters, writes all PCM data into the ring buffer, and starts playback.
Blocks until drain completes or the user presses Ctrl+C.

```
root@veridian:/# play /sounds/startup.wav
Playing: startup.wav (44100 Hz, 16-bit, stereo, 3.2s)
[========================================] 100%
```

### `volume <0-100>`

Sets the master mixer volume. Without an argument, prints the current level.

```
root@veridian:/# volume
Master volume: 75%

root@veridian:/# volume 50
Master volume: 50%
```

---

## Module Layout

```
kernel/src/audio/
    mod.rs              -- Public API, init(), AudioError
    mixer.rs            -- Fixed-point mixer, per-channel + master volume
    buffer.rs           -- SharedAudioBuffer (lock-free SPSC ring)
    client.rs           -- AudioStream lifecycle, stream registry
    wav.rs              -- RIFF/WAVE parser, PCM extraction
    pipeline.rs         -- Output processing loop, underrun/drain
    virtio_sound.rs     -- VirtIO-Sound PCI driver, virtqueue PCM
```

---

## Configuration Constants

| Constant | Default | Description |
|----------|---------|-------------|
| `MAX_STREAMS` | 16 | Maximum simultaneous audio streams |
| `RING_BUFFER_CAPACITY` | 8192 | Samples per ring buffer (power of two) |
| `DEFAULT_SAMPLE_RATE` | 44100 | Hz, also supports 48000 |
| `DEFAULT_CHANNELS` | 2 | Stereo output |
| `MASTER_VOLUME_DEFAULT` | 75 | Initial master volume percentage |
| `UNDERRUN_WARN_THRESHOLD` | 10 | Consecutive silent frames before warning |
| `PIPELINE_BATCH_SIZE` | 1024 | Samples per hardware submission |
