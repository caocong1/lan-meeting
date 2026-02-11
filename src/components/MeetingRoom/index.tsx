import { Component, createSignal, For, onMount, onCleanup, Show } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { listen, UnlistenFn } from "@tauri-apps/api/event";
import { SelfInfo } from "../../App";
import { AddDeviceModal } from "../AddDeviceModal";

interface Member {
  id: string;
  name: string;
  ip: string;
  port: number;
  is_self: boolean;
  is_sharing: boolean;
}

interface MeetingRoomProps {
  selfInfo: SelfInfo;
  onStopService: () => void;
  onOpenSettings: () => void;
  isLoading: boolean;
}

export const MeetingRoom: Component<MeetingRoomProps> = (props) => {
  const [members, setMembers] = createSignal<Member[]>([]);
  const [isSharing, setIsSharing] = createSignal(false);
  const [isSimpleSharing, setIsSimpleSharing] = createSignal(false);
  const [isLoadingMembers, setIsLoadingMembers] = createSignal(true);
  const [showAddModal, setShowAddModal] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  let unlistenDiscovered: UnlistenFn | undefined;
  let unlistenRemoved: UnlistenFn | undefined;
  let unlistenSharingChanged: UnlistenFn | undefined;

  // Fetch members list
  const fetchMembers = async () => {
    try {
      setIsLoadingMembers(true);
      const devices = await invoke<any[]>("get_devices");

      // Convert devices to members, add self
      const memberList: Member[] = devices.map(d => ({
        id: d.id,
        name: d.name,
        ip: d.ip,
        port: d.port,
        is_self: false,
        is_sharing: d.is_sharing || false,
      }));

      // Add self to the list
      if (props.selfInfo) {
        memberList.unshift({
          id: props.selfInfo.id,
          name: props.selfInfo.name + " (me)",
          ip: props.selfInfo.ip,
          port: 19876,
          is_self: true,
          is_sharing: isSharing(),
        });
      }

      setMembers(memberList);
    } catch (e) {
      console.error("Failed to fetch members:", e);
      setError(`获取成员列表失败: ${e}`);
    } finally {
      setIsLoadingMembers(false);
    }
  };

  // Handle member discovered
  const handleMemberDiscovered = (device: any) => {
    setMembers(prev => {
      const existing = prev.find(m => m.id === device.id);
      if (existing) {
        return prev.map(m => m.id === device.id ? {
          ...m,
          name: device.name,
          ip: device.ip,
          is_sharing: device.is_sharing || false,
        } : m);
      }
      return [...prev, {
        id: device.id,
        name: device.name,
        ip: device.ip,
        port: device.port,
        is_self: false,
        is_sharing: device.is_sharing || false,
      }];
    });
  };

  // Handle member removed
  const handleMemberRemoved = (deviceId: string) => {
    setMembers(prev => prev.filter(m => m.id !== deviceId));
  };

  // Start sharing - navigate to share selection
  const handleStartSharing = async () => {
    try {
      // Check permission first (macOS)
      const hasPermission = await invoke<boolean>("check_screen_permission");
      if (!hasPermission) {
        await invoke("request_screen_permission");
        return;
      }

      // Get displays and let user select
      const displays = await invoke<any[]>("get_displays");
      if (displays.length === 0) {
        setError("未找到可共享的显示器");
        return;
      }

      // For now, use first display. TODO: Show selection UI
      const displayId = displays[0].id;
      // broadcast_sharing_status handles capture start internally
      await invoke("broadcast_sharing_status", { isSharing: true, displayId });
      setIsSharing(true);

      // Update self in member list
      setMembers(prev => prev.map(m =>
        m.is_self ? { ...m, is_sharing: true } : m
      ));
    } catch (e) {
      console.error("Failed to start sharing:", e);
      setError(`启动共享失败: ${e}`);
    }
  };

  // Stop sharing
  const handleStopSharing = async () => {
    try {
      // broadcast_sharing_status handles capture stop internally
      await invoke("broadcast_sharing_status", { isSharing: false, displayId: null });
      setIsSharing(false);

      // Update self in member list
      setMembers(prev => prev.map(m =>
        m.is_self ? { ...m, is_sharing: false } : m
      ));
    } catch (e) {
      console.error("Failed to stop sharing:", e);
      setError(`停止共享失败: ${e}`);
    }
  };

  // Watch member's screen (opens native GPU-rendered window)
  const handleWatchScreen = async (member: Member) => {
    try {
      // Request stream - native wgpu window will be created when stream starts
      await invoke("request_screen_stream", {
        peerIp: member.ip,
        peerName: member.name,
      });
    } catch (e) {
      console.error("Failed to request screen stream:", e);
      setError(`请求屏幕流失败: ${e}`);
    }
  };

  // Request control
  const handleRequestControl = async (member: Member) => {
    try {
      await invoke("request_control", { peerId: member.id });
    } catch (e) {
      console.error("Failed to request control:", e);
      setError(`请求控制失败: ${e}`);
    }
  };

  // ===== Simple streaming handlers (minimal pipeline for debugging) =====

  const handleSimpleStartSharing = async () => {
    try {
      const hasPermission = await invoke<boolean>("check_screen_permission");
      if (!hasPermission) {
        await invoke("request_screen_permission");
        return;
      }

      const displays = await invoke<any[]>("get_displays");
      if (displays.length === 0) {
        setError("未找到可共享的显示器");
        return;
      }

      const displayId = displays[0].id;
      await invoke("simple_start_sharing", { displayId });
      setIsSimpleSharing(true);
    } catch (e) {
      console.error("[SIMPLE] Failed to start sharing:", e);
      setError(`[Simple] 启动共享失败: ${e}`);
    }
  };

  const handleSimpleStopSharing = async () => {
    try {
      await invoke("simple_stop_sharing");
      setIsSimpleSharing(false);
    } catch (e) {
      console.error("[SIMPLE] Failed to stop sharing:", e);
      setError(`[Simple] 停止共享失败: ${e}`);
    }
  };

  const handleSimpleWatch = async (member: Member) => {
    try {
      await invoke("simple_request_stream", { peerIp: member.ip });
    } catch (e) {
      console.error("[SIMPLE] Failed to request stream:", e);
      setError(`[Simple] 请求屏幕流失败: ${e}`);
    }
  };

  // Setup event listeners
  onMount(async () => {
    unlistenDiscovered = await listen<any>("device-discovered", (event) => {
      handleMemberDiscovered(event.payload);
    });

    unlistenRemoved = await listen<string>("device-removed", (event) => {
      handleMemberRemoved(event.payload);
    });

    unlistenSharingChanged = await listen<{ device_id: string; is_sharing: boolean }>(
      "sharing-status-changed",
      (event) => {
        setMembers(prev => prev.map(m =>
          m.id === event.payload.device_id
            ? { ...m, is_sharing: event.payload.is_sharing }
            : m
        ));
      }
    );

    await fetchMembers();
  });

  onCleanup(() => {
    unlistenDiscovered?.();
    unlistenRemoved?.();
    unlistenSharingChanged?.();
  });

  return (
    <div class="h-full flex flex-col">
      {/* Add Device Modal */}
      <Show when={showAddModal()}>
        <AddDeviceModal
          onClose={() => setShowAddModal(false)}
          onAdded={() => {
            setShowAddModal(false);
            fetchMembers();
          }}
        />
      </Show>

      {/* Header */}
      <header class="bg-white border-b border-gray-200 px-4 py-3">
        <div class="flex items-center justify-between">
          <div class="flex items-center gap-3">
            <div class="w-8 h-8 bg-primary-500 rounded-lg flex items-center justify-center">
              <span class="i-lucide-monitor text-white text-sm"></span>
            </div>
            <div>
              <h1 class="text-lg font-semibold text-gray-900">LAN Meeting</h1>
            </div>
          </div>

          <div class="flex items-center gap-2">
            {/* Share Button */}
            {isSharing() ? (
              <button
                class="px-4 py-2 bg-red-500 hover:bg-red-600 text-white text-sm font-medium rounded-lg flex items-center gap-2"
                onClick={handleStopSharing}
              >
                <span class="w-2 h-2 bg-white rounded-full animate-pulse"></span>
                停止共享
              </button>
            ) : (
              <button
                class="px-4 py-2 bg-primary-500 hover:bg-primary-600 text-white text-sm font-medium rounded-lg flex items-center gap-2"
                onClick={handleStartSharing}
              >
                <span class="i-lucide-cast"></span>
                开始共享
              </button>
            )}

            {/* Simple Share Button (debugging) */}
            {isSimpleSharing() ? (
              <button
                class="px-4 py-2 bg-orange-500 hover:bg-orange-600 text-white text-sm font-medium rounded-lg flex items-center gap-2"
                onClick={handleSimpleStopSharing}
              >
                <span class="w-2 h-2 bg-white rounded-full animate-pulse"></span>
                Simple Stop
              </button>
            ) : (
              <button
                class="px-4 py-2 bg-orange-400 hover:bg-orange-500 text-white text-sm font-medium rounded-lg flex items-center gap-2"
                onClick={handleSimpleStartSharing}
              >
                Simple Share
              </button>
            )}

            {/* Settings */}
            <button
              class="p-2 text-gray-500 hover:text-gray-700 hover:bg-gray-100 rounded-lg"
              onClick={props.onOpenSettings}
            >
              <span class="i-lucide-settings text-xl"></span>
            </button>
          </div>
        </div>
      </header>

      {/* Error Display */}
      {error() && (
        <div class="mx-4 mt-4 bg-red-50 border border-red-200 text-red-700 px-4 py-3 rounded-lg flex items-center justify-between">
          <span class="text-sm">{error()}</span>
          <button
            class="text-red-500 hover:text-red-700"
            onClick={() => setError(null)}
          >
            <span class="i-lucide-x"></span>
          </button>
        </div>
      )}

      {/* Member List Header */}
      <div class="px-4 py-3 flex items-center justify-between border-b border-gray-100">
        <div class="flex items-center gap-2">
          <span class="text-sm font-medium text-gray-700">
            会议成员
          </span>
          <span class="px-2 py-0.5 bg-gray-100 text-gray-600 text-xs rounded-full">
            {members().length}
          </span>
        </div>
        <div class="flex items-center gap-2">
          <button
            class="p-2 text-gray-500 hover:text-gray-700 hover:bg-gray-100 rounded-lg"
            onClick={() => setShowAddModal(true)}
            title="添加设备"
          >
            <span class="i-lucide-user-plus text-lg"></span>
          </button>
          <button
            class="p-2 text-gray-500 hover:text-gray-700 hover:bg-gray-100 rounded-lg"
            onClick={fetchMembers}
            disabled={isLoadingMembers()}
            title="刷新"
          >
            <span class={`i-lucide-refresh-cw text-lg ${isLoadingMembers() ? 'animate-spin' : ''}`}></span>
          </button>
        </div>
      </div>

      {/* Member List */}
      <div class="flex-1 overflow-auto p-4">
        <div class="space-y-2">
          <For each={members()}>
            {(member) => (
              <div class={`p-4 bg-white rounded-xl border ${member.is_self ? 'border-primary-200 bg-primary-50/30' : 'border-gray-200'} hover:shadow-sm transition-shadow`}>
                <div class="flex items-center justify-between">
                  <div class="flex items-center gap-3">
                    {/* Avatar */}
                    <div class={`w-10 h-10 rounded-full flex items-center justify-center ${member.is_self ? 'bg-primary-100' : 'bg-gray-100'}`}>
                      <span class={`i-lucide-user text-lg ${member.is_self ? 'text-primary-600' : 'text-gray-500'}`}></span>
                    </div>

                    {/* Info */}
                    <div>
                      <div class="flex items-center gap-2">
                        <span class="font-medium text-gray-900">{member.name}</span>
                        {member.is_self && (
                          <span class="px-1.5 py-0.5 bg-primary-100 text-primary-700 text-xs rounded">
                            我
                          </span>
                        )}
                      </div>
                      <span class="text-sm text-gray-500">{member.ip}</span>
                    </div>
                  </div>

                  {/* Actions */}
                  <div class="flex items-center gap-2">
                    {member.is_sharing ? (
                      <>
                        <span class="flex items-center gap-1.5 px-2 py-1 bg-red-100 text-red-700 text-xs rounded-full">
                          <span class="w-1.5 h-1.5 bg-red-500 rounded-full animate-pulse"></span>
                          正在共享
                        </span>
                        {!member.is_self && (
                          <>
                            <button
                              class="px-3 py-1.5 bg-primary-500 hover:bg-primary-600 text-white text-sm rounded-lg"
                              onClick={() => handleWatchScreen(member)}
                            >
                              观看
                            </button>
                            <button
                              class="px-3 py-1.5 bg-orange-400 hover:bg-orange-500 text-white text-sm rounded-lg"
                              onClick={() => handleSimpleWatch(member)}
                            >
                              Simple
                            </button>
                            <button
                              class="px-3 py-1.5 border border-gray-300 hover:bg-gray-50 text-gray-700 text-sm rounded-lg"
                              onClick={() => handleRequestControl(member)}
                            >
                              请求控制
                            </button>
                          </>
                        )}
                      </>
                    ) : (
                      <div class="flex items-center gap-2">
                        <span class="text-sm text-gray-400">未共享</span>
                        {!member.is_self && (
                          <button
                            class="px-3 py-1.5 bg-orange-400 hover:bg-orange-500 text-white text-sm rounded-lg"
                            onClick={() => handleSimpleWatch(member)}
                          >
                            Simple Watch
                          </button>
                        )}
                      </div>
                    )}
                  </div>
                </div>
              </div>
            )}
          </For>

          {/* Empty State */}
          <Show when={!isLoadingMembers() && members().length === 0}>
            <div class="text-center py-12 text-gray-500">
              <span class="i-lucide-users text-4xl mb-4 block opacity-50"></span>
              <p>暂无其他成员</p>
              <p class="text-sm mt-2">点击右上角 + 手动添加设备</p>
            </div>
          </Show>

          {/* Loading State */}
          <Show when={isLoadingMembers() && members().length === 0}>
            <div class="text-center py-12 text-gray-500">
              <span class="i-lucide-loader-2 text-4xl mb-4 block opacity-50 animate-spin"></span>
              <p>正在搜索成员...</p>
            </div>
          </Show>
        </div>
      </div>

      {/* Footer */}
      <footer class="bg-white border-t border-gray-200 px-4 py-3">
        <div class="flex items-center justify-between">
          <div class="text-xs text-gray-500">
            <span class="inline-flex items-center gap-1">
              <span class="w-2 h-2 bg-green-500 rounded-full"></span>
              服务运行中
            </span>
          </div>
          <button
            class="px-4 py-2 text-red-600 hover:bg-red-50 text-sm font-medium rounded-lg"
            onClick={props.onStopService}
            disabled={props.isLoading}
          >
            {props.isLoading ? "关闭中..." : "关闭服务"}
          </button>
        </div>
      </footer>
    </div>
  );
};
