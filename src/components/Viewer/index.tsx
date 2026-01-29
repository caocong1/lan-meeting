import { Component, createSignal, onMount, onCleanup } from "solid-js";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";

interface PeerInfo {
  id: string;
  name: string;
  ip: string;
}

export const Viewer: Component = () => {
  const [peerInfo, setPeerInfo] = createSignal<PeerInfo | null>(null);
  const [status, setStatus] = createSignal<"connecting" | "connected" | "disconnected">("connecting");
  const [error, setError] = createSignal<string | null>(null);

  onMount(() => {
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

      // Request screen stream from peer
      requestScreenStream(peerId, peerIp);
    } else {
      setError("Missing peer information");
      setStatus("disconnected");
    }
  });

  const requestScreenStream = async (peerId: string, peerIp: string) => {
    try {
      setStatus("connecting");
      // For now, just show connecting state
      // TODO: Implement actual screen streaming request
      console.log(`Requesting screen stream from ${peerId} (${peerIp})`);

      // Simulate connection for now
      setTimeout(() => {
        setStatus("connected");
      }, 1000);
    } catch (err) {
      console.error("Failed to request screen stream:", err);
      setError(String(err));
      setStatus("disconnected");
    }
  };

  return (
    <div class="h-screen w-screen bg-black flex flex-col">
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
          {/* Connection status */}
          <div class="flex items-center gap-2">
            <div class={`w-2 h-2 rounded-full ${
              status() === "connected" ? "bg-green-500" :
              status() === "connecting" ? "bg-yellow-500 animate-pulse" :
              "bg-red-500"
            }`} />
            <span class="text-xs text-gray-400">
              {status() === "connected" ? "已连接" :
               status() === "connecting" ? "连接中..." :
               "已断开"}
            </span>
          </div>

          {/* Control buttons */}
          <div class="flex items-center gap-2">
            <button
              class="p-1.5 text-gray-400 hover:text-white hover:bg-gray-700 rounded transition-colors"
              title="请求控制"
            >
              <div class="i-lucide-mouse-pointer w-4 h-4" />
            </button>
            <button
              class="p-1.5 text-gray-400 hover:text-white hover:bg-gray-700 rounded transition-colors"
              title="全屏"
            >
              <div class="i-lucide-maximize w-4 h-4" />
            </button>
          </div>
        </div>
      </div>

      {/* Main video area */}
      <div class="flex-1 flex items-center justify-center relative">
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
        ) : (
          <div class="text-center text-white">
            <div class="i-lucide-monitor w-24 h-24 mx-auto mb-4 text-gray-600" />
            <p class="text-lg mb-2 text-gray-400">等待视频流...</p>
            <p class="text-sm text-gray-500">视频流功能开发中</p>
          </div>
        )}

        {/* Canvas for video rendering (hidden for now) */}
        <canvas
          id="video-canvas"
          class="hidden absolute inset-0 w-full h-full object-contain"
        />
      </div>

      {/* Footer bar (optional info) */}
      <div class="h-6 bg-gray-900 flex items-center justify-center text-xs text-gray-500">
        <span>按 ESC 退出全屏 | 双击进入/退出全屏</span>
      </div>
    </div>
  );
};
