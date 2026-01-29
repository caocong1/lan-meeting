import { Component, createSignal } from "solid-js";
import { invoke } from "@tauri-apps/api/core";

interface AddDeviceModalProps {
  onClose: () => void;
  onAdded: () => void;
}

export const AddDeviceModal: Component<AddDeviceModalProps> = (props) => {
  const [ip, setIp] = createSignal("");
  const [isAdding, setIsAdding] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  const handleAdd = async () => {
    const ipValue = ip().trim();
    if (!ipValue) {
      setError("请输入 IP 地址");
      return;
    }

    // Basic IP validation
    const ipRegex = /^(\d{1,3}\.){3}\d{1,3}$/;
    if (!ipRegex.test(ipValue)) {
      setError("IP 地址格式不正确");
      return;
    }

    setIsAdding(true);
    setError(null);

    try {
      await invoke("add_manual_device", { ip: ipValue });
      props.onAdded();
    } catch (e) {
      console.error("Failed to add device:", e);
      setError(`添加失败: ${e}`);
    } finally {
      setIsAdding(false);
    }
  };

  const handleKeyDown = (e: KeyboardEvent) => {
    if (e.key === "Enter" && !isAdding()) {
      handleAdd();
    }
    if (e.key === "Escape") {
      props.onClose();
    }
  };

  return (
    <div class="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div class="bg-white rounded-2xl shadow-xl w-full max-w-sm mx-4 overflow-hidden">
        {/* Header */}
        <div class="px-6 py-4 border-b border-gray-200 flex items-center justify-between">
          <h2 class="text-lg font-semibold text-gray-900">添加设备</h2>
          <button
            class="p-1 text-gray-400 hover:text-gray-600 rounded"
            onClick={props.onClose}
          >
            <span class="i-lucide-x text-xl"></span>
          </button>
        </div>

        {/* Content */}
        <div class="p-6">
          {/* Error */}
          {error() && (
            <div class="bg-red-50 border border-red-200 text-red-700 px-4 py-3 rounded-lg text-sm mb-4">
              {error()}
            </div>
          )}

          <label class="block text-sm font-medium text-gray-700 mb-2">
            IP 地址
          </label>
          <input
            type="text"
            value={ip()}
            onInput={(e) => setIp(e.currentTarget.value)}
            onKeyDown={handleKeyDown}
            class="w-full px-4 py-3 border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-primary-500 focus:border-transparent text-lg font-mono"
            placeholder="192.168.1.100"
            autofocus
          />
          <p class="text-xs text-gray-500 mt-2">
            输入对方设备的局域网 IP 地址
          </p>
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
            class="px-4 py-2 bg-primary-500 hover:bg-primary-600 text-white rounded-lg text-sm font-medium disabled:opacity-50 flex items-center gap-2"
            onClick={handleAdd}
            disabled={isAdding() || !ip().trim()}
          >
            {isAdding() ? (
              <>
                <span class="i-lucide-loader-2 animate-spin"></span>
                连接中...
              </>
            ) : (
              <>
                <span class="i-lucide-plus"></span>
                添加
              </>
            )}
          </button>
        </div>
      </div>
    </div>
  );
};
