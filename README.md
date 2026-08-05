# Snowsky Transfer

Skrivbordsapp för Linux (Tauri 2) som överför musik till **Snowsky Echo Mini** via USB/MTP.

## Funktioner

- ❄️ Anslut enhet via libmtp (direkt, ingen extra daemon)
- 📂 Lägg till filer via filväljare eller drag & drop
- 🔁 Automatisk formatkonvertering (ffmpeg) — frågar per överföring
- 🏷️ Metadata-redigering (titel, artist, album, genre, år, spårnr, albumomslag)
- 👂 Förhandslyssning av lokala filer
- 🗑️ Hantera och radera låtar på enheten
- 🎵 Spellistor: skapa, uppdatera, radera
- 🌓 Följer systemtema (ljust/mörkt)

## Beroenden

```bash
# Fedora/RHEL
sudo dnf install libmtp libmtp-devel ffmpeg

# Rust + Node
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
curl -fsSL https://fnm.vercel.app/install | bash
fnm install --lts
```

## Komma igång

```bash
npm install
cargo tauri dev        # development
cargo tauri build      # production build
```

## Udev-regel för Snowsky Echo Mini (kräver vid behov)

Skapa `/etc/udev/rules.d/51-snowsky.rules`:

```
SUBSYSTEM=="usb", ATTR{idVendor}=="XXXX", ATTR{idProduct}=="YYYY", MODE="0664", GROUP="plugdev"
```

Ersätt `XXXX:YYYY` med enhetens VID:PID (hitta med `lsusb`). Lägg sedan till din användare i gruppen `plugdev`.

## Projektstruktur

```
snowsky-transfer/
├── src-tauri/src/
│   ├── lib.rs        – Tauri-kommandon (IPC)
│   ├── mtp.rs        – libmtp FFI + säker wrapper
│   ├── metadata.rs   – Läs/skriv ID3/FLAC-taggar (lofty)
│   ├── converter.rs  – Formatkonvertering (ffmpeg CLI)
│   ├── audio.rs      – Förhandslyssning (rodio)
│   └── error.rs      – Feltyper
└── src/
    ├── main.js       – Alpine.js frontend-logik
    ├── style.css     – Tailwind v4 + anpassade stilar
    └── index.html    – HTML-skal
```
