# 跨平台开发环境设置指南

## 前置要求

- **Rust** >= 1.85 (edition 2024)
- **Node.js** >= 18 (推荐用 bun)
- **Tauri CLI**: `cargo install tauri-cli` 或 `bun add -D @tauri-apps/cli`

## Windows 设置

### 1. 安装 Visual Studio Build Tools

安装 [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/)，勾选:
- MSVC v143 (x86/x64)
- Windows 11 SDK
- C++ CMake tools

### 2. 安装 Rust x64 工具链

如果你的 Windows 是 ARM64，需要添加 x64 交叉编译目标:

```bash
rustup target add x86_64-pc-windows-msvc
```

### 3. 安装 GStreamer (视频硬件解码)

1. 下载 GStreamer **MSVC x86_64** 安装包: https://gstreamer.freedesktop.org/download/
   - 需要安装 **Runtime** 和 **Development** 两个包
   - 安装路径: `C:\Program Files\gstreamer\1.0\msvc_x86_64\`
2. 安装时选择 **Complete** 安装（确保包含 d3d11、nvcodec 等插件）

### 4. 安装 FFmpeg (视频编码)

1. 下载预编译的 FFmpeg **shared** 版本: https://github.com/BtbN/FFmpeg-Builds/releases
   - 选择 `ffmpeg-n7.1-latest-win64-gpl-shared-7.1.zip` 或类似版本
2. 解压到: `C:\tools\ffmpeg-n7.1-latest-win64-gpl-shared-7.1\`
   - 确保目录结构为: `C:\tools\ffmpeg-n7.1-latest-win64-gpl-shared-7.1\bin\`, `lib\`, `include\` 等

### 5. 安装前端依赖

```bash
bun install
# 或
npm install
```

### 6. 运行开发模式

使用项目提供的 `dev-win.cmd` 脚本（已配置好所有环境变量）:

```bash
# 方式 1: 直接运行
dev-win.cmd

# 方式 2: 通过 npm/bun
bun run tauri:dev:win
```

`dev-win.cmd` 会自动设置以下环境变量:
- `PKG_CONFIG` - 指向 GStreamer 的 pkg-config
- `PKG_CONFIG_PATH` - GStreamer 和 FFmpeg 的 .pc 文件路径
- `PKG_CONFIG_ALLOW_CROSS` - 允许交叉编译
- `FFMPEG_DIR` - FFmpeg 安装目录
- `PATH` - 添加 GStreamer 和 FFmpeg 的 bin 目录

> **注意**: 如果你的安装路径不同，需要修改 `dev-win.cmd` 中的路径。

### 7. 运行时 DLL

应用运行时需要以下 DLL 在 PATH 中或与可执行文件同目录:
- GStreamer DLL（`gstreamer-1.0-0.dll` 等）
- FFmpeg DLL（`avcodec-61.dll`, `avformat-61.dll`, `avutil-59.dll` 等）

`dev-win.cmd` 已通过 PATH 设置解决此问题。

## macOS 设置

### 1. 安装 Xcode Command Line Tools

```bash
xcode-select --install
```

### 2. 安装 GStreamer

```bash
brew install gstreamer gst-plugins-base gst-plugins-good gst-plugins-bad
```

或下载官方安装包: https://gstreamer.freedesktop.org/download/

### 3. 安装 FFmpeg

```bash
brew install ffmpeg
```

### 4. 运行

```bash
bun install
cargo tauri dev
```

macOS 的 VideoToolbox 硬件编解码已自动启用，无需额外配置。

> **注意**: 首次运行需要在系统设置中授予屏幕录制权限。

## Linux 设置

### 1. 安装系统依赖 (Ubuntu/Debian)

```bash
# 基础构建工具
sudo apt install build-essential pkg-config libssl-dev

# Tauri 依赖
sudo apt install libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev

# GStreamer
sudo apt install libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev \
  gstreamer1.0-plugins-good gstreamer1.0-plugins-bad gstreamer1.0-vaapi

# FFmpeg
sudo apt install libavcodec-dev libavformat-dev libavutil-dev libswscale-dev libavdevice-dev
```

### 2. 可选功能

```bash
# PipeWire 屏幕捕获
cargo tauri dev --features pipewire

# Wayland 支持
cargo tauri dev --features wayland

# X11 支持
cargo tauri dev --features x11
```

### 3. 运行

```bash
bun install
cargo tauri dev
```

## 项目架构

```
lan-meeting/
├── src/                    # 前端 (SolidJS + TypeScript)
│   ├── components/         # UI 组件
│   └── ...
├── src-tauri/              # 后端 (Rust + Tauri 2)
│   ├── src/
│   │   ├── capture/        # 屏幕捕获 (DXGI/ScreenCaptureKit)
│   │   ├── encoder/        # 视频编码 (OpenH264/FFmpeg)
│   │   ├── decoder/        # 视频解码 (GStreamer 硬件加速)
│   │   ├── network/        # P2P 网络 (QUIC + mDNS)
│   │   ├── renderer/       # GPU 渲染 (wgpu)
│   │   ├── streaming/      # 流媒体管理
│   │   └── ...
│   └── Cargo.toml
├── dev-win.cmd             # Windows 开发启动脚本
└── package.json
```

## 常见问题

### Windows: `pkg-config has not been configured to support cross-compilation`

确保设置了 `PKG_CONFIG_ALLOW_CROSS=1` 环境变量。使用 `dev-win.cmd` 可自动处理。

### Windows: 找不到 GStreamer/FFmpeg

检查安装路径是否与 `dev-win.cmd` 中配置的一致。

### macOS: 屏幕捕获无画面

在「系统设置 > 隐私与安全性 > 屏幕录制」中授权应用。

### 连接超时

确保两端设备在同一局域网，且 UDP 端口 19876 未被防火墙阻止:

```powershell
# Windows - 以管理员身份运行
New-NetFirewallRule -DisplayName "LAN Meeting" -Direction Inbound -Protocol UDP -LocalPort 19876 -Action Allow
New-NetFirewallRule -DisplayName "LAN Meeting Out" -Direction Outbound -Protocol UDP -LocalPort 19876 -Action Allow
```
