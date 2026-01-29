import { Component, createSignal, onMount, Show } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { MeetingRoom } from "./components/MeetingRoom";
import { Settings } from "./components/Settings";

export interface SelfInfo {
  id: string;
  name: string;
  ip: string;
}

const App: Component = () => {
  const [isServiceEnabled, setIsServiceEnabled] = createSignal(false);
  const [isLoading, setIsLoading] = createSignal(false);
  const [selfInfo, setSelfInfo] = createSignal<SelfInfo | null>(null);
  const [showSettings, setShowSettings] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  // Fetch self info on mount
  onMount(async () => {
    try {
      const info = await invoke<SelfInfo>("get_self_info");
      setSelfInfo(info);
    } catch (e) {
      console.error("Failed to get self info:", e);
    }
  });

  // Start service
  const handleStartService = async () => {
    setIsLoading(true);
    setError(null);
    try {
      await invoke("start_service");
      setIsServiceEnabled(true);
    } catch (e) {
      console.error("Failed to start service:", e);
      setError(`启动服务失败: ${e}`);
    } finally {
      setIsLoading(false);
    }
  };

  // Stop service
  const handleStopService = async () => {
    setIsLoading(true);
    try {
      await invoke("stop_service");
      setIsServiceEnabled(false);
    } catch (e) {
      console.error("Failed to stop service:", e);
      setError(`停止服务失败: ${e}`);
    } finally {
      setIsLoading(false);
    }
  };

  return (
    <div class="h-full flex flex-col bg-gray-50">
      {/* Settings Modal */}
      <Show when={showSettings()}>
        <Settings onClose={() => setShowSettings(false)} />
      </Show>

      {/* Service Disabled - Small Window */}
      <Show when={!isServiceEnabled()}>
        <div class="flex-1 flex items-center justify-center p-8">
          <div class="text-center max-w-sm">
            {/* Logo */}
            <div class="w-20 h-20 bg-primary-500 rounded-2xl flex items-center justify-center mx-auto mb-6">
              <span class="i-lucide-monitor text-white text-4xl"></span>
            </div>

            <h1 class="text-2xl font-bold text-gray-900 mb-2">LAN Meeting</h1>
            <p class="text-gray-500 mb-8">局域网屏幕共享工具</p>

            {/* Error Display */}
            {error() && (
              <div class="bg-red-50 border border-red-200 text-red-700 px-4 py-3 rounded-lg mb-4 text-sm">
                {error()}
              </div>
            )}

            {/* Start Service Button */}
            <button
              class="w-full py-4 bg-primary-500 hover:bg-primary-600 text-white font-medium rounded-xl transition-colors flex items-center justify-center gap-2 disabled:opacity-50"
              onClick={handleStartService}
              disabled={isLoading()}
            >
              {isLoading() ? (
                <>
                  <span class="i-lucide-loader-2 animate-spin"></span>
                  启动中...
                </>
              ) : (
                <>
                  <span class="i-lucide-power"></span>
                  开启服务
                </>
              )}
            </button>

            {/* Self Info */}
            {selfInfo() && (
              <p class="text-xs text-gray-400 mt-4">
                {selfInfo()!.name} · {selfInfo()!.ip || "获取IP中..."}
              </p>
            )}

            {/* Settings Link */}
            <button
              class="mt-6 text-sm text-gray-500 hover:text-gray-700 flex items-center justify-center gap-1 mx-auto"
              onClick={() => setShowSettings(true)}
            >
              <span class="i-lucide-settings text-lg"></span>
              设置
            </button>
          </div>
        </div>
      </Show>

      {/* Service Enabled - Meeting Room */}
      <Show when={isServiceEnabled()}>
        <MeetingRoom
          selfInfo={selfInfo()!}
          onStopService={handleStopService}
          onOpenSettings={() => setShowSettings(true)}
          isLoading={isLoading()}
        />
      </Show>
    </div>
  );
};

export default App;
