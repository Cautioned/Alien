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
use serde_json::json;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tower_http::cors::{Any, CorsLayer};
use tauri::WebviewUrl;
use portpicker::pick_unused_port;
use once_cell::sync::Lazy;
use tower_http::services::ServeDir;
use std::path::{PathBuf, Path as StdPath};

struct AppState {
    player: Arc<MpvPlayer>,
    port: u16,
    last_seek: Arc<AtomicU64>, // Track last seek time
}

#[tauri::command]
async fn sync_room(room_id: String) -> Result<String, String> {
    println!("Sync request for room: {}", room_id);
    Ok(format!("Connected to room {}", room_id))
}

#[tauri::command]
async fn exit_app(
    app_handle: tauri::AppHandle,
    player: tauri::State<'_, Arc<AppState>>,
) -> Result<(), String> {
    // Tell MPV to exit
    player.player.exit();
    // Schedule the application to exit shortly
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(500)).await;
        app_handle.exit(0);
    });
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

async fn handle_socket(mut socket: axum::extract::ws::WebSocket, state: Arc<AppState>) {
    use axum::extract::ws::Message;
    use tokio::time::{interval, sleep, Instant};
    use std::sync::atomic::Ordering;

    // Constants for connection management
    const STATUS_INTERVAL: Duration = Duration::from_millis(100);
    const PING_INTERVAL: Duration = Duration::from_secs(15);
    const PING_TIMEOUT: Duration = Duration::from_secs(60);
    const MAX_CONSECUTIVE_ERRORS: u32 = 20;
    const MIN_STATUS_INTERVAL: u64 = 8;
    const ERROR_BACKOFF: Duration = Duration::from_millis(100);

    // Connection state
    let mut status_interval = interval(STATUS_INTERVAL);
    let mut ping_interval = interval(PING_INTERVAL);
    let mut last_pong = Instant::now();
    let mut consecutive_errors = 0;
    let last_status_update = Arc::new(AtomicU64::new(0));

    // Pre-allocate buffers for status updates
    let mut status_buffer = String::with_capacity(1024);

    'connection: loop {
        tokio::select! {
            _ = status_interval.tick() => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;

                // Rate limit status updates
                if now - last_status_update.load(Ordering::Relaxed) < MIN_STATUS_INTERVAL {
                    continue;
                }

                match state.player.get_status() {
                    Ok(status) => {
                        // Reuse buffer for status string
                        status_buffer.clear();
                        if let Ok(status_str) = serde_json::to_string(&status) {
                            status_buffer.push_str(&status_str);
                            if socket.send(Message::Text(status_buffer.clone())).await.is_ok() {
                                last_status_update.store(now, Ordering::Relaxed);
                                consecutive_errors = 0;
                            } else {
                                eprintln!("Non-fatal send error, will retry");
                                sleep(ERROR_BACKOFF).await;
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("Error getting player status: {}", e);
                        sleep(ERROR_BACKOFF).await;
                    }
                }
            }
            _ = ping_interval.tick() => {
                if last_pong.elapsed() > PING_TIMEOUT {
                    eprintln!("WebSocket ping timeout, client will reconnect");
                    break 'connection;
                }

                if socket.send(Message::Ping(vec![])).await.is_err() {
                    eprintln!("Non-fatal ping error, will retry");
                    consecutive_errors += 1;
                    if consecutive_errors >= MAX_CONSECUTIVE_ERRORS {
                        eprintln!("Too many errors, allowing client to reconnect");
                        break 'connection;
                    }
                }
            }
            result = socket.recv() => {
                match result {
                    Some(Ok(Message::Pong(_))) => {
                        last_pong = Instant::now();
                        consecutive_errors = 0;
                    }
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                            if let Some(command) = json.get("command").and_then(|v| v.as_str()) {
                                match command {
                                    "loadURL" => {
                                        if let Some(url) = json.get("url").and_then(|v| v.as_str()) {
                                            println!("Loading URL: {}", url);
                                            if let Err(e) = state.player.load_file(url) {
                                                eprintln!("Error loading URL: {}", e);
                                            }
                                        }
                                    },
                                    "play" => {
                                        // Don't hold the mutex guard across an await boundary
                                        let _is_paused = match state.player.get_handle() {
                                            Ok(handle) => {
                                                let paused = handle.get_property::<bool>("pause").unwrap_or(false);
                                                if paused {
                                                    if let Err(e) = handle.set_property("pause", false) {
                                                        eprintln!("Error playing: {}", e);
                                                    } else {
                                                        println!("Play command executed successfully");
                                                    }
                                                }
                                                paused
                                            },
                                            Err(e) => {
                                                eprintln!("Failed to get MPV handle: {}", e);
                                                false
                                            }
                                        };
                                    },
                                    "pause" => {
                                        // Don't hold the mutex guard across an await boundary
                                        let _is_paused = match state.player.get_handle() {
                                            Ok(handle) => {
                                                let paused = handle.get_property::<bool>("pause").unwrap_or(false);
                                                if !paused {
                                                    if let Err(e) = handle.set_property("pause", true) {
                                                        eprintln!("Error pausing: {}", e);
                                                    } else {
                                                        println!("Pause command executed successfully");
                                                    }
                                                }
                                                paused
                                            },
                                            Err(e) => {
                                                eprintln!("Failed to get MPV handle: {}", e);
                                                true
                                            }
                                        };
                                    },
                                    "seek" => {
                                        if let Some(position) = json.get("position").and_then(|v| v.as_f64()) {
                                            println!("Seeking to: {}", position);
                                            if let Err(e) = state.player.command("seek", &[&position.to_string(), "absolute"]) {
                                                eprintln!("Error seeking: {}", e);
                                            }
                                        }
                                    },
                                    "setOffset" => {
                                        // Get current playback info without holding the lock across await
                                        let current_position;
                                        let current_path;
                                        let is_playing;
                                        
                                        // This block ensures the handle is dropped before any await
                                        {
                                            let handle_result = state.player.get_handle();
                                            if let Err(e) = handle_result {
                                                eprintln!("Failed to get MPV handle: {}", e);
                                                continue;
                                            }
                                            
                                            let handle = handle_result.unwrap();
                                            current_position = handle.get_property::<f64>("time-pos").unwrap_or(0.0);
                                            current_path = handle.get_property::<String>("path").ok();
                                            is_playing = !handle.get_property::<bool>("pause").unwrap_or(true);
                                        } // handle is dropped here
                                        
                                        // Handle offset set in seconds
                                        if let Some(seconds) = json.get("seconds").and_then(|v| v.as_f64()) {
                                            println!("Setting offset to {} seconds", seconds);
                                            if let Err(e) = state.player.set_offset_seconds(seconds) {
                                                eprintln!("Error setting offset: {}", e);
                                                continue;
                                            }
                                            
                                            // Process path and seek/unpause commands (no MutexGuard involved)
                                            if let Some(path) = &current_path {
                                                if !path.is_empty() {
                                                    let path_str = path.clone();
                                                    if let Err(e) = state.player.load_file(&path_str) {
                                                        eprintln!("Error reloading file: {}", e);
                                                    } else {
                                                        // Schedule delayed commands
                                                        if current_position > 0.0 {
                                                            state.player.apply_delayed_command(
                                                                700,
                                                                "seek".to_string(),
                                                                vec![current_position.to_string(), "absolute".to_string()],
                                                            );
                                                            
                                                            if is_playing {
                                                                state.player.apply_delayed_command(
                                                                    800,
                                                                    "set".to_string(),
                                                                    vec!["pause".to_string(), "no".to_string()],
                                                                );
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                            
                                            // Prepare response
                                            let response = format!(
                                                "{{\"status\":\"success\",\"command\":\"offsetUpdated\",\"seconds\":{}}}",
                                                seconds
                                            );
                                            
                                            // Send confirmation to client
                                            if let Err(e) = socket.send(Message::Text(response)).await {
                                                eprintln!("Error sending websocket response: {}", e);
                                            }
                                        } 
                                        // Handle offset set in frames
                                        else if let Some(frames) = json.get("frames").and_then(|v| v.as_i64()) {
                                            let fps = json.get("fps").and_then(|v| v.as_f64()).unwrap_or(30.0);
                                            println!("Setting offset to {} frames at {} fps", frames, fps);
                                            if let Err(e) = state.player.set_offset_frames(frames as i32, fps) {
                                                eprintln!("Error setting frame offset: {}", e);
                                                continue;
                                            }
                                            
                                            // Process path and seek/unpause commands (no MutexGuard involved)
                                            if let Some(path) = &current_path {
                                                if !path.is_empty() {
                                                    let path_str = path.clone();
                                                    if let Err(e) = state.player.load_file(&path_str) {
                                                        eprintln!("Error reloading file: {}", e);
                                                    } else {
                                                        // Schedule delayed commands
                                                        if current_position > 0.0 {
                                                            state.player.apply_delayed_command(
                                                                700,
                                                                "seek".to_string(),
                                                                vec![current_position.to_string(), "absolute".to_string()],
                                                            );
                                                            
                                                            if is_playing {
                                                                state.player.apply_delayed_command(
                                                                    800,
                                                                    "set".to_string(),
                                                                    vec!["pause".to_string(), "no".to_string()],
                                                                );
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                            
                                            // Calculate seconds for response
                                            let seconds = (frames as f64) / fps;
                                            
                                            // Prepare response
                                            let response = format!(
                                                "{{\"status\":\"success\",\"command\":\"offsetUpdated\",\"frames\":{},\"seconds\":{},\"fps\":{}}}",
                                                frames, seconds, fps
                                            );
                                            
                                            // Send confirmation to client
                                            if let Err(e) = socket.send(Message::Text(response)).await {
                                                eprintln!("Error sending websocket response: {}", e);
                                            }
                                        }
                                    },
                                    "getOffset" => {
                                        let offset = state.player.get_offset_seconds();
                                        let response = format!(
                                            "{{\"status\":\"success\",\"command\":\"offsetStatus\",\"seconds\":{}}}",
                                            offset
                                        );
                                        
                                        if let Err(e) = socket.send(Message::Text(response)).await {
                                            eprintln!("Error sending offset status: {}", e);
                                        }
                                    },
                                    "setLoop" => {
                                        if let Some(enabled) = json.get("enabled").and_then(|v| v.as_bool()) {
                                            println!("Setting loop to: {}", enabled);
                                            
                                            if let Err(e) = state.player.set_loop(enabled) {
                                                eprintln!("Error setting loop: {}", e);
                                                continue;
                                            }
                                            
                                            // Send confirmation to client
                                            let response = format!(
                                                "{{\"status\":\"success\",\"command\":\"loopUpdated\",\"enabled\":{}}}",
                                                if enabled { "true" } else { "false" }
                                            );
                                            
                                            if let Err(e) = socket.send(Message::Text(response)).await {
                                                eprintln!("Error sending WebSocket response: {}", e);
                                            }
                                        }
                                    },
                                    "getLoop" => {
                                        match state.player.get_loop() {
                                            Ok(enabled) => {
                                                // Send loop status to client
                                                let response = format!(
                                                    "{{\"status\":\"success\",\"command\":\"loopStatus\",\"enabled\":{}}}",
                                                    if enabled { "true" } else { "false" }
                                                );
                                                
                                                if let Err(e) = socket.send(Message::Text(response)).await {
                                                    eprintln!("Error sending WebSocket response: {}", e);
                                                }
                                            },
                                            Err(e) => {
                                                eprintln!("Error getting loop status: {}", e);
                                            }
                                        }
                                    },
                                    _ => {}
                                }
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
                            eprintln!("Too many errors, allowing client to reconnect");
                            break 'connection;
                        }
                    }
                    None => {
                        eprintln!("WebSocket closed by client, allowing reconnect");
                        break 'connection;
                    }
                    _ => {}
                }
            }
        }
    }
    eprintln!("WebSocket connection ended, ready for client reconnect");
}

async fn status_page(
    AxumState(state): AxumState<Arc<AppState>>,
) -> Result<Html<String>, (StatusCode, String)> {
    // Cache the HTML template with the port number
    static HTML_TEMPLATE: Lazy<String> = Lazy::new(|| {
        // Include the HTML template directly in the binary as a fallback
        include_str!("../templates/status.html").to_string()
    });

    // Replace placeholders with actual values
    let html = HTML_TEMPLATE
        .replace("{port}", &state.port.to_string())
        .replace("{port}", &state.port.to_string())
        .replace("{port}", &state.port.to_string());

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
    const MIN_SEEK_INTERVAL: u64 = 50; // Minimum 50ms between seeks

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    // Check for rate limiting
    if now - state.last_seek.load(Ordering::Relaxed) < MIN_SEEK_INTERVAL {
        return Ok(Json(json!({
            "status": "rate_limited",
            "message": "Too many seek requests"
        })));
    }

    // Get time from payload
    let time = payload["time"].as_f64().ok_or((
        StatusCode::BAD_REQUEST,
        "Missing or invalid 'time' field".to_string(),
    ))?;

    // Get the current offset to adjust the seek position
    let offset = state.player.get_offset_seconds();
    let adjusted_time = time + offset; // Add offset to the requested time

    // Send seek command with adjusted time
    println!("Seeking to {} (adjusted from {} with offset {})", adjusted_time, time, offset);
    state
        .player
        .command("seek", &[&adjusted_time.to_string(), "absolute"])
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Update last seek time
    state.last_seek.store(now, Ordering::Relaxed);

    Ok(Json(json!({
        "status": "success",
        "time": time,
        "adjusted_time": adjusted_time,
        "offset": offset,
        "timestamp": now
    })))
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
    let is_paused = get_str("Paused") == "yes";
    let duration = get_f64("Duration");
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
        "position": position
    })))
}

// Add these handlers for offset functionality
async fn set_offset(
    AxumState(state): AxumState<Arc<AppState>>,
    Json(payload): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    // Get current playback position and state
    let current_position;
    let current_path;
    let is_playing;
    
    {
        let handle = state.player.get_handle()
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        current_position = handle.get_property::<f64>("time-pos").unwrap_or(0.0);
        current_path = handle.get_property::<String>("path").ok();
        is_playing = !handle.get_property::<bool>("pause").unwrap_or(true);
    } // handle is automatically dropped here when the scope ends
    
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

    // If currently playing a file, reload it to apply the offset
    if let Some(path) = current_path {
        if !path.is_empty() {
            println!("Reloading file to apply new offset: {}", path);
            
            // Reload the file to apply the new offset
            state.player.load_file(&path)
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
                
            // After a short delay, seek to the previous position if needed
            if current_position > 0.0 {
                // Schedule a seek command to return to the previous position
                state.player.apply_delayed_command(
                    700, // 700ms delay
                    "seek".to_string(),
                    vec![current_position.to_string(), "absolute".to_string()],
                );
                
                // If we were playing, schedule an unpause command
                if is_playing {
                    state.player.apply_delayed_command(
                        800, // 800ms delay (after seek)
                        "set".to_string(),
                        vec!["pause".to_string(), "no".to_string()],
                    );
                }
            }
        }
    }

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
    // production paths (resources next to exe)
    if let Some(ref dir) = exe_dir {
        candidates.push(dir.join("resources/templates"));
        candidates.push(dir.join("templates"));
    }

    // First path that exists wins
    for p in &candidates {
        if p.exists() {
            println!("Using templates directory: {:?}", p);
            return p.clone();
        }
    }

    // Fallback to cwd/src-tauri/templates
    let fallback = cwd.join("src-tauri/templates");
    println!("Templates directory not found, falling back to {:?}", fallback);
    fallback
}

#[tokio::main]
async fn main() {
    let (player, _exit_receiver) = MpvPlayer::new().expect("Failed to initialize MPV player");
    let player = Arc::new(player);

    // Determine port
    let port = match tokio::net::TcpListener::bind(("0.0.0.0", 3000)).await {
        Ok(listener) => { drop(listener); 3000 }
        Err(_) => pick_unused_port().expect("No ports available"),
    };
    let url = format!("http://localhost:{}", port);

    let app_state = Arc::new(AppState {
        player: Arc::clone(&player),
        port,
        last_seek: Arc::new(AtomicU64::new(0)),
    });

    // Define static files path (use a relative path that works in both dev and production)
    let static_path = find_templates_dir();

    println!("Looking for static files at: {:?}", static_path);
    
    // Set up the Axum server with the static path
    let axum_app = Router::new()
        .route("/ws", get(ws_handler))
        .route("/control/:action", post(control_player))
        .route("/room/:id", get(room_status))
        .route("/sync", post(sync))
        .route("/playback/time", get(get_playback_time))
        .route("/playback/time", post(set_playback_time))
        .route("/playback/offset", get(get_offset))
        .route("/playback/offset", post(set_offset))
        .route("/playback/loop", get(get_loop))
        .route("/playback/loop", post(set_loop))
        .route("/status", get(get_connection_status))
        .route("/", get(status_page))
        .nest_service("/static", ServeDir::new(static_path)) // Use the simple path
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        .with_state(app_state.clone());

    // Start the Axum server in a separate task
    let _server_handle = tokio::spawn({
        let router = axum_app.clone();
        let url_clone = url.clone();
        async move {
            match tokio::net::TcpListener::bind(format!("0.0.0.0:{}", port)).await {
                Ok(listener) => {
                    println!("Axum server running on {}", url_clone);
                    if let Err(e) = axum::serve(listener, router).await {
                        eprintln!("Axum server error: {}", e);
                    }
                }
                Err(e) => {
                    eprintln!("Failed to bind Axum server to port {}: {}", port, e);
                }
            }
        }
    });

    // Build and run the Tauri application
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_http::init())
        .manage(app_state.clone())
        .invoke_handler(tauri::generate_handler![sync_room, exit_app])
        .setup(move |app| {
            // Create the main window
            let window = tauri::WebviewWindowBuilder::new(
                app,
                "main",
                WebviewUrl::External(url.parse().unwrap())
            )
            .title("Alien")
            .inner_size(800.0, 600.0)
            .build()?;

            // Monitor MPV events
            let window_clone = window.clone();
            let player_clone = Arc::clone(&player);
            std::thread::spawn(move || {
                loop {
                    player_clone.check_events();
                    if player_clone.is_shutdown() {
                        println!("MPV player shutdown detected");
                        window_clone.close().unwrap();
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");

    println!("Application closing, cleaning up...");
}
