use libmpv2::{events::Event, Error as LibMpvError, Mpv};
use serde_json::{self, json, Value as JsonValue};
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;

#[derive(Debug)]
#[allow(dead_code)]
pub enum Error {
    InitError(String),
    PropertyError(String, i32),
    CommandError(String, i32),
    MutexError(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::InitError(msg) => write!(f, "Initialization error: {}", msg),
            Error::PropertyError(msg, code) => write!(f, "Property error ({}): {}", code, msg),
            Error::CommandError(msg, code) => write!(f, "Command error ({}): {}", code, msg),
            Error::MutexError(msg) => write!(f, "Mutex lock error: {}", msg),
        }
    }
}

impl<T> From<PoisonError<T>> for Error {
    fn from(err: PoisonError<T>) -> Self {
        Error::MutexError(format!("Mutex was poisoned: {}", err))
    }
}

impl From<LibMpvError> for Error {
    fn from(err: LibMpvError) -> Self {
        Error::PropertyError(err.to_string(), -1)
    }
}

pub struct MpvPlayer {
    handle: Arc<Mutex<Mpv>>,
    quit_flag: Arc<AtomicBool>,
    offset_seconds: Arc<AtomicI64>, // Store offset in milliseconds internally
}

unsafe impl Send for MpvPlayer {}
unsafe impl Sync for MpvPlayer {}

#[allow(dead_code)]
impl MpvPlayer {
    pub fn new() -> Result<(Self, mpsc::Receiver<()>), Error> {
        let mpv = Mpv::new().map_err(|e| Error::InitError(e.to_string()))?;

        // Create a channel for exit notification
        let (_exit_sender, exit_receiver) = mpsc::channel();
        let quit_flag = Arc::new(AtomicBool::new(false));

        // Set essential properties for window visibility and UI
        let essential_opts = [
            ("force-window", "yes"),
            ("input-default-bindings", "yes"),
            ("title", "Alien - Sync videos with Moon Animator"),
            ("keep-open", "yes"),
            ("keep-open-pause", "yes"),
            // UI and OSC settings
            ("ontop", "yes"),
            ("osc", "yes"),
            ("osd-level", "2"),
            // Loop settings
            ("loop-file", "inf"),     // Set infinite looping by default
            ("loop-playlist", "inf"), // Also loop playlists
            // ("window-progress-style", "bar"),
            // ("background", "#121212"),
            // ("force-window-colors", "yes"),
            // YouTube support
            ("script-opts", "ytdl_hook-ytdl_path=yt-dlp"),
            ("ytdl", "yes"),
            ("ytdl-format", "bestvideo[height<=?1080]+bestaudio/best"),
            ("ytdl-raw-options", "no-check-certificate="),
        ];

        // Get the scripts directory path
        let scripts_dir = if cfg!(debug_assertions) {
            // In debug mode, use the manifest directory
            let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
            std::path::Path::new(&manifest_dir).join("scripts")
        } else {
            // In release mode, use the executable's directory
            let exe_dir = std::env::current_exe()
                .expect("Failed to get executable path")
                .parent()
                .expect("Failed to get executable directory")
                .to_path_buf();
            let scripts_dir = exe_dir.join("resources").join("scripts");
            println!("Release mode scripts path: {:?}", scripts_dir);
            scripts_dir
        };

        // Check if scripts directory exists
        if !scripts_dir.exists() {
            println!("Warning: Scripts directory not found at {:?}", scripts_dir);
        } else {
            println!("Found scripts directory at {:?}", scripts_dir);
            // Collect all Lua scripts
            let mut script_paths = Vec::new();
            match std::fs::read_dir(&scripts_dir) {
                Ok(entries) => {
                    for entry in entries {
                        if let Ok(entry) = entry {
                            if entry.file_name().to_string_lossy().ends_with(".lua") {
                                let script_path = entry.path();
                                if script_path.exists() {
                                    println!("Found script: {:?}", script_path);
                                    script_paths.push(script_path);
                                }
                            }
                        }
                    }
                }
                Err(e) => println!("Error reading scripts directory: {}", e),
            }

            // If we found scripts, load them all at once
            if !script_paths.is_empty() {
                let script_list = script_paths
                    .iter()
                    .map(|p| p.to_str().unwrap())
                    .collect::<Vec<_>>()
                    .join(";"); // Use semicolon for Windows

                println!("Loading scripts: {}", script_list);
                match mpv.set_property("scripts", script_list.as_str()) {
                    Ok(_) => println!("Successfully loaded all scripts"),
                    Err(e) => {
                        println!("Failed to load scripts: {}", e);
                        // Try to get more detailed error information
                        if let Ok(error_msg) = mpv.get_property::<String>("error-string") {
                            println!("MPV error details: {}", error_msg);
                        }
                    }
                }
            } else {
                println!("No .lua scripts found in {:?}", scripts_dir);
            }
        }

        for (key, value) in essential_opts {
            mpv.set_property(key, value).map_err(|_e| {
                Error::PropertyError(format!("Failed to set property {} to {}", key, value), -1)
            })?;
        }

        // Create a default offset value
        // Using zero offset by default - user can adjust as needed
        let offset_seconds = Arc::new(AtomicI64::new(0));
        println!("Started with zero playback offset, can be adjusted via UI");

        Ok((
            MpvPlayer {
                handle: Arc::new(Mutex::new(mpv)),
                quit_flag,
                offset_seconds,
            },
            exit_receiver,
        ))
    }

    pub fn set_property(&self, name: &str, value: &str) -> Result<(), Error> {
        let handle = self.handle.lock()?;

        // Special handling for pause property which is critical for syncing
        if name == "pause" {
            // For pause property, try as boolean first
            let bool_value = match value {
                "yes" | "true" => true,
                "no" | "false" => false,
                _ => {
                    return Err(Error::PropertyError(
                        format!("Invalid pause value: {}", value),
                        -1,
                    ))
                }
            };

            return handle.set_property(name, bool_value).map_err(|e| {
                Error::PropertyError(
                    format!("Failed to set property {} to {}: {}", name, value, e),
                    -1,
                )
            });
        }

        // Standard property handling for other properties
        handle.set_property(name, value).map_err(|e| {
            Error::PropertyError(
                format!("Failed to set property {} to {}: {}", name, value, e),
                -1,
            )
        })
    }

    pub fn command(&self, cmd: &str, args: &[&str]) -> Result<(), Error> {
        let handle = self.handle.lock()?;

        // Special case for play/pause commands that are critical for syncing
        if cmd == "set" && args.len() >= 2 && args[0] == "pause" {
            let pause_value = match args[1] {
                "yes" | "true" => true,
                "no" | "false" => false,
                _ => {
                    return Err(Error::CommandError(
                        format!("Invalid pause value: {}", args[1]),
                        -1,
                    ))
                }
            };

            return handle.set_property("pause", pause_value).map_err(|e| {
                Error::CommandError(format!("Failed to set pause to {}: {}", args[1], e), -1)
            });
        }

        handle.command(cmd, args).map_err(|e| {
            Error::CommandError(format!("Failed to execute command {}: {}", cmd, e), -1)
        })
    }

    // Set offset in seconds - Just stores the value now
    pub fn set_offset_seconds(&self, offset: f64) -> Result<(), Error> {
        // Convert to milliseconds and store as i64
        let millis = (offset * 1000.0) as i64;
        self.offset_seconds.store(millis, Ordering::Relaxed);
        println!("Stored playback offset: {} seconds ({} ms)", offset, millis);
        Ok(())
    }

    // Set offset in frames - Just stores the value now
    pub fn set_offset_frames(&self, frames: i32, fps: f64) -> Result<(), Error> {
        let seconds = frames as f64 / fps;
        // Convert to milliseconds and store as i64
        let millis = (seconds * 1000.0) as i64;
        self.offset_seconds.store(millis, Ordering::Relaxed);
        println!(
            "Stored playback offset: {} frames ({} seconds at {} fps, {} ms)",
            frames, seconds, fps, millis
        );
        Ok(())
    }

    // Get current offset in seconds
    pub fn get_offset_seconds(&self) -> f64 {
        // Convert from milliseconds to seconds
        self.offset_seconds.load(Ordering::Relaxed) as f64 / 1000.0
    }

    // Simplified load_file: Applies offset immediately after loading
    pub fn load_file(&self, file_path: &str) -> Result<(), Error> {
        println!("Loading file: {}", file_path);
        self.command("loadfile", &[file_path])?;

        // Get the current offset value (in seconds)
        let offset = self.get_offset_seconds();

        // Apply offset immediately ONLY if it's significantly non-zero
        // Use a larger threshold (e.g., 0.05s) to avoid seeking near frame 0
        if offset.abs() > 0.05 {
            // Use a slight delay to ensure file is loaded enough for seek
            std::thread::sleep(std::time::Duration::from_millis(200));
            println!(
                "Applying significant offset after load: seeking to {} seconds",
                offset
            );
            self.command("seek", &[&offset.to_string(), "absolute"])?;
        } else {
            println!(
                "Offset near zero ({:.3}s), letting MPV start normally.",
                offset
            );
        }

        Ok(())
    }

    pub fn get_status(&self) -> Result<JsonValue, Error> {
        let handle = self.handle.lock()?;

        // Get essential properties, handling errors gracefully
        let time_pos_opt = handle.get_property::<f64>("time-pos").ok();
        let duration_opt = handle.get_property::<f64>("duration").ok();
        let path_opt = handle.get_property::<String>("path").ok();
        let volume_opt = handle.get_property::<f64>("volume").ok();
        let speed_opt = handle.get_property::<f64>("speed").ok();
        let loop_file_opt = handle.get_property::<String>("loop-file").ok(); // e.g., "no", "inf"
        let pause_opt = handle.get_property::<bool>("pause").ok();
        let eof_reached_opt = handle.get_property::<String>("eof-reached").ok();
        let idle_active_opt = handle.get_property::<bool>("idle-active").ok();
        let media_title_opt = handle.get_property::<String>("media-title").ok();

        // --- Get FPS ---
        let fps_opt = handle
            .get_property::<f64>("container-fps")
            .or_else(|_| handle.get_property::<f64>("estimated-vf-fps")) // Fallback
            .ok(); // Store as Option<f64>
                   // --- End Get FPS ---

        // Determine overall status string and handle potential None values
        let is_idle = idle_active_opt.unwrap_or(path_opt.is_none()); // Idle if explicit or no path
        let is_paused = pause_opt.unwrap_or(is_idle); // Paused if explicit or idle

        let status_str = if is_idle {
            "Idle".to_string()
        } else if is_paused {
            "Paused".to_string()
        } else {
            "Playing".to_string()
        };

        // Get the current playback offset (in seconds)
        let current_offset = self.get_offset_seconds();

        // Adjust the reported time position by subtracting the offset
        let adjusted_time_pos = time_pos_opt.map(|t| t - current_offset);

        // Build the JSON, handling Option types with json! macro support
        Ok(json!({
            "Status": status_str,
            "Position": adjusted_time_pos,
            "Elapsed": adjusted_time_pos, // Legacy field
            "Duration": duration_opt,
            "Path": path_opt,
            "Title": media_title_opt.or(path_opt), // Use path as fallback title
            "Volume": volume_opt.map(|v| v.round()),
            "Speed": speed_opt,
            "Loop": loop_file_opt.map_or(false, |l| l == "inf" || l == "yes"),
            "Offset": current_offset,
            "EndOfFile": eof_reached_opt, // Send Option<String> directly
            "Idle": is_idle, // Send bool directly
            "fps": fps_opt, // Add Option<f64> directly
        }))
    }

    pub fn check_events(&self) {
        // Revert to blocking lock() to ensure events are checked
        if let Ok(mut handle) = self.handle.lock() {
            let event_context = handle.event_context_mut();
            // Use a short timeout for wait_event to avoid blocking indefinitely if no events
            while let Some(Ok(event)) = event_context.wait_event(0.01) {
                // Use 10ms timeout
                match event {
                    Event::Shutdown => {
                        println!("MPV_EVENT_SHUTDOWN received");
                        self.quit_flag.store(true, Ordering::Relaxed);
                    }
                    Event::EndFile(_) => {
                        println!("MPV_EVENT_END_FILE received");
                    }
                    _ => {}
                }
            }
        } else {
            // This path should ideally not be hit often with a blocking lock,
            // but indicates a potential deadlock or poisoned mutex if it is.
            eprintln!("check_events: Failed to acquire MPV lock (potential poison/deadlock?)");
            self.quit_flag.store(true, Ordering::Relaxed); // Assume critical error if lock fails
        }
    }

    pub fn is_shutdown(&self) -> bool {
        if self.quit_flag.load(Ordering::Relaxed) {
            return true;
        }

        // Revert to blocking lock() and check property *after* acquiring lock
        match self.handle.lock() {
            Ok(handle) => {
                // Getting a property is a good health check.
                // If this fails, MPV is likely closed or unresponsive.
                let is_unresponsive = handle.get_property::<bool>("idle-active").is_err();
                if is_unresponsive {
                    println!("is_shutdown: Failed to get property after lock, assuming shutdown.");
                }
                is_unresponsive
            }
            Err(_) => {
                // If we fail to acquire the blocking lock, the mutex is poisoned.
                eprintln!(
                    "is_shutdown: Failed to acquire MPV lock (mutex poisoned?), assuming shutdown"
                );
                true
            }
        }
    }

    pub fn exit(&self) {
        self.quit_flag.store(true, Ordering::Relaxed);
        // Use try_lock here is okay, as sending quit is best-effort during shutdown
        if let Ok(handle) = self.handle.try_lock() {
            let _ = handle.command("quit", &[]);
        } else {
            eprintln!(
                "exit: Could not acquire MPV lock to send quit command (lock held elsewhere?)"
            );
        }
    }

    pub fn get_handle(&self) -> Result<std::sync::MutexGuard<'_, Mpv>, Error> {
        self.handle.lock().map_err(Error::from)
    }

    pub(crate) fn get_handle_internal(&self) -> std::sync::MutexGuard<'_, Mpv> {
        self.handle.lock().expect("Internal MPV handle lock failed")
    }

    pub fn set_loop(&self, enabled: bool) -> Result<(), Error> {
        let value = if enabled { "inf" } else { "no" };
        println!("Setting loop to: {}", value);

        {
            let handle = self.handle.lock()?;
            handle.set_property("loop-file", value)?;
        }
        {
            let handle = self.handle.lock()?;
            handle.set_property("loop-playlist", value)?;
        }

        Ok(())
    }

    pub fn get_loop(&self) -> Result<bool, Error> {
        let handle = self.handle.lock()?;

        let loop_state = handle
            .get_property::<String>("loop-file")
            .map_err(|e| Error::PropertyError(format!("Failed to get loop state: {}", e), -1))?;

        Ok(loop_state != "no")
    }
}

impl Drop for MpvPlayer {
    fn drop(&mut self) {
        // MPV will be dropped automatically
    }
}
