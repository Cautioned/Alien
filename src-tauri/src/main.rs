// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod mpv;

use axum::{
    extract::{Path, State as AxumState, WebSocketUpgrade},
    http::StatusCode,
    response::{Html, Json},
    routing::{get, post},
    Router,
};
use mpv::MpvPlayer;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::{interval, MissedTickBehavior};
use tower_http::cors::{Any, CorsLayer};
use tauri::{
    AppHandle,
    Manager,
    WebviewUrl,
    WindowEvent,
    menu::{MenuBuilder, MenuItemBuilder},
    tray::{TrayIconBuilder, TrayIconEvent, MouseButton},
};
use portpicker::pick_unused_port;
use once_cell::sync::Lazy;
use tower_http::services::ServeDir;
use std::path::{PathBuf, Path as StdPath};
use serde_json::Value as JsonValue;
use std::collections::HashSet;
use std::io::ErrorKind;

// --- Configuration Handling --- 
const CONFIG_FILE_NAME: &str = "alien_config.json";

#[derive(Serialize, Deserialize, Debug, Clone)]
struct AppConfig {
    port: Option<u16>,
}

impl Default for AppConfig {
    fn default() -> Self {
        AppConfig { port: None }
    }
}

fn get_config_path(app_handle: &AppHandle) -> std::io::Result<PathBuf> {
    let config_dir = app_handle.path().app_config_dir()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::NotFound, format!("App config directory not found: {}", e)))?;
    // Ensure the config directory exists
    fs::create_dir_all(&config_dir)?;
    Ok(config_dir.join(CONFIG_FILE_NAME))
}

fn load_config(app_handle: &AppHandle) -> AppConfig {
    match get_config_path(app_handle) {
        Ok(path) => {
            if path.exists() {
                match File::open(&path) {
                    Ok(mut file) => {
                        let mut contents = String::new();
                        if file.read_to_string(&mut contents).is_ok() {
                            match serde_json::from_str(&contents) {
                                Ok(config) => {
                                    println!("Loaded config from {:?}: {:?}", path, config);
                                    config
                                },
                                Err(e) => {
                                    eprintln!("Failed to parse config file {:?}: {}. Using default.", path, e);
                                    AppConfig::default()
                                }
                            }
                        } else {
                            eprintln!("Failed to read config file {:?}. Using default.", path);
                            AppConfig::default()
                        }
                    }
                    Err(e) => {
                        eprintln!("Failed to open config file {:?}: {}. Using default.", path, e);
                        AppConfig::default()
                    }
                }
            } else {
                println!("Config file {:?} not found. Using default.", path);
                AppConfig::default()
            }
        }
        Err(e) => {
            eprintln!("Failed to determine config path: {}. Using default.", e);
            AppConfig::default()
        }
    }
}

fn save_config(app_handle: &AppHandle, config: &AppConfig) -> std::io::Result<()> {
    let path = get_config_path(app_handle)?;
    let contents = serde_json::to_string_pretty(config)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let mut file = File::create(&path)?;
    file.write_all(contents.as_bytes())?;
    println!("Saved config to {:?}: {:?}", path, config);
    Ok(())
}
// --- End Configuration Handling ---

struct AppState {
    player: Arc<MpvPlayer>,
    port: u16,
    last_seek: Arc<AtomicU64>, // Track last seek time
    config: Arc<tokio::sync::Mutex<AppConfig>>, // Add config to AppState
}

#[tauri::command]
async fn sync_room(room_id: String) -> Result<String, String> {
    println!("Sync request for room: {}", room_id);
    Ok(format!("Connected to room {}", room_id))
}

#[tauri::command]
async fn exit_app(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), String> {
    println!("Exit requested.");
    state.player.exit();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(500)).await;
        app_handle.exit(0);
    });
    Ok(())
}

#[tauri::command]
async fn set_port_and_restart(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    new_port: u16,
) -> Result<(), String> {
    println!("Set port and restart requested: New port = {}", new_port);
    if !(1025..=65535).contains(&new_port) {
        return Err("Invalid port number. Must be between 1025 and 65535.".to_string());
    }
    let mut config_guard = state.config.lock().await;
    config_guard.port = Some(new_port);
    if let Err(e) = save_config(&app_handle, &config_guard) {
        eprintln!("Failed to save config: {}", e);
        return Err(format!("Failed to save configuration: {}", e));
    }
    drop(config_guard);
    println!("Configuration saved. Attempting to restart application...");
    state.player.exit();
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Call restart. This terminates the process if successful.
    app_handle.restart();
    // No code needed here. If restart() succeeds, process terminates.
    // If it somehow failed and returned, the function would implicitly complete,
    // satisfying the Result<(), String> signature, but this path isn't expected.
}

#[tauri::command]
async fn reset_port_and_restart(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), String> {
    println!("Reset port to auto-detect and restart requested.");
    let mut config_guard = state.config.lock().await;
    config_guard.port = None;
    if let Err(e) = save_config(&app_handle, &config_guard) {
        eprintln!("Failed to save config for reset: {}", e);
        return Err(format!("Failed to save configuration for reset: {}", e));
    }
    drop(config_guard);
    println!("Configuration saved for reset. Attempting to restart application...");
    state.player.exit();
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Call restart. This terminates the process if successful.
    app_handle.restart();
    // No code needed here.
}

// --- New Command: Clear Saved Port (No Restart) --- 
#[tauri::command]
async fn clear_saved_port(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), String> {
    println!("Clear saved port requested.");
    let mut config_guard = state.config.lock().await;
    if config_guard.port.is_none() {
        println!("No specific port was saved. Nothing to clear.");
        return Ok(()); // Nothing to do
    }
    config_guard.port = None; // Set to None for auto-detect on next launch
    if let Err(e) = save_config(&app_handle, &config_guard) {
        eprintln!("Failed to save config for port clear: {}", e);
        return Err(format!("Failed to save configuration for port clear: {}", e));
    }
    println!("Saved port configuration cleared. Will use auto-detect on next launch.");
    Ok(())
}

async fn control_player(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    Path(action): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let cmd_result = match action.as_str() {
        "play" => {
            // First check if player is paused
            let handle = state.player.get_handle()
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            let is_paused = handle.get_property::<bool>("pause").unwrap_or(false);
            
            if is_paused {
                // Use direct set_property with bool value for better compatibility
                handle.set_property("pause", false)
                    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
                Ok(())
            } else {
                // Already playing, just return success
                Ok(())
            }
        },
        "pause" => {
            // First check if player is playing
            let handle = state.player.get_handle()
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            let is_paused = handle.get_property::<bool>("pause").unwrap_or(false);
            
            if !is_paused {
                // Use direct set_property with bool value for better compatibility
                handle.set_property("pause", true)
                    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
                Ok(())
            } else {
                // Already paused, just return success
                Ok(())
            }
        },
        "stop" => state.player.command("stop", &[]),
        "volume_up" => state.player.command("add", &["volume", "5"]),
        "volume_down" => state.player.command("add", &["volume", "-5"]),
        "seek_forward" => state.player.command("seek", &["10"]),
        "seek_backward" => state.player.command("seek", &["-10"]),
        _ => return Err((StatusCode::BAD_REQUEST, "Invalid action".to_string())),
    };

    cmd_result
        .map(|_| Json(json!({ "status": "success", "action": action })))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    AxumState(state): AxumState<Arc<AppState>>,
) -> impl axum::response::IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

// Helper to send JSON error messages over WebSocket
async fn send_ws_error(socket: &mut axum::extract::ws::WebSocket, command: &str, error_message: &str) {
    use axum::extract::ws::Message;
    let error_json = json!({
        "status": "error",
        "command": command,
        "error": error_message
    });
    if let Ok(error_str) = serde_json::to_string(&error_json) {
        if socket.send(Message::Text(error_str)).await.is_err() {
            eprintln!("Failed to send WebSocket error message");
        }
    } else {
         eprintln!("Failed to serialize error message");
    }
}

async fn handle_socket(mut socket: axum::extract::ws::WebSocket, state: Arc<AppState>) {
    use axum::extract::ws::Message;
    use tokio::time::{sleep, Instant};
    use std::sync::atomic::Ordering;

    // --- Dynamic Interval Logic ---
    const PLAYING_STATUS_INTERVAL: Duration = Duration::from_millis(100); // Faster when playing
    const PAUSED_STATUS_INTERVAL: Duration = Duration::from_millis(300); // Slower when paused
    let mut current_interval_duration = PLAYING_STATUS_INTERVAL; // Start with playing interval
    let mut status_interval = interval(current_interval_duration);
    status_interval.set_missed_tick_behavior(MissedTickBehavior::Delay); // Prevent burst ticks after delay
    let mut last_known_pause_state = false; // Track pause state to adjust interval
    // --- End Dynamic Interval Logic ---

    // Constants for connection management
    const PING_INTERVAL: Duration = Duration::from_secs(15);
    const PING_TIMEOUT: Duration = Duration::from_secs(60);
    const MAX_CONSECUTIVE_ERRORS: u32 = 20;
    const MIN_STATUS_INTERVAL: u64 = 8;
    const ERROR_BACKOFF: Duration = Duration::from_millis(100);

    // Connection state
    let mut ping_interval = interval(PING_INTERVAL);
    ping_interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut last_pong = Instant::now();
    let mut consecutive_errors = 0;
    let last_status_update = Arc::new(AtomicU64::new(0));

    // --- State for Conditional Updates --- 
    let mut last_sent_status: Option<JsonValue> = None; // Store the last sent status object
    // Define keys whose changes trigger an update
    let relevant_keys: HashSet<String> = [
        "Status", "Position", "Duration", "Path", "Title", "Loop", "Offset", "EndOfFile", "Idle"
    ].iter().map(|s| s.to_string()).collect();

    // --- Buffers --- 
    let mut status_buffer = String::with_capacity(1024);

    'connection: loop {
        // --- Check for MPV Shutdown --- 
        if state.player.is_shutdown() {
            eprintln!("MPV shutdown detected by handle_socket, closing WebSocket.");
            let _ = socket.close().await; // Attempt graceful close
                    break 'connection;
                }
        // --- End Check for MPV Shutdown ---

        tokio::select! {
            biased; // Prioritize receiving messages over sending status/pings

            result = socket.recv() => {
                match result {
                    Some(Ok(Message::Pong(_))) => {
                        last_pong = Instant::now();
                        consecutive_errors = 0;
                    }
                    Some(Ok(Message::Text(text))) => {
                        last_pong = Instant::now(); // Treat text message as activity
                        consecutive_errors = 0;
                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                            if let Some(command_str) = json.get("command").and_then(|v| v.as_str()) {
                                // --- WebSocket Command Handling with Error Reporting ---
                                match command_str {
                                    "loadURL" => {
                                        if let Some(url) = json.get("url").and_then(|v| v.as_str()) {
                                            println!("Loading URL: {}", url);
                                            if let Err(e) = state.player.load_file(url) {
                                                eprintln!("Error loading URL: {}", e);
                                                send_ws_error(&mut socket, "loadURL", &e.to_string()).await;
                                            }
                                        } else {
                                             send_ws_error(&mut socket, "loadURL", "Missing or invalid 'url' field").await;
                                        }
                                    },
                                    "play" => {
                                        match state.player.set_property("pause", "false") { // Use set_property directly
                                            Ok(_) => println!("Play command executed successfully via WebSocket"),
                                            Err(e) => {
                                                eprintln!("Error setting pause=false: {}", e);
                                                send_ws_error(&mut socket, "play", &e.to_string()).await;
                                            }
                                        }
                                    },
                                    "pause" => {
                                         match state.player.set_property("pause", "true") { // Use set_property directly
                                            Ok(_) => println!("Pause command executed successfully via WebSocket"),
                                            Err(e) => {
                                                eprintln!("Error setting pause=true: {}", e);
                                                send_ws_error(&mut socket, "pause", &e.to_string()).await;
                                            }
                                        }
                                    },
                                    "seek" => {
                                        if let Some(position) = json.get("position").and_then(|v| v.as_f64()) {
                                            println!("Seeking to: {}", position);
                                            let offset = state.player.get_offset_seconds();
                                            let adjusted_time = position + offset;
                                            println!("WebSocket seek: adjusted to {} from {} (offset {})", adjusted_time, position, offset);
                                            if let Err(e) = state.player.command("seek", &[&adjusted_time.to_string(), "absolute"]) {
                                                eprintln!("Error seeking: {}", e);
                                                send_ws_error(&mut socket, "seek", &e.to_string()).await;
                                            }
                                        } else {
                                            send_ws_error(&mut socket, "seek", "Missing or invalid 'position' field").await;
                                        }
                                    },
                                    "setOffset" => {
                                        let mut offset_result: Result<f64, mpv::Error> = Err(mpv::Error::CommandError("Invalid offset parameters".to_string(), -1));
                                        let mut update_type = "seconds"; // For response message
                                        let mut original_value: f64 = 0.0;
                                        let mut fps_value: f64 = 30.0;

                                        if let Some(seconds) = json.get("seconds").and_then(|v| v.as_f64()) {
                                            println!("Setting offset to {} seconds via WebSocket", seconds);
                                            offset_result = state.player.set_offset_seconds(seconds).map(|_| seconds);
                                            original_value = seconds;
                                        } else if let Some(frames) = json.get("frames").and_then(|v| v.as_i64()) {
                                            fps_value = json.get("fps").and_then(|v| v.as_f64()).unwrap_or(30.0);
                                            println!("Setting offset to {} frames at {} fps via WebSocket", frames, fps_value);
                                            let seconds = frames as f64 / fps_value;
                                            offset_result = state.player.set_offset_frames(frames as i32, fps_value).map(|_| seconds);
                                            update_type = "frames";
                                            original_value = frames as f64;
                                        }

                                        match offset_result {
                                            Ok(calculated_seconds) => {
                                                let response_json = if update_type == "seconds" {
                                                    json!({
                                                        "status": "success",
                                                        "command": "offsetUpdated",
                                                        "seconds": calculated_seconds
                                                    })
                                                    } else {
                                                     json!({
                                                        "status": "success",
                                                        "command": "offsetUpdated",
                                                        "frames": original_value as i64,
                                                        "seconds": calculated_seconds,
                                                        "fps": fps_value
                                                    })
                                                };
                                                if let Ok(response_str) = serde_json::to_string(&response_json) {
                                                    if socket.send(Message::Text(response_str)).await.is_err() {
                                                        eprintln!("Error sending offset confirmation");
                                                    }
                                                    } else {
                                                     eprintln!("Error serializing offset confirmation");
                                                }
                                            },
                                            Err(e) => {
                                                eprintln!("Error setting offset: {}", e);
                                                send_ws_error(&mut socket, "setOffset", &e.to_string()).await;
                                            }
                                        }
                                    },
                                    "getOffset" => {
                                        let offset = state.player.get_offset_seconds();
                                        let response = json!({
                                            "status": "success",
                                            "command": "offsetStatus",
                                            "seconds": offset
                                        });
                                        if let Ok(response_str) = serde_json::to_string(&response) {
                                            if socket.send(Message::Text(response_str)).await.is_err() {
                                                eprintln!("Error sending offset status");
                                            }
                                        } else {
                                             eprintln!("Error serializing offset status");
                                        }
                                    },
                                    "setLoop" => {
                                        if let Some(enabled) = json.get("enabled").and_then(|v| v.as_bool()) {
                                            println!("Setting loop to: {}", enabled);
                                            match state.player.set_loop(enabled) {
                                                Ok(_) => {
                                                    let response = json!({
                                                        "status": "success",
                                                        "command": "loopUpdated",
                                                        "enabled": enabled
                                                    });
                                                     if let Ok(response_str) = serde_json::to_string(&response) {
                                                        if socket.send(Message::Text(response_str)).await.is_err() {
                                                            eprintln!("Error sending loop confirmation");
                                                        }
                                                    } else {
                                                        eprintln!("Error serializing loop confirmation");
                                                    }
                                                },
                                                Err(e) => {
                                                eprintln!("Error setting loop: {}", e);
                                                    send_ws_error(&mut socket, "setLoop", &e.to_string()).await;
                                                }
                                            }
                                        } else {
                                            send_ws_error(&mut socket, "setLoop", "Missing or invalid 'enabled' field").await;
                                        }
                                    },
                                    "getLoop" => {
                                        match state.player.get_loop() {
                                            Ok(enabled) => {
                                                let response = json!({
                                                    "status": "success",
                                                    "command": "loopStatus",
                                                    "enabled": enabled
                                                });
                                                if let Ok(response_str) = serde_json::to_string(&response) {
                                                     if socket.send(Message::Text(response_str)).await.is_err() {
                                                        eprintln!("Error sending loop status");
                                                    }
                                                } else {
                                                     eprintln!("Error serializing loop status");
                                                }
                                            },
                                            Err(e) => {
                                                eprintln!("Error getting loop status: {}", e);
                                                // Optionally send error back if needed
                                                // send_ws_error(&mut socket, "getLoop", &e.to_string()).await;
                                            }
                                        }
                                    },
                                    _ => {
                                        eprintln!("Received unknown WebSocket command: {}", command_str);
                                        send_ws_error(&mut socket, command_str, "Unknown command").await;
                                }
                                }
                                // --- End WebSocket Command Handling ---
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) => {
                        eprintln!("Clean WebSocket close received");
                        break 'connection;
                    }
                    Some(Err(e)) => {
                        eprintln!("WebSocket error: {}", e);
                        consecutive_errors += 1;
                        if consecutive_errors >= MAX_CONSECUTIVE_ERRORS {
                            eprintln!("Too many errors, closing WebSocket");
                            break 'connection;
                        }
                        sleep(ERROR_BACKOFF).await; // Backoff on error
                    }
                    None => {
                        eprintln!("WebSocket closed by client.");
                        break 'connection;
                    }
                    _ => {} // Ignore other message types like Binary
                }
            }

            // --- Status Update Tick --- 
            _ = status_interval.tick() => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;

                if now - last_status_update.load(Ordering::Relaxed) < MIN_STATUS_INTERVAL {
                    continue;
                }

                match state.player.get_status() {
                    Ok(current_status) => {
                        // --- Check if status changed --- 
                        let mut should_send = true; // Send first time or if comparison fails
                        if let Some(last_status) = &last_sent_status {
                            // Compare only relevant keys for changes
                            should_send = relevant_keys.iter().any(|key| {
                                current_status.get(key) != last_status.get(key)
                            });
                        }
                        // --- End Check --- 

                        if should_send {
                            status_buffer.clear();
                            if let Ok(status_str) = serde_json::to_string(&current_status) {
                                status_buffer.push_str(&status_str);
                                if socket.send(Message::Text(status_buffer.clone())).await.is_ok() {
                                    last_sent_status = Some(current_status.clone()); // Store the sent status
                                    last_status_update.store(now, Ordering::Relaxed);
                                    consecutive_errors = 0;

                                    // Adjust Interval Logic (remains the same)
                                    let is_paused = current_status.get("Status").and_then(|v| v.as_str()) == Some("Paused");
                                    let desired_interval = if is_paused {
                                        PAUSED_STATUS_INTERVAL
                                    } else {
                                        PLAYING_STATUS_INTERVAL
                                    };
                                    if is_paused != last_known_pause_state || current_interval_duration != desired_interval {
                                        println!("Adjusting status interval. Paused: {}, New Interval: {:?}", is_paused, desired_interval);
                                        status_interval = interval(desired_interval);
                                        status_interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
                                        current_interval_duration = desired_interval;
                                        last_known_pause_state = is_paused;
                                    }
                                } else {
                                    eprintln!("Non-fatal status send error, will retry");
                                    consecutive_errors += 1;
                                    if consecutive_errors >= MAX_CONSECUTIVE_ERRORS {
                                        eprintln!("Too many status send errors, closing WebSocket");
                                        break 'connection;
                                    }
                                    sleep(ERROR_BACKOFF).await;
                                }
                            }
                        } else {
                            // Status hasn't changed significantly, skip sending
                            // println!("Status unchanged, skipping send."); // Optional debug log
                        }
                    }
                    Err(e) => {
                        eprintln!("Error getting player status: {}", e);
                        consecutive_errors += 1;
                         if consecutive_errors >= MAX_CONSECUTIVE_ERRORS {
                            eprintln!("Too many status get errors, closing WebSocket");
                            break 'connection;
                        }
                        sleep(ERROR_BACKOFF).await; // Backoff on error
                    }
                }
            }

            // --- Ping Tick --- 
            _ = ping_interval.tick() => {
                if last_pong.elapsed() > PING_TIMEOUT {
                    eprintln!("WebSocket ping timeout, closing connection.");
                    break 'connection;
                }

                if socket.send(Message::Ping(vec![])).await.is_err() {
                    eprintln!("Non-fatal ping error, will retry");
                    consecutive_errors += 1;
                    if consecutive_errors >= MAX_CONSECUTIVE_ERRORS {
                        eprintln!("Too many ping errors, closing WebSocket");
                        break 'connection;
                    }
                     sleep(ERROR_BACKOFF).await; // Backoff on error
                }
            }
        }
    }
    eprintln!("WebSocket connection ended.");
}

async fn status_page(
    AxumState(state): AxumState<Arc<AppState>>,
) -> Result<Html<String>, (StatusCode, String)> {
    // Cache the HTML template
    static HTML_TEMPLATE: Lazy<String> = Lazy::new(|| {
        include_str!("../templates/status.html").to_string()
    });

    // Get the current port from the state
    let current_port = state.port;
    // Get the configured port (if any) for display
    let config = state.config.lock().await;
    let configured_port_str = config.port.map_or("Auto (Default: 3000)".to_string(), |p| p.to_string());
    drop(config);

    // Replace placeholders with actual values
    let html = HTML_TEMPLATE
        .replace("{current_port}", &current_port.to_string())
        .replace("{configured_port}", &configured_port_str)
        // Keep replacing {port} for backward compatibility - remove semicolon
        .replace("{port}", &current_port.to_string());

    Ok(Html(html))
}

async fn room_status(Path(room_id): Path<String>) -> Json<serde_json::Value> {
    Json(json!({
        "room": room_id,
        "status": "active",
        "users": []
    }))
}

async fn sync() -> Json<serde_json::Value> {
    Json(json!({
        "status": "synced",
        "timestamp": chrono::Utc::now().timestamp()
    }))
}

// Add new API endpoints for Roblox sync
async fn get_playback_time(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let handle = state.player.get_handle()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let time = handle
        .get_property::<f64>("time-pos")
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let duration = handle
        .get_property::<f64>("duration")
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(json!({
        "time": time,
        "duration": duration
    })))
}

async fn set_playback_time(
    AxumState(state): AxumState<Arc<AppState>>,
    Json(payload): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    const MIN_SEEK_INTERVAL: u64 = 50; 
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    // --- Explicit Idle Check First ---
    match state.player.get_handle() {
        Ok(handle) => {
            // Check if a path is loaded. If get_property fails, assume idle.
            if handle.get_property::<String>("path").is_err() {
                let time = payload["time"].as_f64().unwrap_or(-1.0); // Get time for logging if possible
                println!("HTTP seek: Player is idle (no path property), ignoring seek request to {}.", time);
        return Ok(Json(json!({
                    "status": "success",
                    "ignored": true, 
                    "reason": "Player is idle (no media loaded)",
                    "time": time, // Include requested time in response
                    "timestamp": now
        })));
            }
            // If path exists, proceed with the seek logic
        }
        Err(e) => {
            // Failed to get handle, this is a more significant error
            return Err((StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to get MPV handle: {}", e)));
        }
    }
    // --- End Idle Check ---

    // Rate limit check (only apply if not idle)
    if now - state.last_seek.load(Ordering::Relaxed) < MIN_SEEK_INTERVAL {
        return Ok(Json(json!({ "status": "rate_limited", "message": "Too many seek requests" })));
    }

    // Get time from payload
    let time = payload["time"].as_f64().ok_or((
        StatusCode::BAD_REQUEST,
        "Missing or invalid 'time' field".to_string(),
    ))?;

    // Get offset
    let offset = state.player.get_offset_seconds();
    let adjusted_time = time + offset;

    // Attempt seek command (should succeed if we passed the idle check)
    println!("HTTP seek: Attempting seek to {} (adjusted from {} with offset {})", adjusted_time, time, offset);
    match state.player.command("seek", &[&adjusted_time.to_string(), "absolute"]) {
        Ok(_) => {
            // Seek succeeded
    state.last_seek.store(now, Ordering::Relaxed);
    Ok(Json(json!({
        "status": "success",
                "ignored": false, 
        "time": time,
        "adjusted_time": adjusted_time,
        "offset": offset,
        "timestamp": now
    })))
        }
        Err(e) => {
            // If seek fails even after the idle check, it's an unexpected error
            let error_string = e.to_string();
            eprintln!("Error executing MPV seek command even after idle check: {}", error_string);
            Err((StatusCode::INTERNAL_SERVER_ERROR, format!("Seek command failed unexpectedly: {}", error_string)))
        }
    }
}

async fn get_connection_status(
    AxumState(state): AxumState<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    // Get player status
    let status = state
        .player
        .get_status()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Extract status values with default fallbacks
    let get_str = |key: &str| status.get(key).and_then(|v| v.as_str()).unwrap_or("");
    let get_f64 = |key: &str| status.get(key).and_then(|v| v.as_f64()).unwrap_or(0.0);

    let eof_reached = get_str("EndOfFile") == "yes";
    let is_idle = get_str("Idle") == "yes";
    // Check Status property for play/pause state as reported by get_status
    let is_paused = get_str("Status") == "Paused"; 
    let duration = get_f64("Duration");
    // Use the adjusted position reported by get_status
    let position = get_f64("Position");

    // Player is playing if not paused, not at EOF, and not idle
    let is_playing = !is_paused && !eof_reached && !is_idle;

    Ok(Json(json!({
        "connected": true,
        "playing": is_playing,
        "port": state.port,
        "timestamp": chrono::Utc::now().timestamp_millis(),
        "eof": eof_reached,
        "idle": is_idle,
        "duration": duration,
        "position": position,
        "paused": is_paused, // Add explicit paused state
        "offset": state.player.get_offset_seconds() // Add current offset
    })))
}

// Add these handlers for offset functionality
async fn set_offset(
    AxumState(state): AxumState<Arc<AppState>>,
    Json(payload): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    // Removed file reload and delayed commands
    
    // Handle both seconds and frames offsets
    let offset_seconds = if let Some(seconds) = payload.get("seconds").and_then(|v| v.as_f64()) {
        state.player.set_offset_seconds(seconds)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        seconds
    } else if let Some(frames) = payload.get("frames").and_then(|v| v.as_i64()) {
        // Default to 30 fps if not specified
        let fps = payload.get("fps").and_then(|v| v.as_f64()).unwrap_or(30.0);
        
        state.player.set_offset_frames(frames as i32, fps)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        
        (frames as f64) / fps
    } else {
        return Err((StatusCode::BAD_REQUEST, "Missing 'seconds' or 'frames' parameter".to_string()));
    };

    // Just updating the internal offset is sufficient now.
    // The change will be reflected in subsequent get_status and seek calls.
    println!("Updated offset to {} seconds via HTTP", offset_seconds);

    Ok(Json(json!({
        "status": "success",
        "offset_seconds": offset_seconds
    })))
}

async fn get_offset(
    AxumState(state): AxumState<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let offset_seconds = state.player.get_offset_seconds();
    
    Ok(Json(json!({
        "status": "success",
        "offset_seconds": offset_seconds
    })))
}

// New endpoints for loop control
async fn set_loop(
    AxumState(state): AxumState<Arc<AppState>>,
    Json(payload): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    // Get loop setting from payload
    let enabled = payload.get("enabled")
        .and_then(|v| v.as_bool())
        .ok_or((StatusCode::BAD_REQUEST, "Missing or invalid 'enabled' field".to_string()))?;
    
    state.player.set_loop(enabled)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    
    Ok(Json(json!({
        "status": "success",
        "loop_enabled": enabled
    })))
}

async fn get_loop(
    AxumState(state): AxumState<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let loop_enabled = state.player.get_loop()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    
    Ok(Json(json!({
        "status": "success",
        "loop_enabled": loop_enabled
    })))
}

// --- Start: Restore find_templates_dir helper ---
fn find_templates_dir() -> PathBuf {
    // Executable directory
    let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(|p| p.to_path_buf()));
    // Current working directory
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    // Cargo manifest dir (compile‑time)
    const MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");

    // Build list of candidate paths
    let mut candidates: Vec<PathBuf> = Vec::new();
    // dev paths
    candidates.push(StdPath::new(MANIFEST_DIR).join("src-tauri/templates"));
    candidates.push(StdPath::new(MANIFEST_DIR).join("templates"));
    candidates.push(cwd.join("src-tauri/templates"));
    candidates.push(cwd.join("templates"));
    // production paths (resources next to exe, or inside Resources on macOS)
    if let Some(ref dir) = exe_dir {
        candidates.push(dir.join("../Resources/templates")); // macOS bundle structure
        candidates.push(dir.join("resources/templates")); // Other platforms
        candidates.push(dir.join("templates")); // Fallback if directly next to exe
    }

    // First path that exists wins
    for p in &candidates {
        if p.exists() {
            println!("Using templates directory: {:?}", p);
            return p.clone();
        }
    }

    // Fallback if nothing found
    let fallback = cwd.join("src-tauri/templates"); // Should ideally not be reached in prod
    println!("Templates directory not found in expected locations, falling back to {:?}", fallback);
    fallback
}
// --- End: Restore find_templates_dir helper ---

#[tokio::main]
async fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_http::init())
        .invoke_handler(tauri::generate_handler![
            sync_room,
            exit_app,
            set_port_and_restart,
            reset_port_and_restart,
            clear_saved_port
        ])
        .setup(|app| {
            let app_handle = app.handle().clone();
            let config = load_config(&app_handle);
            let initial_config = Arc::new(tokio::sync::Mutex::new(config.clone()));

            // --- Determine Port (Sync check) ---
            let port = match config.port {
                Some(p) if (1025..=65535).contains(&p) => {
                    match std::net::TcpListener::bind(format!("0.0.0.0:{}", p)) {
                        Ok(listener) => { drop(listener); p },
                        Err(e) => {
                            eprintln!("Configured port {} is unavailable: {}. Falling back.", p, e);
                            pick_unused_port().expect("No ports available")
                        }
                    }
                },
                Some(p) => {
                    eprintln!("Configured port {} is invalid. Falling back.", p);
                    pick_unused_port().expect("No ports available")
                }
                None => {
                    match std::net::TcpListener::bind(("0.0.0.0", 3000)) {
                        Ok(listener) => { drop(listener); 3000 },
                        Err(_) => pick_unused_port().expect("No ports available"),
                    }
                }
            };
            let url = format!("http://localhost:{}", port);

            // --- Initialize MPV Player ---
            let (player, _exit_receiver) = MpvPlayer::new().expect("Failed to initialize MPV player");
            let player = Arc::new(player);

            // --- Create App State ---
            let app_state = Arc::new(AppState {
                player: Arc::clone(&player),
                port,
                last_seek: Arc::new(AtomicU64::new(0)),
                config: initial_config,
            });
            app.manage(app_state.clone());

            // --- Start Axum Server ---
            let static_path = find_templates_dir();
            println!("Axum will serve static files from: {:?}", static_path);

            let axum_app = Router::new()
                .route("/ws", get(ws_handler))
                .route("/control/:action", post(control_player))
                .route("/room/:id", get(room_status))
                .route("/sync", post(sync))
                .route("/playback/time", get(get_playback_time).post(set_playback_time))
                .route("/playback/offset", get(get_offset).post(set_offset))
                .route("/playback/loop", get(get_loop).post(set_loop))
                .route("/status", get(get_connection_status))
                .route("/", get(status_page))
                .nest_service("/static", ServeDir::new(static_path))
                .layer(CorsLayer::new().allow_origin(Any).allow_methods(Any).allow_headers(Any))
                .with_state(app_state.clone());

            tokio::spawn(async move {
                match tokio::net::TcpListener::bind(format!("0.0.0.0:{}", port)).await {
                    Ok(listener) => {
                        println!("Server running on http://localhost:{}", port);
                        if let Err(e) = axum::serve(listener, axum_app).await {
                            eprintln!("Server error: {}", e);
                        }
                    }
                    Err(e) => {
                        eprintln!("Failed to bind Axum server to port {}: {}", port, e);
                    }
                }
            });

            // --- Create Tray Menu (Using Builder Pattern) ---
            let quit_item = MenuItemBuilder::new("Quit Alien")
                .id("quit")
                .build(&app_handle)?;
            let show_item = MenuItemBuilder::new("Show Main Window")
                .id("show_main")
                .build(&app_handle)?;

            let tray_menu = MenuBuilder::new(&app_handle)
                .items(&[&show_item, &quit_item])
                .build()?;

            // --- Tray Icon Setup (Handling events inline) ---
            let url_clone_for_menu = url.clone(); // Clone url for menu event closure
            let url_clone_for_tray = url.clone(); // Clone url for tray event closure (Corrected: Separate clone)

            let _tray_icon = TrayIconBuilder::new()
                .menu(&tray_menu)
                .tooltip("Alien Sync Tool")
                .icon(app_handle.default_window_icon().cloned().ok_or_else(|| tauri::Error::InvalidIcon(std::io::Error::new(ErrorKind::NotFound, "Default icon not found")))?) 
                .on_menu_event(move |app, event| {
                    let url_clone = url_clone_for_menu.clone(); // Clone again for move closure
                    match event.id.as_ref() {
                        "quit" => {
                            println!("Quit requested from tray.");
                            app.exit(0);
                        }
                        "show_main" => { // Always create new window
                            println!("Show main window requested from tray menu. Creating new window...");
                            if let Err(e) = tauri::WebviewWindowBuilder::new(
                                app,
                                "main", 
                                WebviewUrl::External(url_clone.parse().expect("Invalid external URL in menu handler")),
                            )
                            .title("Alien Sync")
                            .inner_size(800.0, 600.0)
                            .build()
                            {
                                eprintln!("Failed to create main window from tray menu: {}", e);
                            }
                        }
                        _ => {}
                    }
                })
                .on_tray_icon_event(move |tray, event| {
                    let url_clone = url_clone_for_tray.clone(); // Clone again for move closure
                    match event {
                        TrayIconEvent::Click { button: MouseButton::Left, .. } => { // Always create new window
                             println!("Tray icon clicked. Creating new main window...");
                             let app = tray.app_handle();
                             if let Err(e) = tauri::WebviewWindowBuilder::new(
                                app,
                                "main",
                                WebviewUrl::External(url_clone.parse().expect("Invalid external URL in tray handler")),
                            )
                            .title("Alien Sync")
                            .inner_size(800.0, 600.0)
                            .build()
                            {
                                eprintln!("Failed to create main window from tray icon click: {}", e);
                            }
                        }
                        _ => {} // Ignore other tray events
                    }
                })
                .build(&app_handle)?;

             // --- Create Dummy Background Window --- 
             let _dummy_window = tauri::WebviewWindowBuilder::new(
                app,
                "background-runner", // Unique label
                 WebviewUrl::App("".into()), // Load empty content
             )
             .visible(false) // Keep it hidden
             .skip_taskbar(true) // Don't show in taskbar
             .title("Alien Background Runner") // Optional title
             .inner_size(1.0, 1.0) // Minimal size
             .build()?;

            // --- Create Initial Main Window (Using External URL) ---
            let main_window = match app.get_webview_window("main") {
                Some(win) => win, // Should ideally not exist yet, but handle defensively
                None => {
                    tauri::WebviewWindowBuilder::new(
                        app,
                        "main",
                        WebviewUrl::External(url.parse().expect("Invalid external URL for initial window")),
                    )
                    .title("Alien Sync")
                    .inner_size(800.0, 600.0)
                    .build()?
                }
            };

            // --- Monitor MPV Events ---
            let app_handle_clone = app_handle.clone(); // Clone AppHandle for the monitor thread
            let player_clone = Arc::clone(&player);
            std::thread::spawn(move || {
                loop {
                    player_clone.check_events();
                    if player_clone.is_shutdown() {
                        println!("MPV player shutdown detected by monitor thread. Exiting application.");
                        app_handle_clone.exit(0); // Exit the entire application
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            });

            Ok(())
        })
        .on_window_event(|window, event| match event {
            WindowEvent::CloseRequested { .. } => { // Removed `api`
                if window.label() == "main" { // Only handle main window close
                    println!("Main window close requested. Allowing close (webview terminates).");
                    // No hide, no prevent_close. Let it close naturally.
                } else if window.label() == "background-runner" {
                    println!("Background runner window close requested (should not happen normally). Ignoring.");
                    // Optionally prevent close here if needed, but usually unnecessary
                }
            }
            _ => {}
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
