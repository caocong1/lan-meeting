# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

LAN Meeting is a **Tauri 2 desktop app** for LAN-based screen sharing and collaboration. Uses SolidJS frontend with a Rust backend communicating via QUIC protocol. Target platforms: macOS, Windows, Linux.

## Tech Stack

| Layer | Technology |
|-------|-----------|
| **Frontend** | SolidJS + Solid Router, UnoCSS (atomic CSS), Kobalte UI components |
| **Desktop** | Tauri 2 (`@tauri-apps/api`, `@tauri-apps/cli`) |
| **Backend (Rust)** | `quinn` (QUIC), `wgpu` (GPU rendering), `winit` (native window), `mdns-sd` (service discovery) |
| **Video** | `openh264` (software), `ffmpeg-next` (hardware: NVENC/VideoToolbox/VAAPI), `gstreamer` (hardware decoding) |
| **Capture** | macOS: ScreenCaptureKit via `objc2`, Windows: DXGI, Linux: platform-specific |
| **Input** | `enigo` (cross-platform input simulation) |
| **Build** | `bun`, `vite`, `tauri build` |
| **Package Manager** | Bun (`bun.lock`) |

## Key Commands

```bash
# Frontend development
bun run dev          # Start Vite dev server (port 5173, strict)
bun run build        # Build frontend to dist/
bun run preview      # Preview built frontend
bun run check        # TypeScript type check (tsc --noEmit)

# Full desktop development (frontend + Tauri)
bun run tauri:dev    # Full app with hot-reload
bun run tauri:dev:win # Windows dev (uses dev-win.cmd)

# Desktop builds
bun run tauri build  # Production build

# Direct Tauri CLI
bunx tauri <args>
```

### Platform Notes

- **macOS**: Requires Screen Recording permission. Uses ScreenCaptureKit (macOS 12.3+ minimum).
- **Windows**: Uses DXGI capture + Direct3D 11.
- **Linux**: Requires GStreamer, optional PipeWire/Wayland/X11 features.
- **Hardware acceleration**: NVENC (NVIDIA), VideoToolbox (macOS), VAAPI (Linux) — falls back to OpenH264 software encoder.

## Architecture

### Two-Entry Frontend

- `index.html` → `src/index.tsx` → `App.tsx` — Main application window
- `viewer.html` → `src/viewer.tsx` → `Viewer` component — Native screen viewer window (opened per-peer via Tauri)

Vite builds both entry points; Tauri opens viewer windows via `WebviewWindowBuilder` pointing to `/viewer.html`.

### Rust Backend Modules (`src-tauri/src/`)

| Module | Purpose |
|--------|---------|
| `capture/` | Platform-specific screen capture (macos.rs, windows.rs, linux.rs) |
| `network/` | QUIC transport (`quic.rs`), mDNS discovery (`discovery.rs`), binary protocol (`protocol.rs`) |
| `streaming/` | Main encode→send / receive→decode pipeline |
| `simple_streaming/` | Minimal OpenH264 pipeline (debug/fallback) |
| `encoder/` | Hardware encoders (nvenc, videotoolbox, vaapi, ffmpeg), software encoder, scaler |
| `decoder/` | Hardware decoders (dxva, vaapi, videotoolbox, vulkan), software decoder |
| `renderer/` | `wgpu` GPU rendering + `winit` native window management |
| `input/` | Remote control (keyboard/mouse simulation via enigo) |
| `chat/` | Chat message storage |
| `transfer/` | File transfer with SHA-256 checksums |
| `commands/` | All Tauri command handlers (frontend ↔ Rust bridge) |

### Key Global State (Rust)

- `QUIC_ENDPOINT` — Global OnceCell QUIC endpoint (`lib.rs`)
- `APP_HANDLE` — Global Tauri app handle for events
- `SETTINGS` — RwLock<AppSettings> persisted to `~/.config/lan-meeting/settings.json`
- `CAPTURE` — Lazy<Mutex<ScreenCapture>> singleton
- `SERVICE_RUNNING` — RwLock<bool> for network service state

### Communication Protocol

Binary protocol defined in `network/protocol.rs`. Messages include: Handshake, HandshakeAck, ScreenOffer, ScreenStart, ScreenStop, ScreenRequest, ChatMessage, ControlRequest, ControlGrant, ControlRelease, InputEvent, FileOffer, FileAccept, FileReject, FileCancel, SimpleScreenRequest, and more.

### Video Pipeline

```
Capture (platform-specific) → Scaler → Encoder (HW/SW H264) → QUIC stream → Decoder (HW/SW) → wgpu renderer → winit window
```

### Frontend Components

- `MeetingRoom` — Main meeting UI (device list, chat, screen sharing controls)
- `ScreenShare` — Screen sharing controls (display selection, start/stop)
- `DeviceList` — Discovered devices + manual add
- `Chat` — Text messaging with code snippet support
- `FileTransfer` — File send/receive UI
- `Settings` — App settings (device name, quality, FPS, defaults)
- `Viewer` — Stream viewer component for viewer.html entry
- `AddDeviceModal` — Manual IP entry dialog

### Store

`src/stores/app.ts` — SolidJS store with signals for: selfInfo, connectionState, devices, chatMessages, fileTransfers, sharing status.

## Development Tips

- The frontend communicates with Rust via `@tauri-apps/api/core` `invoke()` calls matching `#[tauri::command]` functions in `commands/mod.rs`
- Events flow from Rust → frontend via `app_handle.emit()` (Tauri events)
- QUIC port: 19876 (defined in `network/quic.rs`)
- Two streaming pipelines exist: `streaming/` (full HW-accelerated) and `simple_streaming/` (OpenH264 fallback)
- Rust edition 2024, MSRV 1.85
