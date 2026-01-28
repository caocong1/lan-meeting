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

### 会议模型

```
┌─────────────────────────────────────────────────────────────┐
│                        会议室 (Meeting Room)                  │
│                                                              │
│  ┌──────────────┐                                           │
│  │   主持人      │ ◄─── 创建会议、管理成员、主控制权          │
│  │  (Host)      │                                           │
│  └──────┬───────┘                                           │
│         │                                                    │
│         │ 屏幕流广播 (1-to-N)                                │
│         ▼                                                    │
│  ┌──────────────────────────────────────┐                   │
│  │           参会者 (Participants)        │                   │
│  │  ┌─────┐  ┌─────┐  ┌─────┐  ┌─────┐  │                   │
│  │  │ P1  │  │ P2  │  │ P3  │  │ P4  │  │ ◄── 最多4人观看   │
│  │  └─────┘  └─────┘  └─────┘  └─────┘  │                   │
│  └──────────────────────────────────────┘                   │
│                                                              │
│  功能：                                                      │
│  - 任意成员可申请成为演示者                                    │
│  - 演示者共享屏幕，其他人观看                                  │
│  - 主持人可授权远程控制                                        │
│  - 全员可发送聊天消息                                          │
│  - 全员可传输文件                                              │
└─────────────────────────────────────────────────────────────┘
```

### 连接拓扑

```
    全网状 P2P (Mesh Topology)

         ┌─────┐
         │  A  │
         └──┬──┘
           /│\
          / │ \
         /  │  \
    ┌───┐   │   ┌───┐
    │ B │───┼───│ C │
    └───┘   │   └───┘
         \  │  /
          \ │ /
           \│/
         ┌─────┐
         │  D  │
         └─────┘

每个节点与其他所有节点直连
- 控制消息: 全网状广播
- 屏幕流: 演示者 → 所有观看者 (1-to-N)
```

### 角色定义

| 角色 | 权限 |
|------|------|
| **主持人 (Host)** | 创建会议、踢出成员、授权控制权、结束会议 |
| **演示者 (Presenter)** | 共享屏幕、允许/拒绝控制请求 |
| **参会者 (Participant)** | 观看屏幕、请求控制、聊天、传文件 |
| **控制者 (Controller)** | 被授权后可远程控制演示者屏幕 |

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

## 极致性能架构

```
┌─────────────────────────────────────────────────────────────────────┐
│                           Tauri App                                 │
├─────────────────────┬───────────────────────────────────────────────┤
│  Frontend (WebView) │              Backend (Rust)                   │
│  ─────────────────  │  ───────────────────────────────────────────  │
│  SolidJS + Bun      │                                               │
│  ├── 控制面板 UI     │  ┌─────────────────────────────────────────┐  │
│  ├── 状态监控        │  │  capture/ (平台原生)                     │  │
│  └── 设置界面        │  │  ├── macos.rs   → ScreenCaptureKit      │  │
│                     │  │  ├── windows.rs → DXGI Desktop Dup      │  │
│  ┌───────────────┐  │  │  └── linux.rs   → PipeWire/DMA-BUF      │  │
│  │ wgpu 渲染窗口  │←────│  └─────────────────────────────────────────┘  │
│  │ (独立/GPU直渲) │  │                                               │
│  └───────────────┘  │  ┌─────────────────────────────────────────┐  │
│                     │  │  encoder/ (硬件优先)                     │  │
│                     │  │  ├── videotoolbox.rs → macOS H264/HEVC  │  │
│                     │  │  ├── nvenc.rs        → NVIDIA GPU       │  │
│                     │  │  ├── amf.rs          → AMD GPU          │  │
│                     │  │  ├── qsv.rs          → Intel QSV        │  │
│                     │  │  ├── vaapi.rs        → Linux 通用       │  │
│                     │  │  └── software.rs     → x264 回退        │  │
│                     │  └─────────────────────────────────────────┘  │
│                     │                                               │
│                     │  ┌─────────────────────────────────────────┐  │
│                     │  │  network/                                │  │
│                     │  │  ├── quic.rs      → QUIC (quinn)        │  │
│                     │  │  ├── session.rs  → 会议室管理            │  │
│                     │  │  ├── priority.rs  → 帧优先级调度         │  │
│                     │  │  └── discovery.rs → mDNS                │  │
│                     │  └─────────────────────────────────────────┘  │
│                     │                                               │
│                     │  ┌─────────────────────────────────────────┐  │
│                     │  │  decoder/ (硬件优先)                     │  │
│                     │  │  └── 对应各平台硬件解码                   │  │
│                     │  └─────────────────────────────────────────┘  │
│                     │                                               │
│                     │  ┌─────────────────────────────────────────┐  │
│                     │  │  renderer/                               │  │
│                     │  │  └── wgpu.rs → GPU 直接渲染              │  │
│                     │  └─────────────────────────────────────────┘  │
└─────────────────────┴───────────────────────────────────────────────┘
```

---

## 延迟优化链路

```
捕获        编码         传输        解码         渲染
─────────────────────────────────────────────────────────
平台原生  →  硬件编码  →  QUIC/UDP  →  硬件解码  →  wgpu GPU
  ↓           ↓           ↓            ↓            ↓
 ~3ms       ~3ms        ~1ms         ~3ms         ~2ms
─────────────────────────────────────────────────────────
                    总延迟: ~12-15ms (理想)
                           ~20-30ms (实际)
```

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

### 视频编码 (硬件优先)

| 平台 | 硬件编码器 | Rust 绑定 | 回退方案 |
|------|-----------|-----------|----------|
| macOS | VideoToolbox | `videotoolbox-rs` | - |
| Windows/Linux (NVIDIA) | NVENC | `nvenc-rs` | x264 |
| Windows/Linux (AMD) | AMF | FFI 封装 | x264 |
| Windows/Linux (Intel) | QSV | FFI 封装 | x264 |
| Linux | VAAPI | `va-rs` | x264 |

**编码配置**:
```rust
struct EncoderConfig {
    codec: Codec::H264,           // H264 兼容性最好
    preset: Preset::UltraFast,    // 最低延迟
    tune: Tune::ZeroLatency,      // 零延迟调优
    profile: Profile::Baseline,   // 最简配置
    bitrate: 8_000_000,           // 8 Mbps
    max_bitrate: 15_000_000,      // 峰值 15 Mbps
    fps: 60,
    gop_size: 60,                 // 1秒一个关键帧
    b_frames: 0,                  // 禁用 B 帧
    ref_frames: 1,                // 单参考帧
    rc_mode: RateControl::CBR,    // 恒定比特率
}
```

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

### 渲染

| 组件 | 技术 | 说明 |
|------|------|------|
| GPU 渲染 | wgpu | 跨平台 GPU API |
| 窗口管理 | winit | 独立渲染窗口 |

**渲染优化**:
- 独立窗口渲染，绕过 WebView 延迟
- 直接将解码帧上传 GPU 纹理
- 支持硬件解码输出直接渲染 (零拷贝)

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

## 通信协议设计

### 会议室消息

```rust
/// 会议室相关消息
enum MeetingMessage {
    // 会议管理
    CreateMeeting { meeting_id: String, host_id: String, name: String },
    JoinMeeting { meeting_id: String, device_id: String, name: String },
    LeaveMeeting { meeting_id: String, device_id: String },
    MeetingInfo {
        meeting_id: String,
        host_id: String,
        participants: Vec<Participant>,
        presenter_id: Option<String>,
    },

    // 演示者控制
    RequestPresent { device_id: String },
    GrantPresent { device_id: String },
    RevokePresent,

    // 成员管理
    KickMember { device_id: String, reason: String },
    MemberJoined { participant: Participant },
    MemberLeft { device_id: String },
}

struct Participant {
    device_id: String,
    name: String,
    role: ParticipantRole,
    joined_at: u64,
}

enum ParticipantRole {
    Host,       // 主持人
    Presenter,  // 演示者
    Viewer,     // 观看者
    Controller, // 被授权控制者
}
```

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

### Phase 4: 视频编码 (软件优先) ✅
- [x] OpenH264 跨平台软件编码 (`encoder/software.rs`)
- [x] BGRA 到 YUV420 颜色空间转换
- [x] 编码器抽象接口 (`VideoEncoder` trait)
- [ ] macOS: VideoToolbox H264 编码 (硬件)
- [ ] NVIDIA: NVENC 编码 (硬件)
- [ ] AMD/Linux: VAAPI 编码 (硬件)
- [x] 自动编码器选择 (fallback 到软编码)

### Phase 5: 视频解码 + 渲染 ✅
- [x] OpenH264 跨平台软件解码 (`decoder/software.rs`)
- [x] 解码器抽象接口 (`VideoDecoder` trait)
- [x] wgpu GPU 渲染器 (`renderer/wgpu_renderer.rs`)
- [x] BGRA 和 YUV420 GPU 着色器
- [x] 独立渲染窗口 (`renderer/window.rs`)
- [ ] 各平台硬件解码实现 (VideoToolbox/DXVA/VAAPI)
- [ ] 解码-渲染零拷贝优化

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

### encoder/software.rs
OpenH264 软件编码器。
- `SoftwareEncoder` - 跨平台 H.264 软编码
- BGRA → YUV420 颜色转换
- 支持强制关键帧

### decoder/mod.rs
视频解码抽象层。
- `VideoDecoder` trait - 统一解码接口
- `DecoderConfig` - 解码配置 (分辨率、输出格式等)
- `DecodedFrame` - 解码后的帧数据 (BGRA 或 YUV420)
- `create_decoder()` - 自动选择最佳解码器

### decoder/software.rs
OpenH264 软件解码器。
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

**DeviceList/index.tsx** - 设备发现和连接
- 设备列表显示 (状态指示器: 在线/忙碌/离线)
- 手动 IP 连接
- 事件监听 (device-discovered/device-removed)
- 调用后端: `get_devices`, `add_manual_device`, `connect_to_device`

**ScreenShare/index.tsx** - 屏幕共享控制
- 显示器选择 (从后端获取)
- 权限检查和请求 (macOS)
- 开始/停止共享
- 设置 (帧率/画质/远程控制)
- 调用后端: `check_screen_permission`, `request_screen_permission`, `get_displays`, `start_capture`, `stop_capture`

**Chat/index.tsx** - 实时聊天
- 消息列表 (本地/远程消息区分)
- 文本输入和发送
- 自动滚动到最新消息
- 调用后端: `get_chat_messages`, `send_chat_message`

**FileTransfer/index.tsx** - 文件传输
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
