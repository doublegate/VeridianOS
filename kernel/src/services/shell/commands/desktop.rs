//! Desktop/GUI and audio commands.

#![allow(unused_variables, unused_assignments)]

use alloc::{format, string::String};

use crate::services::shell::{BuiltinCommand, CommandResult, Shell};

// ============================================================================
// Desktop / GUI Commands
// ============================================================================

pub(in crate::services::shell) struct StartGuiCommand;
impl BuiltinCommand for StartGuiCommand {
    fn name(&self) -> &str {
        "startgui"
    }
    fn description(&self) -> &str {
        "Start the graphical desktop environment"
    }
    fn execute(&self, _args: &[String], _shell: &Shell) -> CommandResult {
        crate::desktop::renderer::start_desktop();
        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct WinfoCommand;
impl BuiltinCommand for WinfoCommand {
    fn name(&self) -> &str {
        "winfo"
    }
    fn description(&self) -> &str {
        "Wayland/desktop information"
    }
    fn execute(&self, _args: &[String], _shell: &Shell) -> CommandResult {
        crate::println!("=== Desktop Information ===");
        crate::desktop::wayland::with_display(|display| {
            let (w, h) = display.wl_compositor.output_size();
            crate::println!("Compositor: {}x{}", w, h);
            crate::println!("Surfaces:   {}", display.wl_compositor.surface_count());
        });
        crate::desktop::window_manager::with_window_manager(|wm| {
            let windows = wm.get_all_windows();
            let focused = wm.get_focused_window_id();
            let workspace = wm.get_active_workspace();
            crate::println!("Windows:    {}", windows.len());
            crate::println!("Workspace:  {}", workspace);
            crate::println!("Focused:    {:?}", focused);
            if !windows.is_empty() {
                crate::println!("\nWindow List:");
                crate::println!(
                    "  {:<6} {:<20} {:<12} {}",
                    "ID",
                    "Title",
                    "Size",
                    "Position"
                );
                for w in &windows {
                    crate::println!(
                        "  {:<6} {:<20} {}x{:<8} ({},{})",
                        w.id,
                        w.title_str(),
                        w.width,
                        w.height,
                        w.x,
                        w.y
                    );
                }
            }
        });
        CommandResult::Success(0)
    }
}

// ============================================================================
// Audio Commands
// ============================================================================

pub(in crate::services::shell) struct PlayCommand;
impl BuiltinCommand for PlayCommand {
    fn name(&self) -> &str {
        "play"
    }
    fn description(&self) -> &str {
        "Play a WAV audio file"
    }
    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            crate::println!("Usage: play <file.wav>");
            crate::println!("  Play a PCM WAV file through the audio subsystem");
            return CommandResult::Error(String::from("missing filename"));
        }

        let path = &args[0];

        // Read the file from VFS
        let file_data = match crate::fs::read_file(path) {
            Ok(data) => data,
            Err(e) => {
                crate::println!("play: cannot open '{}': {:?}", path, e);
                return CommandResult::Error(format!("cannot open '{}'", path));
            }
        };

        // Parse WAV header
        let wav = match crate::audio::wav::WavFile::parse(&file_data) {
            Ok(w) => w,
            Err(e) => {
                crate::println!("play: invalid WAV file '{}': {:?}", path, e);
                return CommandResult::Error(format!("invalid WAV: {:?}", e));
            }
        };

        crate::println!(
            "Playing: {} ({} Hz, {} ch, {}-bit, {} ms)",
            path,
            wav.sample_rate,
            wav.num_channels,
            wav.bits_per_sample,
            wav.duration_ms()
        );

        // Convert PCM data to S16Le for the mixer
        let pcm_data = wav.sample_data(&file_data);
        let samples: alloc::vec::Vec<i16> = match wav.bits_per_sample {
            8 => crate::audio::wav::convert_u8_to_s16(pcm_data),
            16 => {
                // Already S16Le: reinterpret bytes as i16 slice
                let num_samples = pcm_data.len() / 2;
                let mut s = alloc::vec::Vec::with_capacity(num_samples);
                for i in 0..num_samples {
                    let lo = pcm_data[i * 2] as u16;
                    let hi = pcm_data[i * 2 + 1] as u16;
                    s.push((lo | (hi << 8)) as i16);
                }
                s
            }
            24 => crate::audio::wav::convert_s24_to_s16(pcm_data),
            32 => crate::audio::wav::convert_s32_to_s16(pcm_data),
            other => {
                crate::println!("play: unsupported bit depth: {}", other);
                return CommandResult::Error(format!("unsupported bit depth: {}", other));
            }
        };

        // Create an audio stream, write samples, and play
        let config = wav.to_audio_config();
        let result =
            crate::audio::client::with_client(|client: &mut crate::audio::client::AudioClient| {
                let stream_id = client.create_stream(path, config)?;
                client.write_samples(stream_id, &samples)?;
                client.play(stream_id)?;
                crate::println!(
                    "Queued {} samples ({} ms) on stream {}",
                    samples.len(),
                    wav.duration_ms(),
                    stream_id.as_u32()
                );
                Ok::<(), crate::error::KernelError>(())
            });

        match result {
            Ok(Ok(())) => CommandResult::Success(0),
            Ok(Err(e)) => {
                crate::println!("play: audio error: {:?}", e);
                CommandResult::Error(format!("audio error: {:?}", e))
            }
            Err(e) => {
                crate::println!("play: audio subsystem not initialized: {:?}", e);
                CommandResult::Error(String::from("audio not initialized"))
            }
        }
    }
}

pub(in crate::services::shell) struct VolumeCommand;
impl BuiltinCommand for VolumeCommand {
    fn name(&self) -> &str {
        "volume"
    }
    fn description(&self) -> &str {
        "Set audio volume (0-100)"
    }
    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            // Show current volume
            let result =
                crate::audio::mixer::with_mixer(|mixer: &mut crate::audio::mixer::AudioMixer| {
                    let vol = mixer.get_master_volume();
                    let pct = (vol as u32 * 100) / 65535;
                    crate::println!("Master volume: {}% (raw: {})", pct, vol);
                });
            if let Err(e) = result {
                crate::println!("volume: audio not initialized: {:?}", e);
                return CommandResult::Error(String::from("audio not initialized"));
            }
            return CommandResult::Success(0);
        }

        // Parse volume level
        let level_str = &args[0];
        let level: u32 = match level_str.parse() {
            Ok(v) => v,
            Err(_) => {
                crate::println!("Usage: volume <0-100> [stream_id]");
                crate::println!("  Set master volume or per-stream volume");
                return CommandResult::Error(String::from("invalid volume"));
            }
        };

        if level > 100 {
            crate::println!("volume: value must be 0-100");
            return CommandResult::Error(String::from("volume out of range"));
        }

        // Convert 0-100 to 0-65535
        let raw_vol = ((level * 65535) / 100) as u16;

        if args.len() >= 2 {
            // Per-stream volume
            let stream_id_val: u32 = match args[1].parse() {
                Ok(v) => v,
                Err(_) => {
                    crate::println!("volume: invalid stream ID");
                    return CommandResult::Error(String::from("invalid stream ID"));
                }
            };

            let stream_id = crate::audio::client::AudioStreamId(stream_id_val);
            let result = crate::audio::client::with_client(
                |client: &mut crate::audio::client::AudioClient| {
                    client.set_volume(stream_id, raw_vol)
                },
            );

            match result {
                Ok(Ok(())) => {
                    crate::println!("Stream {} volume set to {}%", stream_id_val, level);
                    CommandResult::Success(0)
                }
                Ok(Err(e)) => {
                    crate::println!("volume: {:?}", e);
                    CommandResult::Error(format!("{:?}", e))
                }
                Err(e) => {
                    crate::println!("volume: audio not initialized: {:?}", e);
                    CommandResult::Error(String::from("audio not initialized"))
                }
            }
        } else {
            // Master volume
            let result =
                crate::audio::mixer::with_mixer(|mixer: &mut crate::audio::mixer::AudioMixer| {
                    mixer.set_master_volume(raw_vol);
                });

            match result {
                Ok(()) => {
                    crate::println!("Master volume set to {}%", level);
                    CommandResult::Success(0)
                }
                Err(e) => {
                    crate::println!("volume: audio not initialized: {:?}", e);
                    CommandResult::Error(String::from("audio not initialized"))
                }
            }
        }
    }
}
