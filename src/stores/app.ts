import { createSignal, createRoot } from "solid-js";

export interface Device {
  id: string;
  name: string;
  ip: string;
  port: number;
  status: "online" | "busy" | "offline";
  last_seen: number;
}

export interface SelfInfo {
  id: string;
  name: string;
}

export interface ConnectionState {
  isConnected: boolean;
  connectedDevices: string[];
  activeSession: string | null;
}

// Create store in a root context to persist across components
function createAppStore() {
  // Self info
  const [selfInfo, setSelfInfo] = createSignal<SelfInfo | null>(null);

  // Connection state
  const [connectionState, setConnectionState] = createSignal<ConnectionState>({
    isConnected: false,
    connectedDevices: [],
    activeSession: null,
  });

  // Devices
  const [devices, setDevices] = createSignal<Device[]>([]);

  // Screen sharing state
  const [isSharing, setIsSharing] = createSignal(false);
  const [sharingDisplayId, setSharingDisplayId] = createSignal<number | null>(null);

  // Helper functions
  const addConnectedDevice = (deviceId: string) => {
    setConnectionState((prev) => ({
      ...prev,
      connectedDevices: [...new Set([...prev.connectedDevices, deviceId])],
    }));
  };

  const removeConnectedDevice = (deviceId: string) => {
    setConnectionState((prev) => ({
      ...prev,
      connectedDevices: prev.connectedDevices.filter((id) => id !== deviceId),
    }));
  };

  const updateDevice = (device: Device) => {
    setDevices((prev) => {
      const existing = prev.find((d) => d.id === device.id);
      if (existing) {
        return prev.map((d) => (d.id === device.id ? device : d));
      }
      return [...prev, device];
    });
  };

  const removeDevice = (deviceId: string) => {
    setDevices((prev) => prev.filter((d) => d.id !== deviceId));
  };

  return {
    // Self info
    selfInfo,
    setSelfInfo,

    // Connection
    connectionState,
    setConnectionState,
    addConnectedDevice,
    removeConnectedDevice,

    // Devices
    devices,
    setDevices,
    updateDevice,
    removeDevice,

    // Screen sharing
    isSharing,
    setIsSharing,
    sharingDisplayId,
    setSharingDisplayId,
  };
}

// Export singleton store
export const appStore = createRoot(createAppStore);
