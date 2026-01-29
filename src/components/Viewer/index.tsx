import { Component, createSignal, onMount, onCleanup } from "solid-js";
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
  data: string; // Base64 encoded
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

  let canvasRef: HTMLCanvasElement | undefined;
  let unlistenStart: UnlistenFn | undefined;
  let unlistenFrame: UnlistenFn | undefined;
  let unlistenStop: UnlistenFn | undefined;

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
        }
      }
    });

    // Listen for screen frames
    unlistenFrame = await listen<ScreenFrameEvent>("screen-frame", (event) => {
      if (event.payload.peer_ip === peerIp) {
        setFrameCount((c) => c + 1);
        // For now, we just count frames
        // Full video decoding would require WebCodecs or a WASM decoder
        // The backend is sending H.264 NAL units which browsers can't decode directly
        // We'll implement a simpler approach: send decoded BGRA frames via events
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
          {frameInfo() && (
            <span class="text-xs text-gray-400">
              {frameInfo()!.width}x{frameInfo()!.height} @ {frameInfo()!.fps}fps
            </span>
          )}

          {/* Frame counter */}
          {status() === "streaming" && (
            <span class="text-xs text-gray-400">
              帧: {frameCount()}
            </span>
          )}

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
      <div class="flex-1 flex items-center justify-center relative overflow-hidden">
        {error() ? (
          <div class="text-center text-white">
            <div class="i-lucide-alert-circle w-16 h-16 mx-auto mb-4 text-red-500" />
            <p class="text-lg mb-2">连接失败</p>
            <p class="text-sm text-gray-400">{error()}</p>
          </div>
        ) : status() === "connecting" ? (
          <div class="text-center text-white">
            <div class="i-lucide-loader-2 w-16 h-16 mx-auto mb-4 animate-spin text-primary-500" />
            <p class="text-lg mb-2">正在连接到 {peerInfo()?.name}...</p>
            <p class="text-sm text-gray-400">{peerInfo()?.ip}</p>
          </div>
        ) : status() === "connected" ? (
          <div class="text-center text-white">
            <div class="i-lucide-loader-2 w-16 h-16 mx-auto mb-4 animate-spin text-blue-500" />
            <p class="text-lg mb-2">等待视频流...</p>
            <p class="text-sm text-gray-400">已连接，等待对方开始共享</p>
          </div>
        ) : status() === "streaming" ? (
          <>
            {/* Video canvas */}
            <canvas
              ref={canvasRef}
              class="max-w-full max-h-full object-contain"
              style={{ "image-rendering": "pixelated" }}
            />
            {/* Placeholder message while we implement actual rendering */}
            <div class="absolute inset-0 flex items-center justify-center">
              <div class="text-center text-white bg-black/70 p-6 rounded-lg">
                <div class="i-lucide-video w-12 h-12 mx-auto mb-4 text-green-500" />
                <p class="text-lg mb-2">正在接收视频流</p>
                <p class="text-sm text-gray-400">
                  {frameInfo()?.width}x{frameInfo()?.height} @ {frameInfo()?.fps}fps
                </p>
                <p class="text-xs text-gray-500 mt-2">
                  已接收 {frameCount()} 帧
                </p>
                <p class="text-xs text-yellow-500 mt-4">
                  注意: 视频渲染功能开发中
                </p>
              </div>
            </div>
          </>
        ) : (
          <div class="text-center text-white">
            <div class="i-lucide-monitor-off w-16 h-16 mx-auto mb-4 text-gray-600" />
            <p class="text-lg mb-2 text-gray-400">连接已断开</p>
            <p class="text-sm text-gray-500">对方已停止共享</p>
          </div>
        )}
      </div>

      {/* Footer bar */}
      <div class="h-6 bg-gray-900 flex items-center justify-center text-xs text-gray-500">
        <span>双击进入/退出全屏</span>
      </div>
    </div>
  );
};
