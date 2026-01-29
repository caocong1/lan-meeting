import { Component, createSignal, onMount, onCleanup, Show } from "solid-js";
import { listen, UnlistenFn } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";

interface PeerInfo {
  id: string;
  name: string;
  ip: string;
}

interface ScreenStartEvent {
  peer_ip: string;
  width: number;
  height: number;
  fps: number;
  codec: string;
}

interface ScreenFrameEvent {
  peer_ip: string;
  timestamp: number;
  frame_type: string;
  sequence: number;
  data: string; // Base64 encoded H.264 NAL units
}

interface ScreenStopEvent {
  peer_ip: string;
}

export const Viewer: Component = () => {
  const [peerInfo, setPeerInfo] = createSignal<PeerInfo | null>(null);
  const [status, setStatus] = createSignal<"connecting" | "connected" | "streaming" | "disconnected">("connecting");
  const [error, setError] = createSignal<string | null>(null);
  const [frameInfo, setFrameInfo] = createSignal<{ width: number; height: number; fps: number } | null>(null);
  const [frameCount, setFrameCount] = createSignal(0);
  const [decodedFrames, setDecodedFrames] = createSignal(0);
  const [webCodecsSupported, setWebCodecsSupported] = createSignal(true);

  let canvasRef: HTMLCanvasElement | undefined;
  let ctxRef: CanvasRenderingContext2D | null = null;
  let videoDecoder: VideoDecoder | null = null;
  let unlistenStart: UnlistenFn | undefined;
  let unlistenFrame: UnlistenFn | undefined;
  let unlistenStop: UnlistenFn | undefined;
  let pendingFrames: VideoFrame[] = [];
  let isRendering = false;

  // Check WebCodecs support
  const checkWebCodecsSupport = () => {
    if (typeof VideoDecoder === "undefined") {
      setWebCodecsSupported(false);
      return false;
    }
    return true;
  };

  // Initialize video decoder
  const initDecoder = (width: number, height: number) => {
    if (!checkWebCodecsSupport()) {
      console.warn("WebCodecs not supported");
      return;
    }

    // Close existing decoder
    if (videoDecoder) {
      try {
        videoDecoder.close();
      } catch (e) {
        // Ignore
      }
    }

    videoDecoder = new VideoDecoder({
      output: (frame: VideoFrame) => {
        pendingFrames.push(frame);
        setDecodedFrames((c) => c + 1);
        renderFrame();
      },
      error: (e: Error) => {
        console.error("Decoder error:", e);
      },
    });

    // Configure decoder for H.264
    videoDecoder.configure({
      codec: "avc1.42E01E", // H.264 Baseline Profile Level 3.0
      codedWidth: width,
      codedHeight: height,
      hardwareAcceleration: "prefer-hardware",
    });

    console.log("Video decoder initialized:", width, "x", height);
  };

  // Render frame to canvas
  const renderFrame = () => {
    if (isRendering || pendingFrames.length === 0 || !canvasRef || !ctxRef) {
      return;
    }

    isRendering = true;

    // Get the latest frame, discard older ones
    while (pendingFrames.length > 1) {
      const oldFrame = pendingFrames.shift();
      oldFrame?.close();
    }

    const frame = pendingFrames.shift();
    if (frame) {
      // Draw frame to canvas
      ctxRef.drawImage(frame, 0, 0, canvasRef.width, canvasRef.height);
      frame.close();
    }

    isRendering = false;

    // Schedule next render if there are more frames
    if (pendingFrames.length > 0) {
      requestAnimationFrame(renderFrame);
    }
  };

  // Decode H.264 data
  const decodeFrame = (base64Data: string, frameType: string, timestamp: number) => {
    if (!videoDecoder || videoDecoder.state !== "configured") {
      return;
    }

    try {
      // Decode base64 to Uint8Array
      const binaryString = atob(base64Data);
      const bytes = new Uint8Array(binaryString.length);
      for (let i = 0; i < binaryString.length; i++) {
        bytes[i] = binaryString.charCodeAt(i);
      }

      // Create EncodedVideoChunk
      const chunk = new EncodedVideoChunk({
        type: frameType === "keyframe" ? "key" : "delta",
        timestamp: timestamp * 1000, // Convert to microseconds
        data: bytes,
      });

      videoDecoder.decode(chunk);
    } catch (e) {
      console.error("Failed to decode frame:", e);
    }
  };

  onMount(async () => {
    // Parse query parameters
    const params = new URLSearchParams(window.location.search);
    const peerId = params.get("peer_id");
    const peerName = params.get("peer_name");
    const peerIp = params.get("peer_ip");

    if (peerId && peerName && peerIp) {
      setPeerInfo({
        id: peerId,
        name: peerName,
        ip: peerIp,
      });

      // Set up event listeners
      await setupEventListeners(peerIp);

      // Request screen stream from peer
      await requestScreenStream(peerIp);
    } else {
      setError("Missing peer information");
      setStatus("disconnected");
    }
  });

  onCleanup(() => {
    unlistenStart?.();
    unlistenFrame?.();
    unlistenStop?.();

    // Close decoder
    if (videoDecoder) {
      try {
        videoDecoder.close();
      } catch (e) {
        // Ignore
      }
    }

    // Close pending frames
    pendingFrames.forEach((f) => f.close());
    pendingFrames = [];

    // Stop viewing stream
    const info = peerInfo();
    if (info) {
      invoke("stop_viewing_stream", { peerIp: info.ip }).catch(console.error);
    }
  });

  const setupEventListeners = async (peerIp: string) => {
    // Listen for screen start
    unlistenStart = await listen<ScreenStartEvent>("screen-start", (event) => {
      if (event.payload.peer_ip === peerIp) {
        console.log("Screen start received:", event.payload);
        setFrameInfo({
          width: event.payload.width,
          height: event.payload.height,
          fps: event.payload.fps,
        });
        setStatus("streaming");

        // Initialize canvas
        if (canvasRef) {
          canvasRef.width = event.payload.width;
          canvasRef.height = event.payload.height;
          ctxRef = canvasRef.getContext("2d");
        }

        // Initialize decoder
        initDecoder(event.payload.width, event.payload.height);
      }
    });

    // Listen for screen frames
    unlistenFrame = await listen<ScreenFrameEvent>("screen-frame", (event) => {
      if (event.payload.peer_ip === peerIp) {
        setFrameCount((c) => c + 1);

        // Decode frame if WebCodecs is supported
        if (webCodecsSupported()) {
          decodeFrame(
            event.payload.data,
            event.payload.frame_type,
            event.payload.timestamp
          );
        }
      }
    });

    // Listen for screen stop
    unlistenStop = await listen<ScreenStopEvent>("screen-stop", (event) => {
      if (event.payload.peer_ip === peerIp) {
        console.log("Screen stop received");
        setStatus("disconnected");
      }
    });
  };

  const requestScreenStream = async (peerIp: string) => {
    try {
      setStatus("connecting");
      console.log(`Requesting screen stream from ${peerIp}`);

      await invoke("request_screen_stream", { peerIp });

      setStatus("connected");
    } catch (err) {
      console.error("Failed to request screen stream:", err);
      setError(String(err));
      setStatus("disconnected");
    }
  };

  const handleFullscreen = () => {
    if (document.fullscreenElement) {
      document.exitFullscreen();
    } else {
      document.documentElement.requestFullscreen();
    }
  };

  const handleRequestControl = async () => {
    const info = peerInfo();
    if (info) {
      try {
        await invoke("request_control", { peerId: info.id });
      } catch (err) {
        console.error("Failed to request control:", err);
      }
    }
  };

  return (
    <div class="h-screen w-screen bg-black flex flex-col" onDblClick={handleFullscreen}>
      {/* Header bar */}
      <div class="h-10 bg-gray-900 flex items-center justify-between px-4 shrink-0">
        <div class="flex items-center gap-2 text-white">
          <div class="i-lucide-monitor w-4 h-4" />
          <span class="text-sm font-medium">
            {peerInfo()?.name || "Unknown"} 的屏幕
          </span>
          <span class="text-xs text-gray-400">
            ({peerInfo()?.ip})
          </span>
        </div>

        <div class="flex items-center gap-4">
          {/* Stream info */}
          <Show when={frameInfo()}>
            <span class="text-xs text-gray-400">
              {frameInfo()!.width}x{frameInfo()!.height} @ {frameInfo()!.fps}fps
            </span>
          </Show>

          {/* Frame counter */}
          <Show when={status() === "streaming"}>
            <span class="text-xs text-gray-400">
              接收: {frameCount()} | 解码: {decodedFrames()}
            </span>
          </Show>

          {/* Connection status */}
          <div class="flex items-center gap-2">
            <div class={`w-2 h-2 rounded-full ${
              status() === "streaming" ? "bg-green-500" :
              status() === "connected" ? "bg-blue-500" :
              status() === "connecting" ? "bg-yellow-500 animate-pulse" :
              "bg-red-500"
            }`} />
            <span class="text-xs text-gray-400">
              {status() === "streaming" ? "正在播放" :
               status() === "connected" ? "已连接" :
               status() === "connecting" ? "连接中..." :
               "已断开"}
            </span>
          </div>

          {/* Control buttons */}
          <div class="flex items-center gap-2">
            <button
              class="p-1.5 text-gray-400 hover:text-white hover:bg-gray-700 rounded transition-colors"
              title="请求控制"
              onClick={handleRequestControl}
            >
              <div class="i-lucide-mouse-pointer w-4 h-4" />
            </button>
            <button
              class="p-1.5 text-gray-400 hover:text-white hover:bg-gray-700 rounded transition-colors"
              title="全屏"
              onClick={handleFullscreen}
            >
              <div class="i-lucide-maximize w-4 h-4" />
            </button>
          </div>
        </div>
      </div>

      {/* Main video area */}
      <div class="flex-1 flex items-center justify-center relative overflow-hidden bg-black">
        <Show when={error()}>
          <div class="text-center text-white">
            <div class="i-lucide-alert-circle w-16 h-16 mx-auto mb-4 text-red-500" />
            <p class="text-lg mb-2">连接失败</p>
            <p class="text-sm text-gray-400">{error()}</p>
          </div>
        </Show>

        <Show when={!error() && status() === "connecting"}>
          <div class="text-center text-white">
            <div class="i-lucide-loader-2 w-16 h-16 mx-auto mb-4 animate-spin text-primary-500" />
            <p class="text-lg mb-2">正在连接到 {peerInfo()?.name}...</p>
            <p class="text-sm text-gray-400">{peerInfo()?.ip}</p>
          </div>
        </Show>

        <Show when={!error() && status() === "connected"}>
          <div class="text-center text-white">
            <div class="i-lucide-loader-2 w-16 h-16 mx-auto mb-4 animate-spin text-blue-500" />
            <p class="text-lg mb-2">等待视频流...</p>
            <p class="text-sm text-gray-400">已连接，等待对方开始共享</p>
          </div>
        </Show>

        <Show when={!error() && status() === "streaming"}>
          {/* Video canvas - always visible when streaming */}
          <canvas
            ref={canvasRef}
            class="max-w-full max-h-full object-contain"
            style={{
              "image-rendering": "auto",
              display: decodedFrames() > 0 ? "block" : "none"
            }}
          />

          {/* Show loading if no frames decoded yet */}
          <Show when={decodedFrames() === 0}>
            <div class="absolute inset-0 flex items-center justify-center">
              <div class="text-center text-white">
                <div class="i-lucide-loader-2 w-12 h-12 mx-auto mb-4 animate-spin text-green-500" />
                <p class="text-lg mb-2">正在接收视频流...</p>
                <p class="text-sm text-gray-400">
                  已接收 {frameCount()} 帧，等待解码...
                </p>
                <Show when={!webCodecsSupported()}>
                  <p class="text-xs text-yellow-500 mt-2">
                    警告: 浏览器不支持 WebCodecs API
                  </p>
                </Show>
              </div>
            </div>
          </Show>
        </Show>

        <Show when={!error() && status() === "disconnected"}>
          <div class="text-center text-white">
            <div class="i-lucide-monitor-off w-16 h-16 mx-auto mb-4 text-gray-600" />
            <p class="text-lg mb-2 text-gray-400">连接已断开</p>
            <p class="text-sm text-gray-500">对方已停止共享</p>
          </div>
        </Show>
      </div>

      {/* Footer bar */}
      <div class="h-6 bg-gray-900 flex items-center justify-center text-xs text-gray-500">
        <span>双击进入/退出全屏</span>
      </div>
    </div>
  );
};
