import { Component, createSignal, For, onMount, onCleanup } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { listen, UnlistenFn } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";

interface FileInfo {
  id: string;
  name: string;
  size: number;
  checksum: string;
  mime_type: string | null;
}

interface FileTransfer {
  info: FileInfo;
  status: "Pending" | "Offered" | "InProgress" | "Completed" | "Failed" | "Cancelled";
  direction: "Outgoing" | "Incoming";
  progress: number;
  bytes_transferred: number;
  peer_id: string;
  local_path: string | null;
  error: string | null;
}

export const FileTransferPanel: Component = () => {
  const [transfers, setTransfers] = createSignal<FileTransfer[]>([]);
  const [downloadDir, setDownloadDir] = createSignal("");
  const [isLoading, setIsLoading] = createSignal(false);
  let unlistenOffer: UnlistenFn | undefined;
  let unlistenProgress: UnlistenFn | undefined;

  // Format file size
  const formatSize = (bytes: number): string => {
    if (bytes === 0) return "0 B";
    const k = 1024;
    const sizes = ["B", "KB", "MB", "GB"];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return parseFloat((bytes / Math.pow(k, i)).toFixed(2)) + " " + sizes[i];
  };

  // Status display
  const statusInfo: Record<FileTransfer["status"], { text: string; color: string; icon: string }> = {
    Pending: { text: "等待中", color: "text-gray-500", icon: "i-lucide-clock" },
    Offered: { text: "等待接受", color: "text-yellow-500", icon: "i-lucide-hourglass" },
    InProgress: { text: "传输中", color: "text-blue-500", icon: "i-lucide-loader-2 animate-spin" },
    Completed: { text: "已完成", color: "text-green-500", icon: "i-lucide-check-circle" },
    Failed: { text: "失败", color: "text-red-500", icon: "i-lucide-x-circle" },
    Cancelled: { text: "已取消", color: "text-gray-400", icon: "i-lucide-ban" },
  };

  // Fetch transfers
  const fetchTransfers = async () => {
    try {
      const result = await invoke<FileTransfer[]>("get_file_transfers");
      setTransfers(result);
    } catch (e) {
      console.error("Failed to get transfers:", e);
    }
  };

  // Fetch download directory
  const fetchDownloadDir = async () => {
    try {
      const dir = await invoke<string>("get_download_directory");
      setDownloadDir(dir);
    } catch (e) {
      console.error("Failed to get download directory:", e);
    }
  };

  // Select and offer a file
  const selectFile = async () => {
    try {
      const selected = await open({
        multiple: false,
        title: "选择要传输的文件",
      });

      if (selected) {
        // For now, we need a peer ID - in real use, this would come from connected device
        // TODO: Show device selector dialog
        const peerId = "localhost"; // Placeholder
        await invoke("offer_file", {
          filePath: selected,
          peerId,
        });
        await fetchTransfers();
      }
    } catch (e) {
      console.error("Failed to select file:", e);
    }
  };

  // Accept a file transfer
  const acceptTransfer = async (fileId: string) => {
    try {
      setIsLoading(true);
      await invoke("accept_file_transfer", { fileId, destPath: null });
      await fetchTransfers();
    } catch (e) {
      console.error("Failed to accept transfer:", e);
    } finally {
      setIsLoading(false);
    }
  };

  // Reject a file transfer
  const rejectTransfer = async (fileId: string) => {
    try {
      await invoke("reject_file_transfer", { fileId });
      await fetchTransfers();
    } catch (e) {
      console.error("Failed to reject transfer:", e);
    }
  };

  // Cancel a file transfer
  const cancelTransfer = async (fileId: string) => {
    try {
      await invoke("cancel_file_transfer", { fileId });
      await fetchTransfers();
    } catch (e) {
      console.error("Failed to cancel transfer:", e);
    }
  };

  onMount(async () => {
    // Listen for file offers
    unlistenOffer = await listen<FileTransfer>("file-offer", (event) => {
      setTransfers((prev) => {
        if (prev.some((t) => t.info.id === event.payload.info.id)) {
          return prev;
        }
        return [...prev, event.payload];
      });
    });

    // Listen for transfer progress updates
    unlistenProgress = await listen<{ file_id: string; progress: number; bytes: number }>(
      "file-progress",
      (event) => {
        setTransfers((prev) =>
          prev.map((t) =>
            t.info.id === event.payload.file_id
              ? {
                  ...t,
                  progress: event.payload.progress,
                  bytes_transferred: event.payload.bytes,
                  status: event.payload.progress >= 1 ? "Completed" : "InProgress",
                }
              : t
          )
        );
      }
    );

    await fetchDownloadDir();
    await fetchTransfers();

    // Poll for updates every 2 seconds
    const interval = setInterval(fetchTransfers, 2000);
    onCleanup(() => clearInterval(interval));
  });

  onCleanup(() => {
    unlistenOffer?.();
    unlistenProgress?.();
  });

  const activeTransfers = () => transfers().filter((t) =>
    t.status === "InProgress" || t.status === "Offered" || t.status === "Pending"
  );

  const completedTransfers = () => transfers().filter((t) =>
    t.status === "Completed" || t.status === "Failed" || t.status === "Cancelled"
  );

  return (
    <div class="max-w-4xl mx-auto space-y-6">
      {/* Header & Actions */}
      <div class="card">
        <div class="flex items-center justify-between mb-4">
          <div class="flex items-center gap-3">
            <span class="i-lucide-folder-up text-primary-500 text-xl"></span>
            <div>
              <h2 class="text-lg font-semibold text-gray-900">文件传输</h2>
              <p class="text-sm text-gray-500">
                下载目录: {downloadDir() || "加载中..."}
              </p>
            </div>
          </div>
          <button class="btn-primary" onClick={selectFile}>
            <span class="i-lucide-upload mr-2"></span>
            发送文件
          </button>
        </div>
      </div>

      {/* Active Transfers */}
      {activeTransfers().length > 0 && (
        <div class="card">
          <h3 class="text-md font-semibold text-gray-900 mb-4">
            进行中的传输
            <span class="ml-2 text-sm font-normal text-gray-500">
              ({activeTransfers().length})
            </span>
          </h3>

          <div class="space-y-3">
            <For each={activeTransfers()}>
              {(transfer) => (
                <div class="p-4 bg-gray-50 rounded-lg">
                  <div class="flex items-center justify-between mb-2">
                    <div class="flex items-center gap-3">
                      <span
                        class={`${
                          transfer.direction === "Incoming"
                            ? "i-lucide-download"
                            : "i-lucide-upload"
                        } text-gray-600`}
                      ></span>
                      <div>
                        <h4 class="font-medium text-gray-900">
                          {transfer.info.name}
                        </h4>
                        <p class="text-sm text-gray-500">
                          {formatSize(transfer.info.size)} · {transfer.peer_id}
                        </p>
                      </div>
                    </div>

                    <div class="flex items-center gap-2">
                      <span class={`${statusInfo[transfer.status].icon} ${statusInfo[transfer.status].color}`}></span>
                      <span class={`text-sm ${statusInfo[transfer.status].color}`}>
                        {statusInfo[transfer.status].text}
                      </span>
                    </div>
                  </div>

                  {/* Progress bar */}
                  {transfer.status === "InProgress" && (
                    <div class="mt-3">
                      <div class="flex justify-between text-sm text-gray-500 mb-1">
                        <span>{formatSize(transfer.bytes_transferred)}</span>
                        <span>{Math.round(transfer.progress * 100)}%</span>
                      </div>
                      <div class="w-full h-2 bg-gray-200 rounded-full overflow-hidden">
                        <div
                          class="h-full bg-primary-500 transition-all duration-300"
                          style={{ width: `${transfer.progress * 100}%` }}
                        ></div>
                      </div>
                    </div>
                  )}

                  {/* Actions for offered files */}
                  {transfer.status === "Offered" && transfer.direction === "Incoming" && (
                    <div class="flex gap-2 mt-3">
                      <button
                        class="btn-primary text-sm"
                        onClick={() => acceptTransfer(transfer.info.id)}
                        disabled={isLoading()}
                      >
                        <span class="i-lucide-check mr-1"></span>
                        接受
                      </button>
                      <button
                        class="btn-secondary text-sm"
                        onClick={() => rejectTransfer(transfer.info.id)}
                      >
                        <span class="i-lucide-x mr-1"></span>
                        拒绝
                      </button>
                    </div>
                  )}

                  {/* Cancel button */}
                  {(transfer.status === "InProgress" || transfer.status === "Pending") && (
                    <button
                      class="btn-secondary text-sm mt-3"
                      onClick={() => cancelTransfer(transfer.info.id)}
                    >
                      <span class="i-lucide-x mr-1"></span>
                      取消
                    </button>
                  )}
                </div>
              )}
            </For>
          </div>
        </div>
      )}

      {/* Completed Transfers */}
      {completedTransfers().length > 0 && (
        <div class="card">
          <h3 class="text-md font-semibold text-gray-900 mb-4">
            历史记录
            <span class="ml-2 text-sm font-normal text-gray-500">
              ({completedTransfers().length})
            </span>
          </h3>

          <div class="space-y-2">
            <For each={completedTransfers()}>
              {(transfer) => (
                <div class="flex items-center justify-between p-3 bg-gray-50 rounded-lg">
                  <div class="flex items-center gap-3">
                    <span
                      class={`${statusInfo[transfer.status].icon} ${statusInfo[transfer.status].color}`}
                    ></span>
                    <div>
                      <h4 class="font-medium text-gray-900 text-sm">
                        {transfer.info.name}
                      </h4>
                      <p class="text-xs text-gray-500">
                        {formatSize(transfer.info.size)}
                        {transfer.error && ` · ${transfer.error}`}
                      </p>
                    </div>
                  </div>
                  <span class={`text-xs ${statusInfo[transfer.status].color}`}>
                    {statusInfo[transfer.status].text}
                  </span>
                </div>
              )}
            </For>
          </div>
        </div>
      )}

      {/* Empty State */}
      {transfers().length === 0 && (
        <div class="card">
          <div class="text-center py-12 text-gray-500">
            <span class="i-lucide-folder-open text-4xl mb-4 block opacity-50"></span>
            <p>暂无文件传输</p>
            <p class="text-sm mt-2">点击"发送文件"开始传输</p>
          </div>
        </div>
      )}
    </div>
  );
};
