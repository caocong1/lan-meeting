import { Component, createSignal, onMount } from "solid-js";
import { invoke } from "@tauri-apps/api/core";

interface SettingsProps {
  onClose: () => void;
}

interface AppSettings {
  device_name: string;
  quality: "auto" | "high" | "medium" | "low";
  fps: number;
}

export const Settings: Component<SettingsProps> = (props) => {
  const [settings, setSettings] = createSignal<AppSettings>({
    device_name: "",
    quality: "auto",
    fps: 30,
  });
  const [isSaving, setIsSaving] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [success, setSuccess] = createSignal(false);

  // Load settings on mount
  onMount(async () => {
    try {
      const saved = await invoke<AppSettings>("get_settings");
      setSettings(saved);
    } catch (e) {
      console.error("Failed to load settings:", e);
      // Use defaults
      const hostname = await invoke<{ name: string }>("get_self_info");
      setSettings(prev => ({ ...prev, device_name: hostname.name }));
    }
  });

  // Save settings
  const handleSave = async () => {
    setIsSaving(true);
    setError(null);
    setSuccess(false);

    try {
      await invoke("save_settings", { settings: settings() });
      setSuccess(true);
      setTimeout(() => setSuccess(false), 2000);
    } catch (e) {
      console.error("Failed to save settings:", e);
      setError(`保存失败: ${e}`);
    } finally {
      setIsSaving(false);
    }
  };

  return (
    <div class="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div class="bg-white rounded-2xl shadow-xl w-full max-w-md mx-4 overflow-hidden">
        {/* Header */}
        <div class="px-6 py-4 border-b border-gray-200 flex items-center justify-between">
          <h2 class="text-lg font-semibold text-gray-900">设置</h2>
          <button
            class="p-1 text-gray-400 hover:text-gray-600 rounded"
            onClick={props.onClose}
          >
            <span class="i-lucide-x text-xl"></span>
          </button>
        </div>

        {/* Content */}
        <div class="p-6 space-y-6">
          {/* Error */}
          {error() && (
            <div class="bg-red-50 border border-red-200 text-red-700 px-4 py-3 rounded-lg text-sm">
              {error()}
            </div>
          )}

          {/* Success */}
          {success() && (
            <div class="bg-green-50 border border-green-200 text-green-700 px-4 py-3 rounded-lg text-sm flex items-center gap-2">
              <span class="i-lucide-check-circle"></span>
              设置已保存
            </div>
          )}

          {/* Device Name */}
          <div>
            <label class="block text-sm font-medium text-gray-700 mb-2">
              设备名称
            </label>
            <input
              type="text"
              value={settings().device_name}
              onInput={(e) => setSettings(prev => ({ ...prev, device_name: e.currentTarget.value }))}
              class="w-full px-4 py-2 border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-primary-500 focus:border-transparent"
              placeholder="输入设备名称"
            />
            <p class="text-xs text-gray-500 mt-1">其他人将看到此名称</p>
          </div>

          {/* Quality */}
          <div>
            <label class="block text-sm font-medium text-gray-700 mb-2">
              画质
            </label>
            <select
              value={settings().quality}
              onChange={(e) => setSettings(prev => ({ ...prev, quality: e.currentTarget.value as AppSettings["quality"] }))}
              class="w-full px-4 py-2 border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-primary-500 focus:border-transparent"
            >
              <option value="auto">自动 (根据网络调整)</option>
              <option value="high">高画质</option>
              <option value="medium">中等</option>
              <option value="low">低画质 (省流量)</option>
            </select>
          </div>

          {/* FPS */}
          <div>
            <label class="block text-sm font-medium text-gray-700 mb-2">
              帧率
            </label>
            <select
              value={settings().fps}
              onChange={(e) => setSettings(prev => ({ ...prev, fps: parseInt(e.currentTarget.value) }))}
              class="w-full px-4 py-2 border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-primary-500 focus:border-transparent"
            >
              <option value="15">15 FPS (省流量)</option>
              <option value="30">30 FPS (推荐)</option>
              <option value="60">60 FPS (高流畅)</option>
            </select>
            <p class="text-xs text-gray-500 mt-1">更高的帧率需要更多带宽</p>
          </div>
        </div>

        {/* Footer */}
        <div class="px-6 py-4 bg-gray-50 border-t border-gray-200 flex justify-end gap-3">
          <button
            class="px-4 py-2 text-gray-700 hover:bg-gray-100 rounded-lg text-sm font-medium"
            onClick={props.onClose}
          >
            取消
          </button>
          <button
            class="px-4 py-2 bg-primary-500 hover:bg-primary-600 text-white rounded-lg text-sm font-medium disabled:opacity-50"
            onClick={handleSave}
            disabled={isSaving()}
          >
            {isSaving() ? "保存中..." : "保存"}
          </button>
        </div>
      </div>
    </div>
  );
};
