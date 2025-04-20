# Alien

A video synchronization tool for animation reference on Roblox Studio. This tool allows you to sync video playback across multiple instances, making it easier to work on animations with reference videos. Currently, only Moon Animator is supported.

## Features

- **Video Synchronization**: Sync video playback across multiple instances
- **MPV Integration**: High-performance video playback using MPV
- **YouTube Support**: Play YouTube videos directly (requires yt-dlp), paste the link to the video.
- **Cross-Platform**: Works on Windows, macOS, and Linux. (probably, just gotta compile it)

## Installation

1.  Go to the [latest release page](https://github.com/Cautioned/Alien/releases/latest).
2.  Scroll down to the **Assets** section.
3.  Download the correct file for your operating system and architecture:
    *   **Windows:** Look for the `.msi` file (e.g., `Alien_x.y.z_x64_en-US.msi`).
    *   **macOS (Apple Silicon/ARM):** Look for the `_aarch64.dmg` file (e.g., `Alien_x.y.z_aarch64.dmg`).
    *   **macOS (Intel):** Look for the `_x64.dmg` file (e.g., `Alien_x.y.z_x64.dmg`).
    *   _(macOS Alternative): If a `.dmg` is not available for your architecture, look for the corresponding `.app.tar.gz` file and extract it._
    *   **Linux:** Look for the `.AppImage` or `.deb` file.
4.  Run the downloaded installer (`.msi`, `.dmg`) or the extracted application/AppImage.
5.  The required `mpv` library is bundled on Windows and macOS. **Linux users need to install `libmpv` separately** using their distribution's package manager (e.g., `sudo apt install libmpv1` on Debian/Ubuntu, `sudo pacman -S mpv` on Arch).

Windows Defender may falsely think the player is a trojan virus, so if that happens don't worry it's not, I've submitted it to microsoft for review it should clear up soon.

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
