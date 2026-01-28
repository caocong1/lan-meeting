import { Component, createSignal, onMount } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { DeviceList } from "./components/DeviceList";
import { ScreenShare } from "./components/ScreenShare";
import { Chat } from "./components/Chat";
import { FileTransferPanel } from "./components/FileTransfer";

type TabId = "devices" | "share" | "chat" | "files";

interface SelfInfo {
  id: string;
  name: string;
}

const App: Component = () => {
  const [activeTab, setActiveTab] = createSignal<TabId>("devices");
  const [selfInfo, setSelfInfo] = createSignal<SelfInfo | null>(null);
  const [isConnected, setIsConnected] = createSignal(false);

  const tabs: { id: TabId; label: string; icon: string }[] = [
    { id: "devices", label: "设备", icon: "i-lucide-users" },
    { id: "share", label: "共享", icon: "i-lucide-cast" },
    { id: "chat", label: "聊天", icon: "i-lucide-message-square" },
    { id: "files", label: "文件", icon: "i-lucide-folder" },
  ];

  // Fetch self info on mount
  onMount(async () => {
    try {
      const info = await invoke<SelfInfo>("get_self_info");
      setSelfInfo(info);
      setIsConnected(true);
    } catch (e) {
      console.error("Failed to get self info:", e);
    }
  });

  const renderContent = () => {
    switch (activeTab()) {
      case "devices":
        return <DeviceList />;
      case "share":
        return <ScreenShare />;
      case "chat":
        return <Chat />;
      case "files":
        return <FileTransferPanel />;
      default:
        return <DeviceList />;
    }
  };

  return (
    <div class="h-full flex flex-col bg-gray-50">
      {/* Header */}
      <header class="bg-white border-b border-gray-200 px-6 py-4">
        <div class="flex items-center justify-between">
          <div class="flex items-center gap-3">
            <div class="w-10 h-10 bg-primary-500 rounded-xl flex items-center justify-center">
              <span class="i-lucide-monitor text-white text-xl"></span>
            </div>
            <div>
              <h1 class="text-xl font-semibold text-gray-900">LAN Meeting</h1>
              <p class="text-sm text-gray-500">局域网屏幕共享</p>
            </div>
          </div>

          {/* Self Info & Status */}
          <div class="flex items-center gap-4">
            {selfInfo() && (
              <div class="text-right">
                <p class="text-sm font-medium text-gray-900">{selfInfo()!.name}</p>
                <p class="text-xs text-gray-500 font-mono">{selfInfo()!.id.slice(0, 8)}</p>
              </div>
            )}
            <div class="flex items-center gap-2 text-sm text-gray-600">
              <span
                class={`w-2 h-2 rounded-full ${isConnected() ? "bg-green-500" : "bg-gray-400"}`}
              ></span>
              <span>{isConnected() ? "服务运行中" : "未连接"}</span>
            </div>
          </div>
        </div>
      </header>

      {/* Tab Navigation */}
      <nav class="bg-white border-b border-gray-200 px-6">
        <div class="flex gap-1">
          {tabs.map((tab) => (
            <button
              class={`px-4 py-3 text-sm font-medium border-b-2 transition-colors flex items-center gap-2 ${
                activeTab() === tab.id
                  ? "border-primary-500 text-primary-600"
                  : "border-transparent text-gray-500 hover:text-gray-700"
              }`}
              onClick={() => setActiveTab(tab.id)}
            >
              <span class={tab.icon}></span>
              {tab.label}
            </button>
          ))}
        </div>
      </nav>

      {/* Main Content */}
      <main class="flex-1 overflow-auto p-6">{renderContent()}</main>

      {/* Footer */}
      <footer class="bg-white border-t border-gray-200 px-6 py-2">
        <div class="flex items-center justify-between text-xs text-gray-500">
          <span>LAN Meeting v0.1.0</span>
          <span>H.264 · QUIC · 端到端加密</span>
        </div>
      </footer>
    </div>
  );
};

export default App;
