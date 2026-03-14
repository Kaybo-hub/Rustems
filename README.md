# Rustems

A desktop app for splitting any song into stems and uploading them to a compatible Kano Stem Player — built with [Tauri](https://tauri.app), Rust, and React.

![Platform](https://img.shields.io/badge/platform-Windows-lightgrey)
![License](https://img.shields.io/badge/license-MIT-green)

---

## What it does

Rustems has two main uses:

**Stem splitting** — pick any audio file (MP3, WAV, FLAC, AAC, OGG, M4A) and split it locally into four stems: vocals, drums, bass, and melody. Splitting runs entirely on your machine using the `htdemucs` model via ONNX Runtime. No audio is sent to any server. GPU is used with CPU automatically if available; splitting typically takes 1–3 minutes per track.

**Device management** — connect a compatible Kano Stem Player, browse the tracks already on it, upload new stem sets, and delete tracks or entire albums. Rustems communicates with the device directly over USB using a binary protocol.

After splitting, you can preview the stems with per-stem volume sliders, export them to a folder of your choice, or upload them straight to a connected device — all from the same screen.

---

## Features

- Split audio into 4 stems locally (vocals, drums, bass, melody)
- Preview stems with individual volume sliders before uploading
- Export split stems as MP3s to any folder
- Upload stems to a compatible Kano Stem Player
- Browse tracks on device with album/track metadata and colour swatches
- Delete individual tracks or entire albums from the device
- Live storage bar showing used/free space on the device
- Auto-detects connected Stem Players

---

## Requirements

### Building from source

- [Rust](https://rustup.rs) (stable, 1.77+)
- [Node.js](https://nodejs.org) (18+) and npm
- [Tauri prerequisites](https://tauri.app/start/prerequisites/) for Windows (WebView2)
- [Latest Visual Studio Community 2022/2026](https://visualstudio.microsoft.com/insiders/)

### Windows

No driver installation was needed during development. If the app fails to detect your device, try installing the WinUSB driver via [Zadig](https://zadig.akeo.ie).

---

## Building

```bash
# Install frontend dependencies
npm install

# Run in development mode
npm run tauri dev

# Build a release binary for your platform
npm run tauri build
```

---

## First run

On first use of the stem splitter, the app will automatically download the `htdemucs_ort_v1` model (~200 MB) to your system's app data directory. This should only happens once. 

The model is stored at `AppData\Local\StemSplitter\stem-splitter-core\cache\models`.

---

## Usage

### Splitting a song

1. Launch Rustems. The Stem Splitter panel will show **Ready** once the model is available.
2. Click **Pick file & split** and select an audio file.
3. Wait for splitting to complete (progress is shown; CPU splitting takes 2–5 mins).
4. Use the stem sliders and play/pause buttons to preview the result.
5. Optionally edit the track name.
6. Click **Export to folder…** to save the MP3s locally, or **Upload to device** to push them to a connected stem player.

### Manual upload

If you already have a folder of pre-split stems (`vocals.mp3`, `drums.mp3`, `bass.mp3`, `melody.mp3` or `other.mp3`), use the **Manual Upload** section to load and upload them directly without re-splitting.

### Managing device tracks

1. Select your device from the dropdown and click **Connect**.
2. The **On Device** section lists all albums and tracks with their metadata.
3. Click **Delete** next to any track, or **Delete album** to remove an entire album and all its tracks.

---

## Stem format

Stems are encoded as MP3 at 320 kbps, 44100 Hz stereo. Input audio in any supported format is automatically normalised to this spec before splitting. The four stem slots map to:

| Slot | File | Description |
|------|------|-------------|
| 1 | `melody.mp3` | Melody / other (everything that isn't vocals, drums, or bass) |
| 2 | `vocals.mp3` | Lead and backing vocals |
| 3 | `bass.mp3` | Bass |
| 4 | `drums.mp3` | Drums and percussion |

---

## Key dependencies

| Crate / Package | Purpose |
|-----------------|---------|
| `tauri` | Desktop app framework |
| `stem-splitter-core` | htdemucs ONNX inference |
| `symphonia` | Audio decoding (MP3, FLAC, WAV, AAC, OGG, M4A) |
| `rubato` | Sample rate conversion |
| `mp3lame-encoder` | MP3 encoding at 320 kbps |
| `hound` | WAV reading and writing |
| `rusb` | USB communication |
| `tokio` | Async runtime |

---

## License

Rustems is provided under the MIT license. Please refer to the LICENSE file for more details.

## Third-party notices

This application links against the following LGPL-licensed libraries:

- **libmp3lame** via [`mp3lame-encoder`](https://crates.io/crates/mp3lame-encoder) — LGPL-3.0

Audio decoding is provided by [Symphonia](https://github.com/pdeljanov/Symphonia) — MPL-2.0.

Source code for these components is available at their respective repositories linked above.