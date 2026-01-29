# LAN Meeting - 局域网会议工具

## 项目概述

面向开发团队的局域网内部会议工具，支持多人会议、屏幕共享、远程控制、文字聊天和文件传输。

**设计目标**: 性能至上，追求极致低延迟 (<30ms) 和高画质

## 需求规格

| 需求项 | 描述 |
|--------|------|
| 目标平台 | macOS / Windows / Linux |
| 网络环境 | 仅局域网，P2P 直连 |
| 参会人数 | 2-5 人 |
| 音频 | 不需要 |
| 录制 | 不需要 |
| 设备发现 | mDNS 自动发现 + 手动 IP |

## 多人会议架构

### 会议模型 (实际实现)

```
┌─────────────────────────────────────────────────────────────┐
│                        会议室 (Meeting Room)                  │
│                                                              │
│  ┌──────────────────────────────────────────────────────┐   │
│  │                  对等成员 (Peers)                      │   │
│  │  ┌─────┐  ┌─────┐  ┌─────┐  ┌─────┐  ┌─────┐        │   │
│  │  │  A  │  │  B  │  │  C  │  │  D  │  │  E  │        │   │
│  │  └─────┘  └─────┘  └─────┘  └─────┘  └─────┘        │   │
│  └──────────────────────────────────────────────────────┘   │
│                                                              │
│  特点：                                                      │
│  - 无主持人，所有成员对等                                      │
│  - 任意成员可随时开始/停止共享屏幕                              │
│  - 其他成员看到"正在共享"状态后可点击观看                        │
│  - 观看者可请求远程控制（待实现）                               │
│  - 全员可发送聊天消息、传输文件                                 │
└─────────────────────────────────────────────────────────────┘
```

### 连接拓扑

```
    全网状 P2P (Mesh Topology)

         ┌─────┐
         │  A  │ ◄─── 正在共享屏幕
         └──┬──┘
           /│\
          / │ \   屏幕流 (1-to-N)
         /  │  \
    ┌───┐   │   ┌───┐
    │ B │───┼───│ C │  ◄─── 观看者
    └───┘   │   └───┘
         \  │  /
          \ │ /
           \│/
         ┌─────┐
         │  D  │  ◄─── 未观看
         └─────┘

- mDNS 发现 + 手动 IP 添加
- QUIC P2P 直连（自签名证书）
- 屏幕流: 共享者 → 所有观看者 (1-to-N 广播)
- 控制消息: 点对点发送
```

### 角色说明 (简化模型)

| 状态 | 说明 |
|------|------|
| **共享者** | 正在共享屏幕的成员，可同时有多个共享者 |
| **观看者** | 正在观看某个共享者屏幕的成员 |
| **空闲成员** | 在线但未共享/观看的成员 |

注：原计划的主持人/演示者/控制者角色未实现，当前为简单的对等模型

---

## 性能目标

| 指标 | 目标值 |
|------|--------|
| 端到端延迟 | < 30ms |
| 分辨率 | 1080p @ 60fps |
| 码率 | 4-8 Mbps (自适应) |
| CPU 占用 | < 15% |

## 功能优先级

### Phase 1 - 核心功能
1. **屏幕共享** - 主要功能，支持选择窗口/全屏
2. **远程控制** - 请求控制对方屏幕，模拟键鼠输入
3. **设备发现** - mDNS 自动发现 + 手动输入 IP

### Phase 2 - 协作功能
4. **文字聊天** - 实时消息，支持代码片段
5. **文件传输** - P2P 直传文件

### Phase 3 - 扩展功能
6. **白板协作** - 类似 draw.io 的协作画图

---

## 极致性能架构 (实际实现)

```
┌─────────────────────────────────────────────────────────────────────┐
│                           Tauri App                                 │
├─────────────────────┬───────────────────────────────────────────────┤
│  Frontend (WebView) │              Backend (Rust)                   │
│  ─────────────────  │  ───────────────────────────────────────────  │
│  SolidJS + Bun      │                                               │
│  ├── 会议室 UI       │  ┌─────────────────────────────────────────┐  │
│  ├── 成员列表        │  │  capture/ (平台原生) ✅                   │  │
│  ├── 共享开关        │  │  ├── macos.rs   → CGDisplayCreateImage  │  │
│  └── 设置界面        │  │  ├── windows.rs → DXGI Desktop Dup      │  │
│                     │  │  └── linux.rs   → X11 XGetImage          │  │
│                     │  └─────────────────────────────────────────┘  │
│                     │                                               │
│ ┌─────────────────┐ │  ┌─────────────────────────────────────────┐  │
│ │ wgpu 原生窗口    │ │  │  encoder/ (FFmpeg 硬件加速) ✅            │  │
│ │ (独立进程渲染)   │◄┼──│  ├── ffmpeg/   → NVENC/VT/VAAPI/QSV    │  │
│ │                 │ │  │  └── software.rs → OpenH264 (回退)      │  │
│ │ ViewerSession   │ │  └─────────────────────────────────────────┘  │
│ │ ├─ Vulkan解码   │ │                                               │
│ │ └─ GPU渲染      │ │                                               │
│ └─────────────────┘ │                                               │
│                     │  ┌─────────────────────────────────────────┐  │
│                     │  │  network/ ✅                             │  │
│                     │  │  ├── quic.rs      → QUIC (quinn)        │  │
│                     │  │  ├── protocol.rs → 消息协议              │  │
│                     │  │  └── discovery.rs → mDNS                │  │
│                     │  └─────────────────────────────────────────┘  │
│                     │                                               │
│                     │  ┌─────────────────────────────────────────┐  │
│                     │  │  streaming/ ✅                           │  │
│                     │  │  ├── StreamingManager → 发送端           │  │
│                     │  │  └── ViewerSession   → 接收端+渲染       │  │
│                     │  └─────────────────────────────────────────┘  │
│                     │                                               │
│                     │  ┌─────────────────────────────────────────┐  │
│                     │  │  decoder/ (Vulkan Video 硬件) ✅         │  │
│                     │  │  ├── vulkan/  → vk-video (Win/Linux)   │  │
│                     │  │  └── software.rs → OpenH264 (回退)      │  │
│                     │  └─────────────────────────────────────────┘  │
│                     │                                               │
│                     │  ┌─────────────────────────────────────────┐  │
│                     │  │  renderer/ ✅                            │  │
│                     │  │  ├── wgpu_renderer.rs → GPU 渲染        │  │
│                     │  │  └── window.rs → 独立原生窗口            │  │
│                     │  └─────────────────────────────────────────┘  │
└─────────────────────┴───────────────────────────────────────────────┘

✅ = 已实现    ⏳ = 存根/待实现
```

---

## 延迟优化链路 (当前实现)

```
捕获           编码               传输          解码              渲染
─────────────────────────────────────────────────────────────────────────
平台原生     →  FFmpeg (HW)    →  QUIC      →  Vulkan Video   →  wgpu GPU
(CGImage/      (VideoToolbox/    (quinn)      (vk-video)        (原生窗口)
 DXGI/X11)      NVENC/VAAPI)
  ↓               ↓               ↓              ↓                ↓
~5-10ms        ~2-5ms          ~1-5ms        ~2-5ms           ~2-5ms
─────────────────────────────────────────────────────────────────────────
                        总延迟: ~12-30ms (硬件加速)
                               ~25-50ms (软件回退)
```

**硬件加速优化**:
- FFmpeg 硬件编码: VideoToolbox/NVENC/VAAPI/QSV
- Vulkan Video 硬件解码 (Windows/Linux, vk-video)
- macOS 暂时使用 OpenH264 软件解码 (Vulkan 不支持)

**软件回退链**:
- 编码: FFmpeg HW → libx264 → OpenH264
- 解码: Vulkan Video → OpenH264

**已优化**:
- Rust 原生渲染，无 WebView IPC 开销
- 直接 GPU 纹理上传
- 帧率控制避免积压
- 自动硬件检测和回退

---

## 技术选型

### 框架层
| 组件 | 技术 | 说明 |
|------|------|------|
| 应用框架 | Tauri 2.x | 跨平台桌面应用 |
| 前端框架 | SolidJS | 响应式 UI |
| 构建工具 | Bun | 快速的 JS 运行时和打包 |
| 后端语言 | Rust | Tauri 原生支持 |

### 屏幕捕获 (平台原生)

| 平台 | API | Rust 绑定 | 特点 |
|------|-----|-----------|------|
| macOS | ScreenCaptureKit | `screencapturekit-rs` | 零拷贝，支持 HDR |
| Windows | DXGI Desktop Duplication | `windows` crate | GPU 直接访问 |
| Linux | PipeWire + DMA-BUF | `pipewire-rs` | Wayland 原生 |

### 视频编码 (FFmpeg 硬件加速)

| 平台 | 编码器 | 状态 | 说明 |
|------|--------|------|------|
| macOS | VideoToolbox (via FFmpeg) | ✅ 已实现 | FFmpeg h264_videotoolbox |
| Windows | NVENC (via FFmpeg) | ✅ 已实现 | FFmpeg h264_nvenc |
| Windows | QuickSync (via FFmpeg) | ✅ 已实现 | FFmpeg h264_qsv (Intel) |
| Linux | VAAPI (via FFmpeg) | ✅ 已实现 | FFmpeg h264_vaapi |
| Linux | NVENC (via FFmpeg) | ✅ 已实现 | FFmpeg h264_nvenc |
| 全平台 | libx264 (via FFmpeg) | ✅ 已实现 | FFmpeg 软件编码 (fallback) |
| 全平台 | OpenH264 | ✅ 已实现 | 最终回退编码器 |

**编码器选择优先级** (自动检测):
```
macOS:     VideoToolbox → libx264 → OpenH264
Windows:   NVENC → QSV → libx264 → OpenH264
Linux:     NVENC → VAAPI → QSV → libx264 → OpenH264
```

**实际编码配置**:
```rust
struct EncoderConfig {
    width: u32,
    height: u32,
    fps: 30,                      // 默认 30 fps
    bitrate: 8_000_000,           // 8 Mbps (High)
    max_bitrate: 16_000_000,      // 峰值 16 Mbps
    keyframe_interval: 30,        // 1秒一个关键帧
    preset: EncoderPreset::UltraFast,
}
```

**FFmpeg 编码器特点** (encoder/ffmpeg/mod.rs):
- 自动检测可用硬件编码器
- 低延迟配置 (zerolatency tune, CBR rate control)
- BGRA → YUV420 颜色空间转换
- 支持动态请求关键帧
- 硬件编码失败时自动回退到软件

### 网络传输

| 组件 | 技术 | 说明 |
|------|------|------|
| 传输协议 | QUIC (quinn) | 低延迟 + 加密 |
| 服务发现 | mDNS (mdns-sd) | 局域网自动发现 |
| 序列化 | bincode | 高效二进制 |

**帧优先级策略**:
```rust
enum FramePriority {
    /// I帧 - 必须可靠送达，支持重传
    KeyFrame { retries: u8 },
    /// P帧 - 尽力送达，超时丢弃
    DeltaFrame { deadline_ms: u16 },
}

struct NetworkConfig {
    /// 关键帧重传次数
    keyframe_retries: 3,
    /// P帧最大等待时间
    delta_frame_deadline_ms: 16,  // 约1帧时间
    /// 启用拥塞控制
    congestion_control: true,
}
```

### 渲染 (已实现)

| 组件 | 技术 | 说明 |
|------|------|------|
| GPU 渲染 | wgpu | 跨平台 GPU API (Vulkan/Metal/DX12) |
| 窗口管理 | winit | 独立原生渲染窗口 |
| 帧格式 | BGRA / YUV420 | 支持两种输入格式 |

**渲染架构**:
```
观看者点击"观看"
    → request_screen_stream(peer_ip, peer_name)
    → 创建 ViewerSession
    → 收到 ScreenStart
    → RenderWindow::create() 创建原生窗口
    → 收到 ScreenFrame
    → OpenH264 解码 → BGRA 数据
    → RenderWindowHandle::render_frame()
    → wgpu 纹理上传 → GPU 渲染
```

**实现细节**:
- `RenderWindow` - 独立线程运行的 winit 窗口
- `RenderWindowHandle` - 跨线程控制句柄 (channel 通信)
- `WgpuRenderer` - BGRA 纹理 + YUV420 三平面着色器
- 低延迟 Mailbox 呈现模式
- 自动丢弃旧帧，保持最新帧显示

### 远程控制

| 平台 | 输入捕获 | 输入模拟 |
|------|---------|----------|
| macOS | CGEvent | CGEvent |
| Windows | RawInput | SendInput |
| Linux | libinput | uinput |

使用 `enigo` crate 作为跨平台抽象，必要时直接调用平台 API

### 前端库
| 功能 | 库 | 说明 |
|------|-----|------|
| UI 组件 | `@kobalte/core` | SolidJS 无障碍组件库 |
| 样式 | `UnoCSS` | 原子化 CSS |
| 状态管理 | SolidJS 内置 | createSignal/createStore |
| 图标 | `unplugin-icons` | 按需加载图标 |

---

## 项目结构

```
lan-meeting/
├── src/                        # 前端源码
│   ├── components/
│   │   ├── ScreenShare/        # 屏幕共享控制
│   │   ├── RemoteControl/      # 远程控制
│   │   ├── Chat/               # 聊天组件
│   │   ├── FileTransfer/       # 文件传输
│   │   ├── DeviceList/         # 设备列表
│   │   ├── Meeting/            # 会议室组件
│   │   └── common/             # 通用组件
│   ├── stores/                 # 状态管理
│   ├── utils/                  # 工具函数
│   ├── App.tsx
│   └── index.tsx
│
├── src-tauri/                  # Rust 后端
│   ├── src/
│   │   ├── main.rs
│   │   ├── lib.rs
│   │   │
│   │   ├── capture/            # 屏幕捕获 (平台原生)
│   │   │   ├── mod.rs          # 统一接口
│   │   │   ├── macos.rs        # ScreenCaptureKit
│   │   │   ├── windows.rs      # DXGI
│   │   │   └── linux.rs        # PipeWire
│   │   │
│   │   ├── encoder/            # 视频编码 (硬件优先)
│   │   │   ├── mod.rs          # 统一接口 + 自动选择
│   │   │   ├── videotoolbox.rs # macOS
│   │   │   ├── nvenc.rs        # NVIDIA
│   │   │   ├── vaapi.rs        # Linux/AMD
│   │   │   ├── qsv.rs          # Intel
│   │   │   └── software.rs     # x264 回退
│   │   │
│   │   ├── decoder/            # 视频解码 (硬件优先)
│   │   │   ├── mod.rs
│   │   │   ├── videotoolbox.rs
│   │   │   ├── nvdec.rs
│   │   │   ├── vaapi.rs
│   │   │   └── software.rs     # 软解码回退
│   │   │
│   │   ├── renderer/           # GPU 渲染
│   │   │   ├── mod.rs
│   │   │   ├── wgpu_renderer.rs
│   │   │   └── window.rs       # 独立渲染窗口
│   │   │
│   │   ├── network/            # 网络通信
│   │   │   ├── mod.rs
│   │   │   ├── quic.rs         # QUIC 传输
│   │   │   ├── session.rs      # 会议室会话管理
│   │   │   ├── priority.rs     # 帧优先级
│   │   │   ├── discovery.rs    # mDNS 发现
│   │   │   └── protocol.rs     # 消息协议
│   │   │
│   │   ├── input/              # 输入控制
│   │   │   ├── mod.rs
│   │   │   ├── capture.rs      # 输入捕获
│   │   │   └── simulate.rs     # 输入模拟
│   │   │
│   │   ├── transfer/           # 文件传输
│   │   │   └── mod.rs
│   │   │
│   │   └── commands/           # Tauri 命令
│   │       └── mod.rs
│   │
│   ├── Cargo.toml
│   └── tauri.conf.json
│
├── package.json
├── bunfig.toml
├── tsconfig.json
├── uno.config.ts
└── vite.config.ts
```

---

## 通信协议设计 (已实现)

注：原计划的 `MeetingMessage` (会议室管理、主持人、演示者角色) 未实现，
当前为简化的对等模型，所有设备对等，任意设备可共享屏幕。

### 消息类型
```rust
#[derive(Serialize, Deserialize)]
enum Message {
    // 连接管理
    Handshake { device_id: String, name: String, version: String, capabilities: Vec<String> },
    HandshakeAck { device_id: String, name: String, version: String, accepted: bool, reason: Option<String> },
    Disconnect { reason: String },
    Heartbeat { timestamp: u64 },
    HeartbeatAck { timestamp: u64, latency_ms: u32 },

    // 屏幕共享 (1-to-N 广播)
    ScreenOffer { displays: Vec<DisplayInfo> },
    ScreenRequest { display_id: u32, preferred_fps: u8, preferred_quality: u8 },
    ScreenStart { width: u32, height: u32, fps: u8, codec: String },
    ScreenFrame {
        timestamp: u64,
        frame_type: FrameType,  // I/P 帧
        sequence: u32,
        data: Vec<u8>,
    },
    ScreenStop,

    // 远程控制
    ControlRequest { from_user: String },
    ControlGrant { to_user: String },
    ControlRevoke,
    InputEvent {
        event_type: InputEventType,
        x: f32, y: f32,          // 相对坐标 0.0-1.0
        data: InputData,
    },

    // 聊天 (广播给所有成员)
    ChatMessage { from: String, content: String, timestamp: u64 },

    // 文件传输 (点对点)
    FileOffer { file_id: String, name: String, size: u64, checksum: String },
    FileAccept { file_id: String },
    FileReject { file_id: String },
    FileChunk { file_id: String, offset: u64, data: Vec<u8> },
    FileComplete { file_id: String },
    FileCancel { file_id: String },
}

#[derive(Serialize, Deserialize)]
enum FrameType {
    KeyFrame,    // I帧
    DeltaFrame,  // P帧
}

#[derive(Serialize, Deserialize)]
enum InputEventType {
    MouseMove,
    MouseDown,
    MouseUp,
    MouseScroll,
    KeyDown,
    KeyUp,
}
```

### 帧格式
```
┌──────────────┬──────────────┬──────────────┬──────────────┬──────────────────┐
│ Magic (2B)   │ Version (1B) │ Type (1B)    │ Length (4B)  │ Payload (N bytes)│
│ 0x4C 0x4D    │ 0x01         │ MessageType  │ big-endian   │ bincode encoded  │
└──────────────┴──────────────┴──────────────┴──────────────┴──────────────────┘
```

---

## 增量传输优化

### 脏矩形检测
```rust
struct DirtyRegion {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

/// 检测帧间差异区域
fn detect_dirty_regions(prev: &Frame, curr: &Frame, threshold: u8) -> Vec<DirtyRegion> {
    // 将画面分成 16x16 或 32x32 块
    // 对比每个块的差异，超过阈值则标记为脏区域
    // 合并相邻脏区域减少编码开销
}
```

### 自适应码率
```rust
struct AdaptiveBitrate {
    target_bitrate: u32,
    min_bitrate: u32,
    max_bitrate: u32,

    // 根据网络状况调整
    fn adjust(&mut self, rtt: Duration, packet_loss: f32) {
        if packet_loss > 0.05 {
            self.decrease();
        } else if rtt < Duration::from_millis(5) {
            self.increase();
        }
    }
}
```

---

## 开发计划与进度

### Phase 1: 项目初始化 ✅
- [x] 创建 Tauri + SolidJS + Bun 项目骨架
- [x] 配置开发环境和构建流程
- [x] 搭建基础 UI 框架
- [x] 设置跨平台条件编译

### Phase 2: 网络基础 ✅
- [x] 实现 mDNS 服务发现 (`network/discovery.rs`)
- [x] 实现 QUIC P2P 连接 (`network/quic.rs`)
- [x] 设计并实现通信协议 (`network/protocol.rs`)
- [x] 前后端联调 (DeviceList 组件)
- [ ] 会议室会话管理 (`network/session.rs`)

### Phase 3: 屏幕捕获 (平台原生) ✅
- [x] macOS: CoreGraphics CGDisplayCreateImage 实现
- [x] Windows: DXGI Desktop Duplication 实现
- [x] Linux: X11 实现 (PipeWire 需要 portal 集成)
- [x] 统一抽象层 (`capture/mod.rs`)
- [x] 权限检查和请求 (macOS)

### Phase 4: 视频编码 (硬件加速) ✅
- [x] FFmpeg 硬件编码器 (`encoder/ffmpeg/mod.rs`, via ffmpeg-next)
  - [x] macOS: VideoToolbox (h264_videotoolbox)
  - [x] Windows: NVENC (h264_nvenc)
  - [x] Windows: QuickSync (h264_qsv)
  - [x] Linux: VAAPI (h264_vaapi)
  - [x] Linux: NVENC (h264_nvenc)
  - [x] 软件回退: libx264
- [x] OpenH264 跨平台软件编码 (`encoder/software.rs`, 最终回退)
- [x] BGRA 到 YUV420 颜色空间转换
- [x] 编码器抽象接口 (`VideoEncoder` trait)
- [x] 自动编码器选择 (硬件优先，自动回退)

### Phase 5: 视频解码 + 渲染 ✅
- [x] OpenH264 跨平台软件解码 (`decoder/software.rs`)
- [x] Vulkan Video 硬件解码 (`decoder/vulkan/mod.rs`, via vk-video)
- [x] 解码器抽象接口 (`VideoDecoder` trait)
- [x] DecodedFrame 支持 CPU 和 GPU 数据路径
- [x] wgpu GPU 渲染器 (`renderer/wgpu_renderer.rs`)
- [x] BGRA 和 YUV420 GPU 着色器
- [x] 独立渲染窗口 (`renderer/window.rs`)
- [x] 自动解码器选择 (Vulkan Video → OpenH264)
- [ ] 零拷贝 GPU 纹理渲染 (待 vk-video 更新到 wgpu 28)

### Phase 6: 远程控制 ✅
- [x] 输入事件结构定义 (`input/events.rs`)
- [x] 跨平台输入模拟 (`input/controller.rs`, enigo)
- [x] macOS 辅助功能权限检查 (`input/macos.rs`)
- [x] USB HID 扫描码映射
- [ ] 控制权限管理 UI

### Phase 7: 聊天功能 ✅
- [x] 聊天消息结构 (`chat/mod.rs`)
- [x] 聊天管理器和历史记录
- [x] Tauri 命令 (`send_chat_message`, `get_chat_messages`)
- [x] 聊天 UI (`src/components/Chat/index.tsx`)
- [ ] 代码片段高亮

### Phase 8: 文件传输 ✅
- [x] 文件信息和校验和计算 (`transfer/mod.rs`)
- [x] 文件分块发送器 (`FileSender`)
- [x] 文件分块接收器 (`FileReceiver`)
- [x] 传输管理器 (`TransferManager`)
- [x] Tauri 命令 (offer/accept/reject/cancel/get transfers)
- [x] 协议消息处理 (FileOffer/Accept/Reject/Chunk/Complete/Cancel)
- [x] 断点续传支持 (missing_chunks)
- [x] 传输 UI (`src/components/FileTransfer/index.tsx`)

### Phase 9: 前端 UI 完善 ✅
- [x] 设备列表组件 (`DeviceList`)
- [x] 屏幕共享组件 (`ScreenShare`) - 已连接后端 API
- [x] 聊天组件 (`Chat`) - 消息列表和发送
- [x] 文件传输组件 (`FileTransfer`) - 发送/接收/进度
- [x] 全局状态管理 (`stores/app.ts`)
- [x] 四标签导航 (设备/共享/聊天/文件)
- [x] Tauri Dialog 插件集成

### Phase 10: 端到端通信优化 ✅
- [x] 修复编译警告 (unused imports/variables)
- [x] 实现聊天消息 QUIC 广播 (`broadcast_message`)
- [x] 实现文件传输 QUIC 发送 (`send_to_peer`)
- [x] Tauri 事件系统 (chat-message, file-offer, file-progress, file-complete)
- [x] 全局 APP_HANDLE 用于事件发送

### Phase 11: UI 重构 - 会议室模式 ✅
- [x] 重新设计 UI 为单一"会议室"概念
- [x] 服务开关 (默认关闭，用户手动开启)
- [x] 服务关闭时显示简洁的开关界面
- [x] 服务开启后显示会议室成员列表
- [x] 成员卡片显示：名称、IP、共享状态
- [x] 自己可以开始/停止共享
- [x] 他人共享时显示"正在共享"标识，可点击观看
- [x] 手动添加设备功能 (IP 输入)
- [x] 设置页面 (设备名称、画质、帧率)

**新增前端组件**:
- `src/App.tsx` - 主应用，服务开关逻辑
- `src/components/MeetingRoom/index.tsx` - 会议室组件
- `src/components/Settings/index.tsx` - 设置弹窗
- `src/components/AddDeviceModal/index.tsx` - 添加设备弹窗

**新增后端命令**:
- `start_service` - 启动 QUIC + mDNS 服务
- `stop_service` - 停止服务
- `is_service_running` - 检查服务状态
- `get_settings` / `save_settings` - 设置管理
- `broadcast_sharing_status` - 广播共享状态到所有对等端
- `request_screen_stream` - 请求屏幕流 (创建原生渲染窗口)
- `stop_viewing_stream` - 停止观看
- `request_control` - 请求远程控制 (待实现)

**通信改进**:
- 双向设备发现 (A 添加 B 后，B 的列表也显示 A)
- 共享状态同步 (ScreenOffer 消息 + sharing-status-changed 事件)
- 连接后自动监听消息

### Phase 12: 观看者窗口 ✅ (已重构为原生渲染)
- [x] ~~Vite 多页面配置~~ (已弃用，改为原生窗口)
- [x] ~~观看者 WebView 组件~~ (已弃用)
- [x] Rust 原生渲染窗口 (`RenderWindow`)
- [x] 跨线程窗口控制 (`RenderWindowHandle`)
- [x] wgpu GPU 渲染管线

### Phase 13: 视频流传输 ✅
- [x] 发送端: 捕获 → 编码 → 发送 ScreenFrame 消息
- [x] 接收端: 接收 ScreenFrame → 解码 → GPU渲染
- [x] 视频流请求协议 (ScreenRequest → ScreenStart)
- [x] 帧率控制 (基于配置的 FPS)
- [x] StreamingManager 管理发送端流
- [x] ViewerSession 管理接收端会话 (原生窗口渲染)
- [x] Rust 端解码渲染 (wgpu 独立窗口)
- [ ] 自适应码率 (根据网络状况调整)
- [ ] 关键帧请求机制

**新增模块**:
- `src-tauri/src/streaming/mod.rs` - 视频流管理模块
  - `StreamingManager` - 发送端流管理 (捕获→编码→发送)
  - `ViewerSession` - 接收端会话管理 (接收→解码→渲染)
  - `StreamingConfig` - 流配置 (FPS、画质、显示器)
  - `Quality` - 画质枚举 (Auto/High/Medium/Low)

**新增命令**:
- `request_screen_stream` - 请求对方的视频流 (创建原生渲染窗口)
- `stop_viewing_stream` - 停止观看视频流

**流程**:
1. A 开始共享 → 调用 `broadcast_sharing_status(true)`
2. A 端 StreamingManager 开始捕获、编码、发送 ScreenFrame
3. B 点击"观看" → 调用 `request_screen_stream`
4. `request_screen_stream` 创建 ViewerSession → 发送 ScreenRequest
5. A 端收到请求 → 发送 ScreenStart 响应
6. B 端收到 ScreenStart → 创建原生 wgpu 渲染窗口 → 初始化解码器
7. A 端持续发送 ScreenFrame → B 端 Rust 解码 → GPU 渲染到原生窗口
8. A 停止共享 → ScreenStop → B 端关闭渲染窗口

**Rust 端视频解码渲染** (高性能原生方案):
- `ViewerSession` - 管理解码器和渲染窗口
- `handle_screen_start()` - 创建 wgpu 原生窗口
- `handle_screen_frame()` - 解码 H.264 → 上传 GPU → 渲染
- `RenderWindow` - 独立 winit + wgpu 窗口
- `RenderWindowHandle` - 跨线程窗口控制
- 当前使用 OpenH264 软件解码 (硬件解码器待实现)
- 直接 GPU 纹理上传，无 IPC 开销
- 比 WebCodecs 方案更高效 (无 Base64 编码、无 Tauri 事件开销)

---

## 当前状态总结

### ✅ 已完成功能
| 功能 | 说明 |
|------|------|
| 设备发现 | mDNS 自动发现 + 手动 IP 添加 |
| P2P 连接 | QUIC 加密连接，自签名证书 |
| 屏幕捕获 | macOS/Windows/Linux 原生 API |
| 视频编码 | FFmpeg 硬件编码 (NVENC/VT/VAAPI/QSV) + OpenH264 回退 |
| 视频解码 | Vulkan Video 硬件解码 (Win/Linux) + OpenH264 回退 |
| 视频渲染 | wgpu 原生窗口 GPU 渲染 |
| 视频流 | 实时传输，帧率控制 |
| 聊天 | 文本消息广播 |
| 文件传输 | P2P 分块传输，断点续传 |
| UI | 会议室模式，成员列表，共享控制 |

### ⏳ 待实现功能
| 功能 | 优先级 | 说明 |
|------|--------|------|
| 零拷贝解码渲染 | 中 | vk-video 升级到 wgpu 28 后启用 |
| 远程控制 | 中 | 输入模拟已实现，权限管理待做 |
| 自适应码率 | 中 | 根据网络状况调整 |
| 关键帧请求 | 低 | 丢帧时请求 I 帧 |
| 代码高亮 | 低 | 聊天中的代码片段 |

### ❌ 已放弃/简化
| 原计划 | 实际 |
|--------|------|
| 主持人角色 | 对等模型，无主持人 |
| 演示者/观看者角色 | 任意成员可共享，任意成员可观看 |
| 会议室管理 | 简化为设备列表 |
| WebView 视频渲染 | 改为 Rust 原生 wgpu 窗口 |

### ⏸️ 暂缓功能
| 功能 | 原因 |
|------|------|
| macOS Vulkan 解码 | vk-video 不支持 macOS (Metal only) |
| 零拷贝 GPU 纹理 | vk-video 使用 wgpu 24, 我们用 wgpu 28 |

---

## 已实现模块说明

### network/discovery.rs
mDNS 服务发现模块，使用 `mdns-sd` crate。
- `start_discovery()` - 启动 mDNS 发现和服务注册
- `register_service()` - 注册本机服务
- `browse_services()` - 浏览局域网内其他服务
- `DiscoveredDevice` - 发现的设备信息结构

### network/quic.rs
QUIC P2P 传输模块，使用 `quinn` crate。
- `QuicEndpoint` - QUIC 端点，支持服务端和客户端
- `QuicConnection` - 活跃连接管理
- `QuicStream` - 双向流，支持 framed 消息
- `CONNECTIONS` - 全局连接注册表
- `broadcast_message()` - 向所有连接广播消息
- `send_to_peer()` - 向指定对等端发送消息
- `get_all_connections()` - 获取所有活跃连接
- 自签名证书生成 (LAN 使用)
- 支持 Datagram (用于视频帧)

### network/protocol.rs
二进制通信协议模块。
- `Message` - 所有消息类型枚举
- `MessageCodec` - 流式消息编解码器
- `encode()`/`decode()` - 消息序列化
- 协议头: Magic(LM) + Version + Type + Length

### commands/mod.rs
Tauri 命令接口。
- `get_devices` - 获取发现的设备列表
- `add_manual_device` - 手动添加设备
- `connect_to_device` - QUIC 连接 + 握手
- `disconnect` - 断开连接
- `get_self_info` - 获取本机信息
- `get_displays` - 获取可用显示器列表
- `start_capture` - 开始屏幕捕获
- `stop_capture` - 停止屏幕捕获
- `check_screen_permission` - 检查屏幕录制权限 (macOS)
- `request_screen_permission` - 请求屏幕录制权限 (macOS)
- `send_chat_message` - 发送聊天消息
- `get_chat_messages` - 获取聊天历史
- `check_input_permission` - 检查输入控制权限
- `request_input_permission` - 请求输入控制权限
- `offer_file` - 发起文件传输
- `accept_file_transfer` - 接受文件传输
- `reject_file_transfer` - 拒绝文件传输
- `cancel_file_transfer` - 取消文件传输
- `get_file_transfers` - 获取所有传输
- `get_active_file_transfers` - 获取活跃传输
- `get_file_transfer` - 获取指定传输
- `get_download_directory` - 获取下载目录

### capture/mod.rs
屏幕捕获抽象层。
- `ScreenCapture` trait - 统一捕获接口
- `Display` - 显示器信息结构
- `CapturedFrame` - 捕获的帧数据
- `create_capture()` - 创建平台特定的捕获实例

### capture/macos.rs
macOS 屏幕捕获，使用 CoreGraphics。
- `MacOSCapture` - 捕获实现
- `CGDisplayCreateImage` - 按需捕获帧
- `has_permission()` / `request_permission()` - 权限管理

### capture/windows.rs
Windows 屏幕捕获，使用 DXGI Desktop Duplication。
- `WindowsCapture` - 捕获实现
- D3D11 设备和上下文管理
- `IDXGIOutputDuplication` - 高效桌面复制

### capture/linux.rs
Linux 屏幕捕获，支持 X11 后端。
- `LinuxCapture` - 捕获实现
- X11 `XGetImage` 捕获
- PipeWire 后端 (需要 xdg-desktop-portal)

### encoder/mod.rs
视频编码抽象层。
- `VideoEncoder` trait - 统一编码接口
- `EncoderConfig` - 编码配置 (分辨率、码率、帧率等)
- `EncodedFrame` - 编码后的帧数据
- `create_encoder()` - 自动选择最佳编码器

### encoder/ffmpeg/mod.rs
FFmpeg 硬件加速编码器 (via ffmpeg-next)。
- `FfmpegEncoder` - 统一 FFmpeg 编码器
- `HwEncoderType` - 硬件编码器类型枚举
- 自动检测最佳可用编码器
- 支持: NVENC, VideoToolbox, VAAPI, QSV, libx264
- BGRA → YUV420 颜色转换
- 低延迟配置 (zerolatency, CBR)

### encoder/software.rs
OpenH264 软件编码器 (最终回退)。
- `SoftwareEncoder` - 跨平台 H.264 软编码
- BGRA → YUV420 颜色转换
- 支持强制关键帧

### decoder/mod.rs
视频解码抽象层。
- `VideoDecoder` trait - 统一解码接口
- `DecoderConfig` - 解码配置 (分辨率、输出格式等)
- `DecodedFrameData` - 支持 CPU 和 GPU 数据路径
- `DecodedFrame` - 解码后的帧数据
- `create_decoder()` - 自动选择最佳解码器

### decoder/vulkan/mod.rs
Vulkan Video 硬件解码器 (via vk-video)。
- `VulkanDecoder` - Vulkan Video H.264 解码
- 支持 Windows 和 Linux (NVIDIA/AMD)
- macOS 不支持 (返回 HardwareNotAvailable)
- NV12 → BGRA/YUV420 颜色转换
- 当前: CPU 输出路径
- 未来: 零拷贝 GPU 纹理 (待 vk-video 升级到 wgpu 28)

### decoder/software.rs
OpenH264 软件解码器 (回退)。
- `SoftwareDecoder` - 跨平台 H.264 软解码
- YUV420 → BGRA 颜色转换
- 支持 YUV420 直出 (用于 GPU 渲染)

### renderer/mod.rs
GPU 渲染抽象层。
- `RenderFrame` - 待渲染帧数据
- `FrameFormat` - 帧格式 (BGRA/YUV420)

### renderer/wgpu_renderer.rs
wgpu GPU 渲染器。
- `WgpuRenderer` - 跨平台 GPU 渲染
- BGRA 纹理上传和渲染
- YUV420 三平面纹理 + GPU 颜色转换
- 低延迟 Mailbox 呈现模式

### renderer/window.rs
独立渲染窗口。
- `RenderWindow` - 独立窗口管理
- `RenderWindowHandle` - 线程安全窗口控制
- 支持窗口事件回调 (键盘、鼠标等)
- winit 窗口 + wgpu 渲染

### input/mod.rs
输入控制抽象层。
- `InputEvent` - 可序列化的输入事件
- `has_permission()` / `request_permission()` - 权限管理

### input/events.rs
输入事件定义。
- `InputEvent` - 鼠标移动/点击/滚动、键盘按下/释放、文本输入
- `MouseButton` - 鼠标按钮类型
- `Modifiers` - 键盘修饰键 (Shift/Ctrl/Alt/Meta)
- `ControlState` - 控制权限状态

### input/controller.rs
跨平台输入控制器，使用 enigo。
- `InputController` - 输入模拟控制器
- `execute()` - 执行输入事件
- `scancode_to_key()` - USB HID 扫描码到按键映射

### input/macos.rs
macOS 辅助功能权限。
- `has_accessibility_permission()` - 检查 AXIsProcessTrusted
- `request_accessibility_permission()` - 触发系统授权弹窗

### chat/mod.rs
聊天消息管理。
- `ChatMessage` - 聊天消息结构 (支持文本/代码/系统消息)
- `ChatManager` - 消息历史管理 (最多 1000 条)
- `send_message()` / `receive_message()` - 发送/接收消息
- `get_chat_manager()` - 获取全局聊天管理器

### streaming/mod.rs
视频流传输模块，管理屏幕共享的捕获、编码、发送和接收。
- `StreamingManager` - 发送端管理器
  - `start_sync()` - 开始流 (同步启动，异步执行)
  - `stop_sync()` - 停止流
  - `is_streaming()` - 检查是否正在流
  - `frame_count()` - 获取已发送帧数
- `ViewerSession` - 接收端会话 (原生窗口渲染)
  - `handle_screen_start()` - 创建 wgpu 原生窗口，初始化解码器
  - `handle_screen_frame()` - 解码 H.264 帧，渲染到 GPU 窗口
  - `handle_screen_stop()` - 关闭渲染窗口
  - `close()` - 手动关闭会话
  - `is_window_open()` - 检查窗口是否仍打开
  - `frame_count()` - 获取已解码帧数
- `StreamingConfig` - 流配置 (fps, quality, display_id)
- `Quality` - 画质枚举 (Auto=8Mbps, High=8Mbps, Medium=4Mbps, Low=2Mbps)
- `request_screen_stream()` - 发送流请求
- `create_viewer_session()` / `remove_viewer_session()` - 会话管理

**发送流程**:
1. 用户开始共享 → `broadcast_sharing_status(true)`
2. 创建 StreamingManager → 初始化捕获和编码器
3. 启动后台任务:
   - 发送 ScreenStart 消息
   - 循环: 捕获帧 → 编码 → 发送 ScreenFrame
   - 帧率控制 (根据配置的 FPS)
4. 停止时发送 ScreenStop 消息

**接收流程** (Rust 原生渲染):
1. 用户点击"观看" → `request_screen_stream(peer_ip, peer_name)`
2. 创建 ViewerSession (解码器初始化) → 发送 ScreenRequest 消息
3. 接收 ScreenStart → 创建 wgpu 原生渲染窗口
4. 接收 ScreenFrame → Rust 解码 → GPU 纹理上传 → 渲染
5. 接收 ScreenStop 或用户关闭窗口 → 会话结束

### transfer/mod.rs
文件传输模块，支持 P2P 文件共享和断点续传。
- `FileInfo` - 文件信息结构 (ID、名称、大小、SHA-256 校验和、MIME 类型)
- `FileTransfer` - 传输状态记录 (进度、方向、状态)
- `TransferStatus` - 传输状态枚举 (Pending/Offered/InProgress/Completed/Failed/Cancelled)
- `TransferDirection` - 传输方向 (Outgoing/Incoming)
- `FileSender` - 文件发送器 (分块读取、校验和计算)
- `FileReceiver` - 文件接收器 (分块写入、完整性验证)
- `TransferManager` - 全局传输管理器 (并发传输、状态跟踪)
- `get_transfer_manager()` - 获取全局传输管理器

**文件传输流程**:
1. 发送方调用 `offer_file()` 创建传输并发送 `FileOffer` 消息
2. 接收方收到 `FileOffer`，通过 UI 展示给用户
3. 用户接受后调用 `accept_file_transfer()`，发送 `FileAccept` 消息
4. 发送方开始分块发送 (`FileChunk` 消息，每块 64KB)
5. 传输完成后发送 `FileComplete` 消息
6. 接收方验证校验和，确认文件完整性

**特性**:
- 64KB 分块传输，适合网络传输
- SHA-256 校验和验证文件完整性
- 支持断点续传 (`missing_chunks()` 获取缺失块)
- 默认下载到系统下载目录

### 前端组件 (src/components/)

**App.tsx** - 主应用入口
- 服务开关界面 (默认关闭)
- 服务开启后显示 MeetingRoom
- 设置弹窗管理
- 调用后端: `start_service`, `stop_service`, `is_service_running`, `get_self_info`

**MeetingRoom/index.tsx** - 会议室主界面
- 成员列表显示 (自己 + 其他设备)
- 开始/停止共享按钮
- 观看他人共享屏幕
- 请求远程控制
- 添加设备弹窗
- 事件监听: `device-discovered`, `device-removed`, `sharing-status-changed`
- 调用后端: `get_devices`, `broadcast_sharing_status`, `open_viewer_window`, `request_control`

**Settings/index.tsx** - 设置弹窗
- 设备名称设置
- 画质选择 (自动/高/中/低)
- 帧率选择 (15/30/60 FPS)
- 调用后端: `get_settings`, `save_settings`

**AddDeviceModal/index.tsx** - 手动添加设备
- IP 地址输入
- 连接验证
- 调用后端: `add_manual_device`

**Viewer/index.tsx** - 视频观看窗口 (已弃用，视频由 Rust 原生窗口渲染)
- 原本使用 WebCodecs API 解码渲染，现已改为 Rust 端原生渲染
- 视频现在通过 wgpu 独立窗口渲染，无需 WebView

**MeetingRoom/index.tsx** - 会议室主界面 (观看功能)
- 点击"观看"按钮 → 调用 `request_screen_stream`
- Rust 端创建原生 wgpu 窗口进行视频渲染
- 调用后端: `request_screen_stream` (创建原生渲染窗口)

**DeviceList/index.tsx** - 设备发现和连接 (旧版)
- 设备列表显示 (状态指示器: 在线/忙碌/离线)
- 手动 IP 连接
- 事件监听 (device-discovered/device-removed)
- 调用后端: `get_devices`, `add_manual_device`, `connect_to_device`

**ScreenShare/index.tsx** - 屏幕共享控制 (旧版)
- 显示器选择 (从后端获取)
- 权限检查和请求 (macOS)
- 开始/停止共享
- 设置 (帧率/画质/远程控制)
- 调用后端: `check_screen_permission`, `request_screen_permission`, `get_displays`, `start_capture`, `stop_capture`

**Chat/index.tsx** - 实时聊天 (旧版)
- 消息列表 (本地/远程消息区分)
- 文本输入和发送
- 自动滚动到最新消息
- 调用后端: `get_chat_messages`, `send_chat_message`

**FileTransfer/index.tsx** - 文件传输 (旧版)
- 文件选择 (Tauri Dialog 插件)
- 传输列表 (进行中/已完成)
- 进度条显示
- 接受/拒绝/取消操作
- 调用后端: `offer_file`, `accept_file_transfer`, `reject_file_transfer`, `cancel_file_transfer`, `get_file_transfers`, `get_download_directory`

### 前端状态管理 (src/stores/)

**app.ts** - 全局应用状态
- `selfInfo` - 本机设备信息
- `connectionState` - 连接状态 (已连接设备列表)
- `devices` - 发现的设备列表
- `isSharing` / `sharingDisplayId` - 屏幕共享状态

---

## 平台特定注意事项

### macOS
- 需要 `NSScreenCaptureUsageDescription` 权限描述
- 需要辅助功能权限（远程控制）
- ScreenCaptureKit 需要 macOS 12.3+
- Info.plist 配置:
  ```xml
  <key>NSScreenCaptureUsageDescription</key>
  <string>需要屏幕录制权限以共享屏幕</string>
  <key>NSAccessibilityUsageDescription</key>
  <string>需要辅助功能权限以进行远程控制</string>
  ```

### Windows
- DXGI 需要 Windows 8+
- 远程控制可能需要 UAC 提升
- 需要处理 DPI 缩放

### Linux
- PipeWire 需要较新的发行版
- X11 回退方案
- Wayland 权限门户

---

## 安全考虑

- 远程控制需要明确的用户授权弹窗
- 可选连接密码/PIN 码
- QUIC 自带 TLS 1.3 加密
- 会话密钥定期轮换
