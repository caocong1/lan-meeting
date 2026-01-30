import { Component, createSignal, For, Show, onMount } from "solid-js";
import { invoke } from "@tauri-apps/api/core";

interface DisplayInfo {
  id: number;
  name: string;
  width: number;
  height: number;
  scale_factor: number;
  primary: boolean;
}

export const ScreenShare: Component = () => {
  const [isSharing, setIsSharing] = createSignal(false);
  const [selectedDisplay, setSelectedDisplay] = createSignal<number | null>(null);
  const [displays, setDisplays] = createSignal<DisplayInfo[]>([]);
  const [isLoading, setIsLoading] = createSignal(true);
  const [error, setError] = createSignal<string | null>(null);
  const [hasPermission, setHasPermission] = createSignal(true);
  const [fps, setFps] = createSignal(60);
  const [quality, setQuality] = createSignal("auto");
  const [allowRemoteControl, setAllowRemoteControl] = createSignal(true);

  // Check and fetch displays
  const fetchDisplays = async () => {
    try {
      setIsLoading(true);
      setError(null);

      // Check screen permission first (macOS)
      const permitted = await invoke<boolean>("check_screen_permission");
      setHasPermission(permitted);

      if (!permitted) {
        setError("需要屏幕录制权限才能共享屏幕");
        return;
      }

      const result = await invoke<DisplayInfo[]>("get_displays");
      setDisplays(result);

      // Auto-select primary display
      const primary = result.find((d) => d.primary);
      if (primary && selectedDisplay() === null) {
        setSelectedDisplay(primary.id);
      }
    } catch (e) {
      console.error("Failed to get displays:", e);
      setError(String(e));
    } finally {
      setIsLoading(false);
    }
  };

  // Request screen permission
  const requestPermission = async () => {
    try {
      const granted = await invoke<boolean>("request_screen_permission");
      setHasPermission(granted);
      if (granted) {
        await fetchDisplays();
      }
    } catch (e) {
      console.error("Failed to request permission:", e);
    }
  };

  // Start screen sharing
  const handleStartSharing = async () => {
    const displayId = selectedDisplay();
    if (displayId === null) return;

    try {
      setError(null);
      // Use broadcast_sharing_status which handles capture internally
      await invoke("broadcast_sharing_status", { isSharing: true, displayId });
      setIsSharing(true);
      console.log("Started sharing display:", displayId);
    } catch (e) {
      console.error("Failed to start sharing:", e);
      setError(`启动屏幕共享失败: ${e}`);
    }
  };

  // Stop screen sharing
  const handleStopSharing = async () => {
    try {
      // Use broadcast_sharing_status which handles capture stop internally
      await invoke("broadcast_sharing_status", { isSharing: false, displayId: null });
      setIsSharing(false);
      console.log("Stopped sharing");
    } catch (e) {
      console.error("Failed to stop sharing:", e);
      setError(`停止屏幕共享失败: ${e}`);
    }
  };

  onMount(() => {
    fetchDisplays();
  });

  return (
    <div class="max-w-4xl mx-auto space-y-6">
      {/* Error Display */}
      {error() && (
        <div class="bg-red-50 border border-red-200 text-red-700 px-4 py-3 rounded-lg flex items-center justify-between">
          <span>{error()}</span>
          <button
            class="text-red-500 hover:text-red-700"
            onClick={() => setError(null)}
          >
            ×
          </button>
        </div>
      )}

      {/* Permission Request */}
      <Show when={!hasPermission() && !isLoading()}>
        <div class="card bg-yellow-50 border-yellow-200">
          <div class="flex items-center gap-4">
            <span class="i-lucide-shield-alert text-yellow-500 text-2xl"></span>
            <div class="flex-1">
              <h3 class="font-medium text-yellow-800">需要屏幕录制权限</h3>
              <p class="text-sm text-yellow-700 mt-1">
                请在系统设置中授予屏幕录制权限，以便共享您的屏幕
              </p>
            </div>
            <button class="btn-primary" onClick={requestPermission}>
              <span class="i-lucide-key mr-2"></span>
              授权
            </button>
          </div>
        </div>
      </Show>

      {/* Sharing Status */}
      <Show when={isSharing()}>
        <div class="card bg-red-50 border-red-200">
          <div class="flex items-center justify-between">
            <div class="flex items-center gap-3">
              <span class="w-3 h-3 bg-red-500 rounded-full animate-pulse"></span>
              <div>
                <span class="font-medium text-red-700">正在共享屏幕</span>
                <p class="text-sm text-red-600">
                  {displays().find((d) => d.id === selectedDisplay())?.name}
                </p>
              </div>
            </div>
            <button
              class="btn bg-red-500 text-white hover:bg-red-600"
              onClick={handleStopSharing}
            >
              <span class="i-lucide-square mr-2"></span>
              停止共享
            </button>
          </div>
        </div>
      </Show>

      {/* Display Selection */}
      <div class="card">
        <div class="flex items-center justify-between mb-4">
          <h2 class="text-lg font-semibold text-gray-900">选择要共享的屏幕</h2>
          <button
            class="btn-secondary text-sm"
            onClick={fetchDisplays}
            disabled={isLoading()}
          >
            <span class={`i-lucide-refresh-cw mr-2 ${isLoading() ? "animate-spin" : ""}`}></span>
            刷新
          </button>
        </div>

        {/* Loading State */}
        <Show when={isLoading()}>
          <div class="text-center py-12 text-gray-500">
            <span class="i-lucide-loader-2 text-4xl mb-4 block opacity-50 animate-spin"></span>
            <p>正在获取显示器信息...</p>
          </div>
        </Show>

        {/* Display Grid */}
        <Show when={!isLoading() && displays().length > 0}>
          <div class="grid grid-cols-2 gap-4 mb-6">
            <For each={displays()}>
              {(display) => (
                <button
                  class={`p-4 border-2 rounded-xl text-left transition-all ${
                    selectedDisplay() === display.id
                      ? "border-primary-500 bg-primary-50"
                      : "border-gray-200 hover:border-gray-300"
                  }`}
                  onClick={() => setSelectedDisplay(display.id)}
                  disabled={isSharing()}
                >
                  <div class="aspect-video bg-gray-200 rounded-lg mb-3 flex items-center justify-center relative overflow-hidden">
                    <span class="i-lucide-monitor text-4xl text-gray-400"></span>
                    {selectedDisplay() === display.id && (
                      <div class="absolute inset-0 bg-primary-500/10 flex items-center justify-center">
                        <span class="i-lucide-check-circle text-primary-500 text-2xl"></span>
                      </div>
                    )}
                  </div>
                  <div class="flex items-center gap-2">
                    <span class="font-medium text-gray-900">{display.name}</span>
                    {display.primary && (
                      <span class="text-xs bg-primary-100 text-primary-700 px-2 py-0.5 rounded">
                        主屏
                      </span>
                    )}
                  </div>
                  <p class="text-sm text-gray-500 mt-1">
                    {display.width} × {display.height}
                    {display.scale_factor > 1 && (
                      <span class="ml-1 text-xs">@{display.scale_factor}x</span>
                    )}
                  </p>
                </button>
              )}
            </For>
          </div>
        </Show>

        {/* No Displays */}
        <Show when={!isLoading() && displays().length === 0 && hasPermission()}>
          <div class="text-center py-12 text-gray-500">
            <span class="i-lucide-monitor-off text-4xl mb-4 block opacity-50"></span>
            <p>未检测到显示器</p>
            <p class="text-sm mt-2">请检查显示器连接</p>
          </div>
        </Show>

        <button
          class="btn-primary w-full"
          disabled={selectedDisplay() === null || isSharing() || !hasPermission()}
          onClick={handleStartSharing}
        >
          <span class="i-lucide-cast mr-2"></span>
          开始共享
        </button>
      </div>

      {/* Settings */}
      <div class="card">
        <h2 class="text-lg font-semibold text-gray-900 mb-4">共享设置</h2>

        <div class="space-y-4">
          {/* FPS */}
          <div class="flex items-center justify-between">
            <div>
              <p class="font-medium text-gray-900">帧率</p>
              <p class="text-sm text-gray-500">更高的帧率需要更多带宽</p>
            </div>
            <select
              class="px-3 py-2 border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-primary-500"
              value={fps()}
              onChange={(e) => setFps(parseInt(e.currentTarget.value))}
              disabled={isSharing()}
            >
              <option value="30">30 FPS</option>
              <option value="60">60 FPS</option>
            </select>
          </div>

          {/* Quality */}
          <div class="flex items-center justify-between">
            <div>
              <p class="font-medium text-gray-900">画质</p>
              <p class="text-sm text-gray-500">自动根据网络调整</p>
            </div>
            <select
              class="px-3 py-2 border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-primary-500"
              value={quality()}
              onChange={(e) => setQuality(e.currentTarget.value)}
              disabled={isSharing()}
            >
              <option value="auto">自动</option>
              <option value="high">高画质</option>
              <option value="medium">中等</option>
              <option value="low">低画质</option>
            </select>
          </div>

          {/* Remote Control Toggle */}
          <div class="flex items-center justify-between">
            <div>
              <p class="font-medium text-gray-900">允许远程控制</p>
              <p class="text-sm text-gray-500">其他用户可以请求控制你的屏幕</p>
            </div>
            <button
              class={`w-12 h-6 rounded-full relative transition-colors ${
                allowRemoteControl() ? "bg-primary-500" : "bg-gray-300"
              }`}
              onClick={() => setAllowRemoteControl(!allowRemoteControl())}
              disabled={isSharing()}
            >
              <span
                class={`absolute top-1 w-4 h-4 bg-white rounded-full shadow transition-transform ${
                  allowRemoteControl() ? "right-1" : "left-1"
                }`}
              ></span>
            </button>
          </div>
        </div>
      </div>

      {/* Info */}
      <div class="card bg-blue-50 border-blue-200">
        <div class="flex items-start gap-3">
          <span class="i-lucide-info text-blue-500 text-lg mt-0.5"></span>
          <div class="text-sm text-blue-700">
            <p class="font-medium">屏幕共享说明</p>
            <ul class="mt-2 space-y-1 list-disc list-inside">
              <li>确保已连接到其他设备</li>
              <li>共享会自动使用 H.264 编码以获得最佳兼容性</li>
              <li>如遇到性能问题，可尝试降低帧率或画质</li>
            </ul>
          </div>
        </div>
      </div>
    </div>
  );
};
