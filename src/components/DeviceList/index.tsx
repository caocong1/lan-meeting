import { Component, createSignal, For, onMount, onCleanup } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { listen, UnlistenFn } from "@tauri-apps/api/event";

interface Device {
  id: string;
  name: string;
  ip: string;
  port: number;
  status: "online" | "busy" | "offline";
  last_seen: number;
}

export const DeviceList: Component = () => {
  const [devices, setDevices] = createSignal<Device[]>([]);
  const [manualIp, setManualIp] = createSignal("");
  const [isLoading, setIsLoading] = createSignal(true);
  const [error, setError] = createSignal<string | null>(null);

  let unlistenDiscovered: UnlistenFn | undefined;
  let unlistenRemoved: UnlistenFn | undefined;

  const statusColors = {
    online: "bg-green-500",
    busy: "bg-yellow-500",
    offline: "bg-gray-400",
  };

  const statusText = {
    online: "在线",
    busy: "忙碌",
    offline: "离线",
  };

  // Fetch devices from backend
  const fetchDevices = async () => {
    try {
      setIsLoading(true);
      setError(null);
      const result = await invoke<Device[]>("get_devices");
      setDevices(result);
    } catch (e) {
      console.error("Failed to get devices:", e);
      setError(String(e));
    } finally {
      setIsLoading(false);
    }
  };

  // Handle device discovered event
  const handleDeviceDiscovered = (device: Device) => {
    setDevices((prev) => {
      const existing = prev.find((d) => d.id === device.id);
      if (existing) {
        return prev.map((d) => (d.id === device.id ? device : d));
      }
      return [...prev, device];
    });
  };

  // Handle device removed event
  const handleDeviceRemoved = (deviceId: string) => {
    setDevices((prev) => prev.filter((d) => d.id !== deviceId));
  };

  // Setup event listeners
  onMount(async () => {
    // Listen for device discovery events
    unlistenDiscovered = await listen<Device>("device-discovered", (event) => {
      handleDeviceDiscovered(event.payload);
    });

    unlistenRemoved = await listen<string>("device-removed", (event) => {
      handleDeviceRemoved(event.payload);
    });

    // Initial fetch
    await fetchDevices();
  });

  // Cleanup listeners
  onCleanup(() => {
    unlistenDiscovered?.();
    unlistenRemoved?.();
  });

  const handleConnect = async (device: Device) => {
    try {
      await invoke("connect_to_device", { deviceId: device.id });
      console.log("Connected to:", device);
      // Update device status locally
      setDevices((prev) =>
        prev.map((d) => (d.id === device.id ? { ...d, status: "busy" as const } : d))
      );
    } catch (e) {
      console.error("Failed to connect:", e);
      setError(`连接失败: ${e}`);
    }
  };

  const handleManualConnect = async () => {
    const ip = manualIp().trim();
    if (!ip) return;

    try {
      const device = await invoke<Device>("add_manual_device", { ip });
      handleDeviceDiscovered(device);
      setManualIp("");
    } catch (e) {
      console.error("Failed to add manual device:", e);
      setError(`添加设备失败: ${e}`);
    }
  };

  return (
    <div class="max-w-4xl mx-auto space-y-6">
      {/* Error Display */}
      {error() && (
        <div class="bg-red-50 border border-red-200 text-red-700 px-4 py-3 rounded-lg">
          {error()}
          <button
            class="ml-2 text-red-500 hover:text-red-700"
            onClick={() => setError(null)}
          >
            ×
          </button>
        </div>
      )}

      {/* Manual Connection */}
      <div class="card">
        <h2 class="text-lg font-semibold text-gray-900 mb-4">手动连接</h2>
        <div class="flex gap-3">
          <input
            type="text"
            placeholder="输入 IP 地址，如 192.168.1.100"
            value={manualIp()}
            onInput={(e) => setManualIp(e.currentTarget.value)}
            onKeyDown={(e) => e.key === "Enter" && handleManualConnect()}
            class="flex-1 px-4 py-2 border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-primary-500 focus:border-transparent"
          />
          <button class="btn-primary" onClick={handleManualConnect}>
            <span class="i-lucide-link mr-2"></span>
            连接
          </button>
        </div>
      </div>

      {/* Device List */}
      <div class="card">
        <div class="flex items-center justify-between mb-4">
          <h2 class="text-lg font-semibold text-gray-900">
            发现的设备
            {devices().length > 0 && (
              <span class="ml-2 text-sm font-normal text-gray-500">
                ({devices().length})
              </span>
            )}
          </h2>
          <button
            class="btn-secondary text-sm"
            onClick={fetchDevices}
            disabled={isLoading()}
          >
            <span
              class={`i-lucide-refresh-cw mr-2 ${isLoading() ? "animate-spin" : ""}`}
            ></span>
            刷新
          </button>
        </div>

        <div class="space-y-3">
          <For each={devices()}>
            {(device) => (
              <div class="flex items-center justify-between p-4 bg-gray-50 rounded-lg hover:bg-gray-100 transition-colors">
                <div class="flex items-center gap-4">
                  <div class="w-12 h-12 bg-gray-200 rounded-xl flex items-center justify-center">
                    <span class="i-lucide-monitor text-gray-600 text-xl"></span>
                  </div>
                  <div>
                    <h3 class="font-medium text-gray-900">{device.name}</h3>
                    <p class="text-sm text-gray-500">
                      {device.ip}:{device.port}
                    </p>
                  </div>
                </div>

                <div class="flex items-center gap-4">
                  <div class="flex items-center gap-2">
                    <span
                      class={`w-2 h-2 rounded-full ${statusColors[device.status]}`}
                    ></span>
                    <span class="text-sm text-gray-600">
                      {statusText[device.status]}
                    </span>
                  </div>

                  <button
                    class="btn-primary text-sm"
                    disabled={device.status === "offline"}
                    onClick={() => handleConnect(device)}
                  >
                    {device.status === "busy" ? "请求控制" : "连接"}
                  </button>
                </div>
              </div>
            )}
          </For>

          {isLoading() && devices().length === 0 && (
            <div class="text-center py-12 text-gray-500">
              <span class="i-lucide-loader-2 text-4xl mb-4 block opacity-50 animate-spin"></span>
              <p>正在搜索局域网设备...</p>
            </div>
          )}

          {!isLoading() && devices().length === 0 && (
            <div class="text-center py-12 text-gray-500">
              <span class="i-lucide-wifi-off text-4xl mb-4 block opacity-50"></span>
              <p>未发现设备</p>
              <p class="text-sm mt-2">请确保其他设备已启动 LAN Meeting</p>
            </div>
          )}
        </div>
      </div>
    </div>
  );
};
