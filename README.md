# Alien

A video synchronization tool for animation reference on Roblox Studio. This tool allows you to sync video playback across multiple instances, making it easier to work on animations with reference videos. Currently, only Moon Animator is supported.

## Features

- **Video Synchronization**: Sync video playback across multiple instances
- **MPV Integration**: High-performance video playback using MPV
- **YouTube Support**: Play YouTube videos directly (requires yt-dlp), paste the link to the video.
- **Cross-Platform**: Works on Windows, macOS, and Linux. (probably, just gotta compile it)

## Installation

### Windows

1. Download the latest release from the [releases page](https://github.com/cautioned/alien/releases/latest)
2. Run the installer
3. The app will automatically install required dependencies

## Usage

1. Launch Alien
2. Use the port number provided in the app window to connect to the Roblox Plugin. The port number is displayed in the window.

### Controls

- **Play/Pause**: Space bar or click the play/pause button
- **Seek**: Use the timeline slider or arrow keys
- **Volume**: Use the volume slider or up/down arrow keys
- **Fullscreen**: F key or double-click the video

## Troubleshooting

### Common Issues

1. **Video won't play**
   - Check if MPV is installed correctly
   - Try restarting the application
   - Verify the video file is supported

2. **YouTube videos don't work**
   - Ensure yt-dlp is installed (On windows you can install it with winget, open terminal/powershell and run `winget install yt-dlp`)
   - Check your internet connection
   - Try a different video URL

### Technical Details

The application is built with:
- Tauri (Rust backend)
- MPV for video playback
- WebSocket for synchronization
- Axum for the web server

## License

[MIT License](LICENSE)
