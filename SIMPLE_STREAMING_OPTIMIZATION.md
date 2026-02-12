# Simple Streaming 优化计划

**当前状态**: 3456x2160 原始分辨率, 10 FPS, 2 Mbps, 全软件编解码, ~8秒延迟

---

## Step 1: 降低编码分辨率 (最大收益, 最简单)

**问题**: 当前以 3456x2160 全分辨率软件编码, 每帧 BGRA→YUV 转换处理 ~750万像素 + OpenH264 编码, 非常慢

**方案**: 修改 `FrameScaler` 增加真正的缩放(不只是裁剪), 将编码分辨率降到 1280x720

**改动文件**:
- `src-tauri/src/encoder/scaler.rs` - 增加 downscale 模式
- `src-tauri/src/simple_streaming/mod.rs` - 使用 target 分辨率初始化 scaler

**预期日志**:
```
[SIMPLE] Display: xxx (3456x2160)
[SIMPLE] Encoder initialized: 3456x2160 -> 1280x720 (scaled) @ 10 fps
[SIMPLE] Frame 0 encoded: ~5000-15000 bytes  (之前是 ~50000-200000 bytes)
```

**预期效果**: 编码耗时降低 ~6倍, 帧体积大幅缩小, 延迟从 ~8秒降到 ~2-3秒

**状态**: [x] 已完成 (延迟从 ~8秒降到 ~2-3秒, 视频比例已修复)

---

## Step 2: 接收端丢弃过期帧

**问题**: 渲染通道 `unbounded()`, 解码慢时帧在队列中堆积, 延迟越来越大

**方案**:
- 接收端 `recv_framed` 后检查是否有更新的帧, 有则跳过当前帧只解码最新的
- 渲染窗口命令通道: 发送新帧前先清空旧帧

**改动文件**:
- `src-tauri/src/simple_streaming/mod.rs` - 接收端帧跳过逻辑
- `src-tauri/src/renderer/window.rs` - 渲染通道丢帧

**预期日志**:
```
[SIMPLE] Skipped N stale frames, processing latest
Render thread: dropped N stale frames, rendering latest
```

**预期效果**: 延迟不再随时间累积, 保持稳定在 1-2秒

**状态**: [x] 已完成

---

## Step 3: 解码直出 YUV420, GPU 做色彩转换

**问题**: 解码后做像素级 YUV420→BGRA 转换(~750万次运算), 再上传 BGRA texture

**方案**: 解码输出 YUV420 → 直接上传 Y/U/V 三个纹理 → 用已有的 YUV shader 在 GPU 转换

**改动文件**:
- `src-tauri/src/simple_streaming/mod.rs` - 改 `OutputFormat::YUV420`, 构造 YUV RenderFrame
- `src-tauri/src/renderer/window.rs` - 渲染循环使用 YUV format

**预期日志**:
```
[SIMPLE] Decoder initialized (OpenH264 software, output=YUV420)
Render thread: frame N received and uploaded (1280x720, YUV420)
```

**预期效果**: 接收端 CPU 占用显著降低, 解码+渲染更快

**状态**: [ ] 未开始

---

## Step 4: 提升帧率到 30 FPS

**问题**: 10 FPS 视觉上很不流畅

**方案**: Step 1-3 优化后每帧处理已足够快, 将 `SIMPLE_FPS` 从 10 改为 30

**改动文件**:
- `src-tauri/src/simple_streaming/mod.rs` - 改常量

**预期日志**:
```
[SIMPLE] Starting frame streaming loop at 30 fps
[SIMPLE] Frame 50 encoded: ...  (每 1.67 秒打印一次)
```

**预期效果**: 明显更流畅, 延迟保持在 1-2秒内

**状态**: [ ] 未开始

---

## Step 5: 优化编码端 BGRA→YUV 转换

**问题**: `bgra_to_yuv420` 是逐像素循环, 对 1280x720 仍有一定开销

**方案**: 按行批量处理, 优化缓存命中率; 或用 rayon 并行转换多行

**改动文件**:
- `src-tauri/src/encoder/software.rs` - 优化转换函数

**预期日志**:
```
[SIMPLE] Frame N encode time: ~Xms  (添加计时日志)
```

**预期效果**: 编码端 CPU 占用降低 30-50%

**状态**: [ ] 未开始

---

## Step 6: 切换硬件编码器 (FFmpeg VideoToolbox/NVENC)

**问题**: 软件编码有上限, 即使优化也受限于 CPU

**方案**: 用已有的 `FfmpegEncoder` (Windows 走 NVENC, macOS 走 VideoToolbox)

**改动文件**:
- `src-tauri/src/simple_streaming/mod.rs` - 编码器创建改用 `encoder::create_encoder()`

**预期日志**:
```
Using FFmpeg encoder: h264_nvenc (或 h264_videotoolbox)
[SIMPLE] Frame 0 encoded: ~3000-8000 bytes, encode time: <1ms
```

**预期效果**: 编码速度提升 10x+, 支持更高分辨率和帧率, 可提升到 1920x1080@60fps

**状态**: [ ] 未开始

---

## Step 7: 调优 QUIC 传输参数

**问题**: QUIC 默认拥塞控制和缓冲可能引入额外延迟

**方案**: 减小发送/接收缓冲区, 配置拥塞控制偏好低延迟

**改动文件**:
- `src-tauri/src/network/quic.rs` - 传输配置参数

**预期效果**: 传输延迟降低 50-100ms

**状态**: [ ] 未开始
