use libmpv2::{events::Event, Mpv};
use serde_json::{self, json};
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::sync::Mutex;

#[derive(Debug)]
#[allow(dead_code)]
pub enum Error {
    InitError(String),
    PropertyError(String, i32),
    CommandError(String, i32),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::InitError(msg) => write!(f, "Initialization error: {}", msg),
            Error::PropertyError(msg, code) => write!(f, "Property error ({}): {}", code, msg),
            Error::CommandError(msg, code) => write!(f, "Command error ({}): {}", code, msg),
        }
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
            ("loop-file", "inf"),  // Set infinite looping by default
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
        let handle = self.handle.lock().unwrap();
        
        // Special handling for pause property which is critical for syncing
        if name == "pause" {
            // For pause property, try as boolean first
            let bool_value = match value {
                "yes" | "true" => true,
                "no" | "false" => false,
                _ => return Err(Error::PropertyError(
                    format!("Invalid pause value: {}", value), -1
                )),
            };
            
            return handle.set_property(name, bool_value)
                .map_err(|e| Error::PropertyError(
                    format!("Failed to set property {} to {}: {}", name, value, e), -1
                ));
        }
        
        // Standard property handling for other properties
        handle.set_property(name, value).map_err(|e| {
            Error::PropertyError(format!("Failed to set property {} to {}: {}", name, value, e), -1)
        })
    }

    pub fn command(&self, cmd: &str, args: &[&str]) -> Result<(), Error> {
        let handle = self.handle.lock().map_err(|_| {
            Error::CommandError("Failed to lock MPV handle".to_string(), -1)
        })?;
        
        // Special case for play/pause commands that are critical for syncing
        if cmd == "set" && args.len() >= 2 && args[0] == "pause" {
            let pause_value = match args[1] {
                "yes" | "true" => true,
                "no" | "false" => false,
                _ => return Err(Error::CommandError(
                    format!("Invalid pause value: {}", args[1]), -1
                )),
            };
            
            return handle.set_property("pause", pause_value)
                .map_err(|e| Error::CommandError(
                    format!("Failed to set pause to {}: {}", args[1], e), -1
                ));
        }
        
        handle
            .command(cmd, args)
            .map_err(|e| Error::CommandError(format!("Failed to execute command {}: {}", cmd, e), -1))
    }

    // Set offset in seconds
    pub fn set_offset_seconds(&self, offset: f64) -> Result<(), Error> {
        // Convert to milliseconds and store as i64
        let millis = (offset * 1000.0) as i64;
        self.offset_seconds.store(millis, Ordering::Relaxed);
        println!("Set playback offset to {} seconds", offset);
        Ok(())
    }

    // Set offset in frames (converts to seconds based on framerate)
    pub fn set_offset_frames(&self, frames: i32, fps: f64) -> Result<(), Error> {
        let seconds = frames as f64 / fps;
        // Convert to milliseconds and store as i64
        let millis = (seconds * 1000.0) as i64;
        self.offset_seconds.store(millis, Ordering::Relaxed);
        println!("Set playback offset to {} frames ({} seconds at {} fps)", frames, seconds, fps);
        Ok(())
    }

    // Get current offset in seconds
    pub fn get_offset_seconds(&self) -> f64 {
        // Convert from milliseconds to seconds
        self.offset_seconds.load(Ordering::Relaxed) as f64 / 1000.0
    }

    // Modified load_file method to apply offset when loading
    pub fn load_file(&self, file_path: &str) -> Result<(), Error> {
        println!("Loading file: {}", file_path);
        self.command("loadfile", &[file_path])?;
        
        // Get the offset value (in seconds)
        let offset = self.get_offset_seconds();
        
        // If there's a positive offset, apply it after a short delay to ensure file is loaded
        if offset > 0.0 {
            // Schedule a delayed seek command 
            println!("Scheduling positive offset: seeking {} seconds", offset);
            self.apply_delayed_command(
                500, // 500ms delay
                "seek".to_string(),
                vec![offset.to_string(), "absolute".to_string()],
            );
        } 
        // If there's a negative offset, we need to pause and wait
        else if offset < 0.0 {
            let delay = -offset;
            
            // First make sure looping is enabled
            self.apply_delayed_command(
                400, // 400ms delay
                "set".to_string(),
                vec!["loop-file".to_string(), "inf".to_string()],
            );

            // Get the video duration after a short delay to check if we're close to the end
            let handle_clone = Arc::clone(&self.handle);
            let offset_val = offset;
            
            std::thread::spawn(move || {
                // Wait for the file to be loaded
                std::thread::sleep(std::time::Duration::from_millis(500));
                
                if let Ok(handle) = handle_clone.lock() {
                    if let Ok(duration) = handle.get_property::<f64>("duration") {
                        println!("Video duration: {} seconds", duration);
                        
                        // If the offset would take us before 0, loop from the end
                        // Use better threshold detection for EOF comparison
                        if duration + offset_val < 0.0 {
                            // Calculate the wrapped position, ensuring we're not too close to EOF
                            let mut wrapped_pos = duration + (offset_val % duration);
                            
                            // Check if we'd end up within 50ms of EOF, which can cause playback issues
                            if duration - wrapped_pos < 0.05 {
                                // Move away from EOF by a small amount to avoid timing edge cases
                                wrapped_pos = duration - 0.1;
                                println!("Adjusted wrapped position to avoid EOF edge case");
                            }
                            
                            // Seek to the appropriate position from the end
                            println!("Applying wrapped negative offset: seeking to {} seconds from end", wrapped_pos);
                            let _ = handle.command("seek", &[&wrapped_pos.to_string(), "absolute"]);
                            // Pause after seeking
                            let _ = handle.set_property("pause", true);
                            
                            // Wait for the delay and then unpause
                            drop(handle); // Drop the mutex lock before sleeping
                            std::thread::sleep(std::time::Duration::from_secs_f64(delay));
                            if let Ok(handle) = handle_clone.lock() {
                                println!("Resuming playback after offset delay");
                                let _ = handle.set_property("pause", false);
                            }
                        } else {
                            // Normal negative offset within duration
                            println!("Applying negative offset: pausing for {} seconds", delay);
                            let _ = handle.set_property("pause", true);
                            
                            drop(handle); // Drop the mutex lock before sleeping
                            std::thread::sleep(std::time::Duration::from_secs_f64(delay));
                            if let Ok(handle) = handle_clone.lock() {
                                println!("Resuming playback after offset delay");
                                let _ = handle.set_property("pause", false);
                            }
                        }
                    } else {
                        // If we can't get the duration, fall back to simple pause-wait-unpause
                        let _ = handle.set_property("pause", true);
                        
                        drop(handle); // Drop the mutex lock before sleeping
                        std::thread::sleep(std::time::Duration::from_secs_f64(delay));
                        if let Ok(handle) = handle_clone.lock() {
                            println!("Resuming playback after offset delay");
                            let _ = handle.set_property("pause", false);
                        }
                    }
                }
            });
        }
        
        Ok(())
    }

    pub fn get_status(&self) -> Result<serde_json::Value, Error> {
        let handle = self.handle.lock().map_err(|_| {
            Error::PropertyError("Failed to lock MPV handle".to_string(), -1)
        })?;
        
        let mut status = serde_json::Map::new();

        // First check if we have a file loaded
        let path_result = handle.get_property::<String>("path");
        
        if path_result.is_err() {
            status.insert("Status".to_string(), json!(serde_json::Value::String("Idle".to_string())));
            return Ok(json!(status));
        }
        
        // Get basic playback info
        let pause = handle.get_property::<bool>("pause").unwrap_or(true);
        let position = handle.get_property::<f64>("time-pos").unwrap_or(0.0);
        let duration = handle.get_property::<f64>("duration").unwrap_or(0.0);
        let volume = handle.get_property::<f64>("volume").unwrap_or(100.0);
        let speed = handle.get_property::<f64>("speed").unwrap_or(1.0);
        let path = path_result.unwrap_or_else(|_| "".to_string());
        
        // Add loop status
        let loop_file = handle.get_property::<String>("loop-file").unwrap_or_else(|_| "no".to_string());
        let loop_enabled = loop_file == "yes" || loop_file == "inf";
        
        // Populate the status object
        status.insert("Status".to_string(), json!(if pause { "Paused" } else { "Playing" }));
        status.insert("Position".to_string(), json!(position));
        status.insert("Duration".to_string(), json!(duration));
        status.insert("Volume".to_string(), json!(volume));
        status.insert("Speed".to_string(), json!(speed));
        status.insert("Path".to_string(), json!(path));
        status.insert("Loop".to_string(), json!(loop_enabled));
        
        // Get other boolean properties
        let bool_properties = [
            ("eof-reached", "EndOfFile"),
            ("idle-active", "Idle"),
        ];
        
        for (prop, label) in bool_properties.iter() {
            if let Ok(value) = handle.get_property::<bool>(prop) {
                status.insert(label.to_string(), json!(if value { "yes" } else { "no" }));
            }
        }

        // Get string properties
        let string_properties = [
            ("media-title", "Title"),
        ];

        for (prop, label) in string_properties.iter() {
            if let Ok(value) = handle.get_property::<String>(prop) {
                status.insert(label.to_string(), json!(value));
            }
        }

        // Get the offset value (in seconds)
        let offset = self.get_offset_seconds();

        // Get numeric properties and adjust with offset for external sync
        let numeric_properties = [
            ("time-pos", "Position"),
            ("percent-pos", "Progress"),
            ("playback-time", "Elapsed"),
            ("playtime-remaining", "Remaining"),
        ];

        for (prop, label) in numeric_properties.iter() {
            if let Ok(value) = handle.get_property::<f64>(prop) {
                // If this is a time position, we need to account for the offset in the reported time
                if prop == &"time-pos" || prop == &"playback-time" {
                    // For time positions, we subtract the offset from the reported time
                    // This makes external tools think the video is at a different position
                    let adjusted_value = value - offset;
                    
                    // Handle precise zero value for accurate frame detection
                    if value < 0.001 {
                        // If raw value is essentially zero, report exactly zero
                        status.insert(label.to_string(), json!(0.0));
                    } else if adjusted_value.abs() < 0.001 {
                        // If adjusted value is very close to zero, report exactly zero
                        status.insert(label.to_string(), json!(0.0));
                    } else {
                        status.insert(label.to_string(), json!(adjusted_value));
                    }
                } else if prop == &"playtime-remaining" && offset != 0.0 {
                    // For remaining time, add the offset
                    status.insert(label.to_string(), json!(value + offset));
                } else if prop == &"percent-pos" && offset != 0.0 {
                    // Try to adjust the percent position if we have duration
                    if let Ok(duration) = handle.get_property::<f64>("duration") {
                        if duration > 0.0 {
                            let adjusted_time = value - offset;
                            let adjusted_percent = (adjusted_time / duration) * 100.0;
                            status.insert(label.to_string(), json!(adjusted_percent.max(0.0).min(100.0)));
                        } else {
                            status.insert(label.to_string(), json!(value));
                        }
                    } else {
                        status.insert(label.to_string(), json!(value));
                    }
                } else {
                    status.insert(label.to_string(), json!(value));
                }
            }
        }

        // Add connection health indicators
        status.insert("healthy".to_string(), json!(true));
        status.insert(
            "timestamp".to_string(),
            json!(std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()),
        );

        // Add the offset value to the status
        status.insert("Offset".to_string(), json!(offset));

        Ok(json!(status))
    }

    pub fn check_events(&self) {
        if let Ok(mut handle) = self.handle.lock() {
            let event_context = handle.event_context_mut();
            while let Some(Ok(event)) = event_context.wait_event(0.0) {
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
        }
    }

    pub fn is_shutdown(&self) -> bool {
        if self.quit_flag.load(Ordering::Relaxed) {
            return true;
        }

        // Try to get a property to see if MPV is still responsive
        if let Ok(handle) = self.handle.lock() {
            handle.get_property::<bool>("idle-active").is_err()
        } else {
            // If we can't lock the mutex, assume MPV is no longer responsive
            true
        }
    }

    pub fn exit(&self) {
        self.quit_flag.store(true, Ordering::Relaxed);
        if let Ok(handle) = self.handle.lock() {
            let _ = handle.command("quit", &[]);
        }
    }

    pub fn get_handle(&self) -> Result<std::sync::MutexGuard<'_, Mpv>, Error> {
        self.handle.lock().map_err(|_| {
            Error::PropertyError("Failed to lock MPV handle".to_string(), -1)
        })
    }

    // Add a method to get the MPV handle for internal use
    pub(crate) fn get_handle_internal(&self) -> std::sync::MutexGuard<'_, Mpv> {
        self.handle.lock().unwrap()
    }

    // Add a command to be executed after a delay
    pub fn apply_delayed_command(&self, delay_ms: u64, command: String, args: Vec<String>) {
        // Create clones of needed data for the thread
        let handle_clone = Arc::clone(&self.handle);
        
        std::thread::spawn(move || {
            // Sleep for the specified delay
            std::thread::sleep(std::time::Duration::from_millis(delay_ms));
            
            // Now execute the command on the MPV instance
            println!("Executing delayed command: {} {:?}", command, args);
            
            // Convert string args to &str for the command
            let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
            
            // Lock the mutex to access the MPV instance
            if let Ok(handle) = handle_clone.lock() {
                // Special handling for pause which needs to be set as a property
                if command == "set" && args.len() >= 2 && args[0] == "pause" {
                    let pause_value = match args[1].as_str() {
                        "yes" | "true" => true,
                        "no" | "false" => false,
                        _ => {
                            eprintln!("Invalid pause value: {}", args[1]);
                            return;
                        },
                    };
                    
                    if let Err(e) = handle.set_property("pause", pause_value) {
                        eprintln!("Error setting property: {}", e);
                    }
                } else {
                    // Regular command
                    if let Err(e) = handle.command(&command, &arg_refs) {
                        eprintln!("Error executing delayed command: {}", e);
                    }
                }
            } else {
                eprintln!("Failed to lock MPV instance for delayed command");
            }
        });
    }

    // Set looping mode
    pub fn set_loop(&self, enabled: bool) -> Result<(), Error> {
        let value = if enabled { "inf" } else { "no" };
        println!("Setting loop to: {}", value);
        
        // Set both loop-file and loop-playlist for maximum compatibility
        self.set_property("loop-file", value)?;
        self.set_property("loop-playlist", value)?;
        
        Ok(())
    }

    // Get current loop state
    pub fn get_loop(&self) -> Result<bool, Error> {
        let handle = self.handle.lock().map_err(|_| {
            Error::PropertyError("Failed to lock MPV handle".to_string(), -1)
        })?;
        
        let loop_state = handle.get_property::<String>("loop-file")
            .map_err(|e| Error::PropertyError(format!("Failed to get loop state: {}", e), -1))?;
        
        Ok(loop_state != "no")
    }
}

impl Drop for MpvPlayer {
    fn drop(&mut self) {
        // MPV will be dropped automatically
    }
}
