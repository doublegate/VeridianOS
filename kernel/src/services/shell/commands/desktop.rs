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
    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        use crate::desktop::session_config::{self, SessionPreference};

        // Parse explicit session argument: "startgui builtin" or "startgui plasma"
        let preference = if !args.is_empty() {
            match args[0].as_str() {
                "builtin" | "default" => SessionPreference::Builtin,
                "plasma" | "kde" => SessionPreference::Plasma,
                "help" | "--help" | "-h" => {
                    crate::println!("Usage: startgui [builtin|plasma]");
                    crate::println!("  builtin  - Launch built-in desktop environment");
                    crate::println!("  plasma   - Launch KDE Plasma 6 desktop session");
                    crate::println!("  (none)   - Read preference from /etc/veridian/session.conf");
                    return CommandResult::Success(0);
                }
                other => {
                    crate::println!("startgui: unknown session type '{}'", other);
                    crate::println!("Usage: startgui [builtin|plasma]");
                    return CommandResult::Error(format!("unknown session type '{}'", other));
                }
            }
        } else {
            // Read from config file
            session_config::read_session_preference()
        };

        match preference {
            SessionPreference::Plasma => {
                if session_config::kde_binaries_available() {
                    crate::println!("[startgui] Starting KDE Plasma 6 session...");
                    crate::desktop::kde_session::start_kde_session();
                } else {
                    crate::println!(
                        "[startgui] KDE not available, falling back to built-in desktop"
                    );
                    crate::desktop::renderer::start_desktop();
                }
            }
            SessionPreference::Builtin => {
                crate::println!("[startgui] Starting built-in desktop environment...");
                crate::desktop::renderer::start_desktop();
            }
        }

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

pub(in crate::services::shell) struct BrowserCommand;
impl BuiltinCommand for BrowserCommand {
    fn name(&self) -> &str {
        "browser"
    }
    fn description(&self) -> &str {
        "Open the web browser"
    }
    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        let url = if args.is_empty() {
            "veridian://start"
        } else {
            &args[0]
        };

        // Check desktop status via window manager
        let window_count =
            crate::desktop::window_manager::with_window_manager(|wm| wm.get_all_windows().len());
        match window_count {
            Some(count) => {
                crate::println!("=== Browser ===");
                crate::println!("URL:     {}", url);
                crate::println!("Desktop: active ({} window(s) open)", count);
                crate::println!("Launch the browser from the desktop app launcher.");
            }
            None => {
                crate::println!("Desktop is not running. Start it with 'startgui' first.");
            }
        }

        // Report browser engine capabilities
        crate::println!("\nBrowser Engine Capabilities:");
        crate::println!("  HTML5 parser:   html_tokenizer + tree_builder");
        crate::println!("  CSS engine:     css_parser + style + layout + flexbox");
        crate::println!("  Rendering:      paint + incremental");
        crate::println!("  JavaScript:     js_vm + js_gc + js_compiler (JIT)");
        crate::println!("  DOM bindings:   dom + dom_bindings + forms + events");
        crate::println!("  Tabs:           process-isolated (tab_isolation)");

        CommandResult::Success(0)
    }
}

// ============================================================================
// Desktop Enhancement Commands
// ============================================================================

pub(in crate::services::shell) struct ScreenshotCommand;
impl BuiltinCommand for ScreenshotCommand {
    fn name(&self) -> &str {
        "screenshot"
    }
    fn description(&self) -> &str {
        "Capture a screenshot"
    }
    fn execute(&self, _args: &[String], _shell: &Shell) -> CommandResult {
        // Check framebuffer dimensions via Wayland compositor
        let display_size = crate::desktop::wayland::with_display(|d| d.wl_compositor.output_size());

        let (width, height) = match display_size {
            Some((w, h)) if w > 0 && h > 0 => (w, h),
            _ => {
                crate::println!("screenshot: no framebuffer available (desktop not running?)");
                return CommandResult::Error(String::from("no framebuffer"));
            }
        };

        crate::println!("Capturing screenshot ({}x{})...", width, height);

        // Build a minimal BMP file (BITMAPINFOHEADER, 32bpp, top-down)
        let row_bytes = width * 4; // 32bpp, no padding needed
        let pixel_data_size = row_bytes * height;
        let file_header_size: u32 = 14;
        let dib_header_size: u32 = 40;
        let pixel_offset = file_header_size + dib_header_size;
        let file_size = pixel_offset + pixel_data_size;

        let mut bmp = alloc::vec::Vec::with_capacity(file_size as usize);

        // -- 14-byte BMP file header --
        bmp.push(b'B');
        bmp.push(b'M');
        bmp.extend_from_slice(&file_size.to_le_bytes()); // file size
        bmp.extend_from_slice(&[0u8; 4]); // reserved
        bmp.extend_from_slice(&pixel_offset.to_le_bytes()); // pixel data offset

        // -- 40-byte DIB header (BITMAPINFOHEADER) --
        bmp.extend_from_slice(&dib_header_size.to_le_bytes()); // header size
        bmp.extend_from_slice(&(width as i32).to_le_bytes()); // width
        bmp.extend_from_slice(&(-(height as i32)).to_le_bytes()); // height (negative = top-down)
        bmp.extend_from_slice(&1u16.to_le_bytes()); // planes
        bmp.extend_from_slice(&32u16.to_le_bytes()); // bpp
        bmp.extend_from_slice(&[0u8; 24]); // compression + rest zeros

        // Fill pixel area with black (placeholder scanlines)
        bmp.resize(file_size as usize, 0u8);

        // Write via VFS
        match crate::fs::write_file("/tmp/screenshot.bmp", &bmp) {
            Ok(bytes) => {
                crate::println!("Screenshot saved to /tmp/screenshot.bmp ({} bytes)", bytes);
                CommandResult::Success(0)
            }
            Err(e) => {
                crate::println!("screenshot: failed to write file: {:?}", e);
                CommandResult::Error(format!("write failed: {:?}", e))
            }
        }
    }
}

pub(in crate::services::shell) struct NotifyCommand;
impl BuiltinCommand for NotifyCommand {
    fn name(&self) -> &str {
        "notify"
    }
    fn description(&self) -> &str {
        "Send a desktop notification"
    }
    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            crate::println!("Usage: notify <message>");
            return CommandResult::Success(1);
        }
        let msg = args.join(" ");
        let _id = crate::desktop::notification::notify(
            &msg,
            "",
            crate::desktop::notification::NotificationUrgency::Normal,
            "shell",
        );
        crate::println!("Notification: {}", msg);
        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct ThemeCommand;
impl BuiltinCommand for ThemeCommand {
    fn name(&self) -> &str {
        "theme"
    }
    fn description(&self) -> &str {
        "Manage desktop themes"
    }
    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        use crate::desktop::desktop_ext::theme::{ThemeManager, ThemePreset};

        if args.is_empty() {
            let manager = ThemeManager::new();
            let current = manager.current_preset();
            crate::println!("Current theme: {:?}", current);
            crate::println!("Usage: theme list|set <name>");
            return CommandResult::Success(0);
        }
        match args[0].as_str() {
            "list" => {
                let manager = ThemeManager::new();
                let current = manager.current_preset();
                let presets = [
                    ("Light", ThemePreset::Light),
                    ("Dark", ThemePreset::Dark),
                    ("SolarizedDark", ThemePreset::SolarizedDark),
                    ("SolarizedLight", ThemePreset::SolarizedLight),
                    ("Nord", ThemePreset::Nord),
                    ("Dracula", ThemePreset::Dracula),
                ];
                crate::println!("Available themes:");
                for (i, (name, preset)) in presets.iter().enumerate() {
                    let marker = if *preset == current { " (active)" } else { "" };
                    crate::println!("  {}. {}{}", i + 1, name, marker);
                }
            }
            "set" => {
                if args.len() < 2 {
                    crate::println!("Usage: theme set <name>");
                    crate::println!(
                        "Names: Light, Dark, SolarizedDark, SolarizedLight, Nord, Dracula"
                    );
                } else {
                    let preset = match args[1].to_lowercase().as_str() {
                        "light" => Some(ThemePreset::Light),
                        "dark" => Some(ThemePreset::Dark),
                        "solarizeddark" | "solarized-dark" => Some(ThemePreset::SolarizedDark),
                        "solarizedlight" | "solarized-light" => Some(ThemePreset::SolarizedLight),
                        "nord" => Some(ThemePreset::Nord),
                        "dracula" => Some(ThemePreset::Dracula),
                        _ => None,
                    };
                    match preset {
                        Some(p) => {
                            let mut manager = ThemeManager::new();
                            manager.set_theme(p);
                            crate::println!("Theme set to: {:?}", p);
                        }
                        None => {
                            crate::println!("theme: unknown theme '{}'", args[1]);
                            crate::println!(
                                "Names: Light, Dark, SolarizedDark, SolarizedLight, Nord, Dracula"
                            );
                            return CommandResult::Error(format!("unknown theme '{}'", args[1]));
                        }
                    }
                }
            }
            _ => {
                let manager = ThemeManager::new();
                let current = manager.current_preset();
                crate::println!("Current theme: {:?}", current);
                crate::println!("Usage: theme list|set <name>");
            }
        }
        CommandResult::Success(0)
    }
}
