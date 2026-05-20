import { Component, createSignal, For, onMount, onCleanup, Show } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { listen, UnlistenFn } from "@tauri-apps/api/event";
import { SelfInfo } from "../../App";
import { AddDeviceModal } from "../AddDeviceModal";

interface DeviceInfo {
  id: string;
  name: string;
  ip: string;
  port: number;
  status?: "online" | "busy" | "offline";
  is_sharing: boolean;
}

interface Member {
  id: string;
  name: string;
  ip: string;
  port: number;
  status?: "online" | "busy" | "offline";
  is_self: boolean;
  is_sharing: boolean;
}

interface ViewerEvent {
  peer_ip: string;
  error?: string | null;
}

interface ViewerConnectedEvent {
  peer_ip: string;
}

interface ControlRequestEvent {
  from_user: string;
  peer_ip: string;
}

interface ConnectionReceivedEvent {
  device_name: string;
  ip: string;
}

interface MeetingRoomProps {
  selfInfo: SelfInfo;
  onStopService: () => void;
  onOpenSettings: () => void;
  isLoading: boolean;
}

const WATCH_TIMEOUT_MS = 12_000;

export const MeetingRoom: Component<MeetingRoomProps> = (props) => {
  const [members, setMembers] = createSignal<Member[]>([]);
  const [isSharing, setIsSharing] = createSignal(false);
  const [isLoadingMembers, setIsLoadingMembers] = createSignal(true);
  const [showAddModal, setShowAddModal] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [notice, setNotice] = createSignal<string | null>(null);
  const [connectingIds, setConnectingIds] = createSignal<Set<string>>(new Set());
  const [watchingIds, setWatchingIds] = createSignal<Set<string>>(new Set());
  const [controlRequestingIds, setControlRequestingIds] = createSignal<Set<string>>(new Set());
  const [viewerIps, setViewerIps] = createSignal<Set<string>>(new Set());

  let unlistenDiscovered: UnlistenFn | undefined;
  let unlistenRemoved: UnlistenFn | undefined;
  let unlistenSharingChanged: UnlistenFn | undefined;
  let unlistenViewerStarted: UnlistenFn | undefined;
  let unlistenViewerFailed: UnlistenFn | undefined;
  let unlistenViewerConnected: UnlistenFn | undefined;
  let unlistenControlRequest: UnlistenFn | undefined;
  let unlistenConnectionReceived: UnlistenFn | undefined;
  const watchTimeouts = new Map<string, number>();

  const fetchMembers = async () => {
    try {
      setIsLoadingMembers(true);
      const devices = await invoke<DeviceInfo[]>("get_devices");

      const memberList: Member[] = devices.map(d => ({
        id: d.id,
        name: d.name,
        ip: d.ip,
        port: d.port,
        status: d.status,
        is_self: false,
        is_sharing: d.is_sharing || false,
      }));

      if (props.selfInfo) {
        memberList.unshift({
          id: props.selfInfo.id,
          name: props.selfInfo.name + " (me)",
          ip: props.selfInfo.ip,
          port: 19876,
          status: "busy",
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

  const handleMemberDiscovered = (device: DeviceInfo) => {
    setMembers(prev => {
      const existing = prev.find(m => m.id === device.id);
      if (existing) {
        return prev.map(m => m.id === device.id ? {
          ...m,
          name: device.name,
          ip: device.ip,
          status: device.status,
          is_sharing: device.is_sharing || false,
        } : m);
      }
      return [...prev, {
        id: device.id,
        name: device.name,
        ip: device.ip,
        port: device.port,
        status: device.status,
        is_self: false,
        is_sharing: device.is_sharing || false,
      }];
    });
  };

  const handleMemberRemoved = (deviceId: string) => {
    const removed = members().find(m => m.id === deviceId);
    clearWatchTimeout(deviceId);
    setConnectingIds(prev => {
      const next = new Set(prev);
      next.delete(deviceId);
      return next;
    });
    setWatchingIds(prev => {
      const next = new Set(prev);
      next.delete(deviceId);
      return next;
    });
    setControlRequestingIds(prev => {
      const next = new Set(prev);
      next.delete(deviceId);
      return next;
    });
    if (removed) {
      setViewerIps(prev => {
        const next = new Set(prev);
        next.delete(removed.ip);
        return next;
      });
    }
    setMembers(prev => prev.filter(m => m.id !== deviceId));
  };

  const handleStartSharing = async () => {
    try {
      const hasPermission = await invoke<boolean>("check_screen_permission");
      if (!hasPermission) {
        await invoke("request_screen_permission");
        return;
      }

      const displays = await invoke<{ id: number; name: string }[]>("get_displays");
      if (displays.length === 0) {
        setError("未找到可共享的显示器");
        return;
      }

      const displayId = displays[0]?.id;
      if (displayId === undefined) {
        setError("无法获取显示器ID");
        return;
      }

      await invoke("broadcast_sharing_status", { isSharing: true, displayId });
      setIsSharing(true);

      setMembers(prev => prev.map(m =>
        m.is_self ? { ...m, is_sharing: true } : m
      ));
    } catch (e) {
      console.error("Failed to start sharing:", e);
      setError(`启动共享失败: ${e}`);
    }
  };

  const handleStopSharing = async () => {
    try {
      await invoke("broadcast_sharing_status", { isSharing: false, displayId: null });
      setIsSharing(false);
      setViewerIps(new Set<string>());

      setMembers(prev => prev.map(m =>
        m.is_self ? { ...m, is_sharing: false } : m
      ));
    } catch (e) {
      console.error("Failed to stop sharing:", e);
      setError(`停止共享失败: ${e}`);
    }
  };

  const clearWatchTimeout = (memberId: string) => {
    const timeoutId = watchTimeouts.get(memberId);
    if (timeoutId !== undefined) {
      window.clearTimeout(timeoutId);
      watchTimeouts.delete(memberId);
    }
  };

  const clearWatchingState = (memberId: string) => {
    clearWatchTimeout(memberId);
    setWatchingIds(prev => {
      const next = new Set(prev);
      next.delete(memberId);
      return next;
    });
  };

  const clearWatchingStateByIp = (peerIp: string) => {
    const member = members().find(m => m.ip === peerIp);
    if (member) {
      clearWatchingState(member.id);
    }
  };

  const operationLabel = (member: Member) => {
    if (connectingIds().has(member.id)) return "正在建立连接...";
    if (watchingIds().has(member.id)) return "正在打开观看窗口...";
    if (controlRequestingIds().has(member.id)) return "正在发送控制请求...";
    return null;
  };

  const handleWatchScreen = async (member: Member) => {
    if (watchingIds().has(member.id)) {
      return;
    }

    setWatchingIds(prev => new Set(prev).add(member.id));
    setError(null);
    setNotice(null);

    const timeoutId = window.setTimeout(() => {
      if (!watchingIds().has(member.id)) {
        return;
      }
      clearWatchingState(member.id);
      setError(`观看请求已发送，但 ${member.name} 还没有返回屏幕流，请确认对方仍在共享并保持应用运行`);
    }, WATCH_TIMEOUT_MS);
    watchTimeouts.set(member.id, timeoutId);

    try {
      await invoke("request_screen_stream", {
        peerIp: member.ip,
        peerName: member.name,
      });
    } catch (e) {
      console.error("Failed to request screen stream:", e);
      clearWatchingState(member.id);
      setError(`请求屏幕流失败: ${e}`);
    }
  };

  const handleConnectDevice = async (member: Member) => {
    if (connectingIds().has(member.id)) {
      return;
    }

    setConnectingIds(prev => new Set(prev).add(member.id));
    setError(null);
    setNotice(null);

    try {
      await invoke("connect_to_device", { deviceId: member.id });
      await fetchMembers();
    } catch (e) {
      console.error("Failed to connect to device:", e);
      setError(`连接失败: ${e}`);
    } finally {
      setConnectingIds(prev => {
        const next = new Set(prev);
        next.delete(member.id);
        return next;
      });
    }
  };

  const handleRequestControl = async (member: Member) => {
    if (controlRequestingIds().has(member.id)) {
      return;
    }

    setControlRequestingIds(prev => new Set(prev).add(member.id));
    setError(null);
    setNotice(null);

    try {
      await invoke("request_control", { peerId: member.id });
      setError("控制请求已发送，但控制授权和远程输入功能尚未实现，当前只能观看屏幕");
    } catch (e) {
      console.error("Failed to request control:", e);
      setError(`请求控制失败: ${e}`);
    } finally {
      setControlRequestingIds(prev => {
        const next = new Set(prev);
        next.delete(member.id);
        return next;
      });
    }
  };

  const isConnected = (member: Member) => member.status === "busy";

  onMount(async () => {
    unlistenDiscovered = await listen<DeviceInfo>("device-discovered", (event) => {
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

    unlistenViewerStarted = await listen<ViewerEvent>("viewer-started", (event) => {
      clearWatchingStateByIp(event.payload.peer_ip);
      const member = members().find(m => m.ip === event.payload.peer_ip);
      if (member) {
        setNotice(`正在观看 ${member.name} 的屏幕`);
      }
    });

    unlistenViewerFailed = await listen<ViewerEvent>("viewer-failed", (event) => {
      clearWatchingStateByIp(event.payload.peer_ip);
      const member = members().find(m => m.ip === event.payload.peer_ip);
      const name = member?.name ?? event.payload.peer_ip;
      setError(`打开 ${name} 的屏幕失败: ${event.payload.error ?? "未知错误"}`);
    });

    unlistenViewerConnected = await listen<ViewerConnectedEvent>("viewer-connected", (event) => {
      setViewerIps(prev => new Set(prev).add(event.payload.peer_ip));
      const member = members().find(m => m.ip === event.payload.peer_ip);
      setNotice(`${member?.name ?? event.payload.peer_ip} 正在观看你的屏幕`);
    });

    unlistenControlRequest = await listen<ControlRequestEvent>("control-request-received", (event) => {
      setNotice(`${event.payload.from_user} 请求控制你的屏幕；控制授权功能尚未实现`);
    });

    unlistenConnectionReceived = await listen<ConnectionReceivedEvent>("connection-received", (event) => {
      setNotice(`${event.payload.device_name} 已连接`);
    });

    await fetchMembers();
  });

  onCleanup(() => {
    unlistenDiscovered?.();
    unlistenRemoved?.();
    unlistenSharingChanged?.();
    unlistenViewerStarted?.();
    unlistenViewerFailed?.();
    unlistenViewerConnected?.();
    unlistenControlRequest?.();
    unlistenConnectionReceived?.();
    watchTimeouts.forEach(timeoutId => window.clearTimeout(timeoutId));
    watchTimeouts.clear();
  });

  return (
    <div class="h-full flex flex-col">
      <Show when={showAddModal()}>
        <AddDeviceModal
          onClose={() => setShowAddModal(false)}
          onAdded={() => {
            setShowAddModal(false);
            fetchMembers();
          }}
        />
      </Show>

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

            <button
              class="p-2 text-gray-500 hover:text-gray-700 hover:bg-gray-100 rounded-lg"
              onClick={props.onOpenSettings}
            >
              <span class="i-lucide-settings text-xl"></span>
            </button>
          </div>
        </div>
      </header>

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

      {notice() && (
        <div class="mx-4 mt-4 bg-blue-50 border border-blue-200 text-blue-700 px-4 py-3 rounded-lg flex items-center justify-between">
          <span class="text-sm">{notice()}</span>
          <button
            class="text-blue-500 hover:text-blue-700"
            onClick={() => setNotice(null)}
          >
            <span class="i-lucide-x"></span>
          </button>
        </div>
      )}

      <div class="px-4 py-3 flex items-center justify-between border-b border-gray-100">
        <div class="flex items-center gap-2">
          <span class="text-sm font-medium text-gray-700">会议成员</span>
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

      <div class="flex-1 overflow-auto p-4">
        <div class="space-y-2">
          <For each={members()}>
            {(member) => (
              <div class={`p-4 bg-white rounded-xl border ${member.is_self ? 'border-primary-200 bg-primary-50/30' : 'border-gray-200'} hover:shadow-sm transition-shadow`}>
                <div class="flex items-center justify-between">
                  <div class="flex items-center gap-3">
                    <div class={`w-10 h-10 rounded-full flex items-center justify-center ${member.is_self ? 'bg-primary-100' : 'bg-gray-100'}`}>
                      <span class={`i-lucide-user text-lg ${member.is_self ? 'text-primary-600' : 'text-gray-500'}`}></span>
                    </div>
                    <div>
                      <div class="flex items-center gap-2">
                        <span class="font-medium text-gray-900">{member.name}</span>
                        {member.is_self && (
                          <span class="px-1.5 py-0.5 bg-primary-100 text-primary-700 text-xs rounded">我</span>
                        )}
                      </div>
                      <span class="text-sm text-gray-500">{member.ip}</span>
                      <Show when={operationLabel(member)}>
                        <span class="mt-1 flex items-center gap-1 text-xs text-primary-600">
                          <span class="i-lucide-loader-2 animate-spin"></span>
                          {operationLabel(member)}
                        </span>
                      </Show>
                    </div>
                  </div>

                  <div class="flex items-center gap-2">
                    {member.is_sharing ? (
                      <>
                        <span class="flex items-center gap-1.5 px-2 py-1 bg-red-100 text-red-700 text-xs rounded-full">
                          <span class="w-1.5 h-1.5 bg-red-500 rounded-full animate-pulse"></span>
                          正在共享
                        </span>
                        {member.is_self && viewerIps().size > 0 && (
                          <span class="px-2 py-1 bg-blue-100 text-blue-700 text-xs rounded-full">
                            {viewerIps().size} 人观看
                          </span>
                        )}
                        {!member.is_self && (
                          isConnected(member) ? (
                            <>
                              <button
                                class="px-3 py-1.5 bg-primary-500 hover:bg-primary-600 text-white text-sm rounded-lg disabled:opacity-60 disabled:cursor-not-allowed flex items-center gap-1.5"
                                onClick={() => handleWatchScreen(member)}
                                disabled={watchingIds().has(member.id) || controlRequestingIds().has(member.id)}
                              >
                                {watchingIds().has(member.id) && (
                                  <span class="i-lucide-loader-2 animate-spin"></span>
                                )}
                                {watchingIds().has(member.id) ? "打开中..." : "观看"}
                              </button>
                              <button
                                class="px-3 py-1.5 border border-gray-300 hover:bg-gray-50 text-gray-700 text-sm rounded-lg disabled:opacity-60 disabled:cursor-not-allowed flex items-center gap-1.5"
                                onClick={() => handleRequestControl(member)}
                                disabled={watchingIds().has(member.id) || controlRequestingIds().has(member.id)}
                              >
                                {controlRequestingIds().has(member.id) && (
                                  <span class="i-lucide-loader-2 animate-spin"></span>
                                )}
                                {controlRequestingIds().has(member.id) ? "请求中..." : "请求控制"}
                              </button>
                            </>
                          ) : (
                            <button
                              class="px-3 py-1.5 border border-gray-300 hover:bg-gray-50 text-gray-700 text-sm rounded-lg disabled:opacity-60 disabled:cursor-not-allowed flex items-center gap-1.5"
                              onClick={() => handleConnectDevice(member)}
                              disabled={connectingIds().has(member.id)}
                            >
                              {connectingIds().has(member.id) && (
                                <span class="i-lucide-loader-2 animate-spin"></span>
                              )}
                              {connectingIds().has(member.id) ? "连接中..." : "连接"}
                            </button>
                          )
                        )}
                      </>
                    ) : (
                      <div class="flex items-center gap-2">
                        <span class="text-sm text-gray-400">未共享</span>
                        {!member.is_self && isConnected(member) && (
                          <span class="px-2 py-1 bg-green-100 text-green-700 text-xs rounded-full">
                            已连接
                          </span>
                        )}
                      </div>
                    )}
                  </div>
                </div>
              </div>
            )}
          </For>

          <Show when={!isLoadingMembers() && members().length === 0}>
            <div class="text-center py-12 text-gray-500">
              <span class="i-lucide-users text-4xl mb-4 block opacity-50"></span>
              <p>暂无其他成员</p>
              <p class="text-sm mt-2">点击右上角 + 手动添加设备</p>
            </div>
          </Show>

          <Show when={isLoadingMembers() && members().length === 0}>
            <div class="text-center py-12 text-gray-500">
              <span class="i-lucide-loader-2 text-4xl mb-4 block opacity-50 animate-spin"></span>
              <p>正在搜索成员...</p>
            </div>
          </Show>
        </div>
      </div>

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
