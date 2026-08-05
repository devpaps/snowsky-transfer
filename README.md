# Snowsky Transfer

Snowsky Transfer is a Linux desktop application built with Tauri 2 for transferring music to the **Snowsky Echo Mini** over USB/MTP.

**Version:** 0.1b

## Features

- Connect directly to a device through libmtp without an additional daemon
- Support for mounted USB storage devices
- Add files through a file picker or drag and drop
- Saved sync profiles for recurring music transfers
- Automatic format conversion with ffmpeg, confirmed per transfer
- Edit metadata including title, artist, album, genre, year, track number, and cover art
- Preview local audio files
- Manage and delete tracks on the device
- Create, update, and delete playlists
- Warn when the device is disconnected or powered off
- Show transfer progress with automatic and manual notification dismissal
- Follow the system light or dark theme

## Dependencies

```bash
# Fedora/RHEL
sudo dnf install libmtp libmtp-devel ffmpeg

# Rust + Node
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
curl -fsSL https://fnm.vercel.app/install | bash
fnm install --lts
```

## Getting Started

```bash
npm install
npm run tauri dev       # development
npm run tauri build     # production build
```

## Build from Source

Install the system dependencies, Rust, and Node.js first. Then clone the repository and build the application:

```bash
git clone <repository-url>
cd snowsky-transfer
npm install
npm run build
npm run tauri build
```

The frontend is built into `dist/`. Tauri production bundles are generated under `src-tauri/target/release/bundle/`.

## Udev Rule for Snowsky Echo Mini

This may be required to allow the application to access the device without elevated privileges.

Create `/etc/udev/rules.d/51-snowsky.rules`:

```
SUBSYSTEM=="usb", ATTR{idVendor}=="XXXX", ATTR{idProduct}=="YYYY", MODE="0664", GROUP="plugdev"
```

Replace `XXXX:YYYY` with the device VID:PID, which can be found with `lsusb`. Then add your user to the `plugdev` group.

## Project Structure

```
snowsky-transfer/
├── src-tauri/src/
│   ├── lib.rs        - Tauri commands (IPC)
│   ├── mtp.rs        - libmtp FFI and safe wrapper
│   ├── metadata.rs   - Read/write ID3 and FLAC tags (lofty)
│   ├── converter.rs  - Format conversion (ffmpeg CLI)
│   ├── audio.rs      - Audio preview (rodio)
│   └── error.rs      - Error types
└── src/
    ├── main.js       - Alpine.js frontend logic
    ├── style.css     - Tailwind v4 and custom styles
    └── index.html    - HTML shell
```
