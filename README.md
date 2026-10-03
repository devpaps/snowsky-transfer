# Snowsky Transfer

Snowsky Transfer is a Linux desktop application built with Tauri 2 for transferring music to the **Snowsky Echo Mini** over USB/MTP or mounted USB storage.

The app copies supported audio files as-is. It does not transcode files or improve audio quality.

**Version:** 0.1b

## Features

- Connect directly to a device through libmtp without an additional daemon
- Support for mounted USB storage devices
- Add files through a file picker or drag and drop
- Transfer supported Echo Mini formats: MP3, FLAC, OGG, WAV, and M4A
- Saved sync profiles for recurring music transfers
- Edit metadata including title, artist, album, genre, year, track number, and cover art
- Preview local audio files
- Manage and delete tracks on the device
- Warn when the device is disconnected or powered off
- Show transfer progress with automatic and manual notification dismissal
- Follow the system light or dark theme

## Dependencies

```bash
# Fedora/RHEL
sudo dnf install libmtp libmtp-devel

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

## Project Structure

```
snowsky-transfer/
├── src-tauri/src/
│   ├── lib.rs        - Tauri commands (IPC)
│   ├── mtp.rs        - libmtp FFI and safe wrapper
│   ├── metadata.rs   - Read/write ID3 and FLAC tags (lofty)
│   ├── audio.rs      - Audio preview (rodio)
│   └── error.rs      - Error types
└── src/
    ├── main.js       - Alpine.js frontend logic
    ├── style.css     - Tailwind v4 and custom styles
    └── index.html    - HTML shell
```
