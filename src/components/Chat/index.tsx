import { Component, createSignal, For, onMount, onCleanup, createEffect } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { listen, UnlistenFn } from "@tauri-apps/api/event";

interface ChatMessage {
  id: string;
  from_device_id: string;
  from_name: string;
  content: string;
  timestamp: number;
  is_local: boolean;
  message_type: "Text" | "Code" | "System";
}

export const Chat: Component = () => {
  const [messages, setMessages] = createSignal<ChatMessage[]>([]);
  const [inputText, setInputText] = createSignal("");
  const [isLoading, setIsLoading] = createSignal(false);
  let messagesEndRef: HTMLDivElement | undefined;
  let unlistenMessage: UnlistenFn | undefined;

  // Format timestamp
  const formatTime = (timestamp: number) => {
    const date = new Date(timestamp);
    return date.toLocaleTimeString("zh-CN", {
      hour: "2-digit",
      minute: "2-digit",
    });
  };

  // Scroll to bottom when new messages arrive
  const scrollToBottom = () => {
    messagesEndRef?.scrollIntoView({ behavior: "smooth" });
  };

  // Fetch message history
  const fetchMessages = async () => {
    try {
      const result = await invoke<ChatMessage[]>("get_chat_messages");
      setMessages(result);
    } catch (e) {
      console.error("Failed to get messages:", e);
    }
  };

  // Send a message
  const sendMessage = async () => {
    const text = inputText().trim();
    if (!text || isLoading()) return;

    try {
      setIsLoading(true);
      const message = await invoke<ChatMessage>("send_chat_message", {
        content: text,
      });
      setMessages((prev) => [...prev, message]);
      setInputText("");
    } catch (e) {
      console.error("Failed to send message:", e);
    } finally {
      setIsLoading(false);
    }
  };

  // Handle incoming messages
  const handleNewMessage = (message: ChatMessage) => {
    setMessages((prev) => {
      // Avoid duplicates
      if (prev.some((m) => m.id === message.id)) {
        return prev;
      }
      return [...prev, message];
    });
  };

  onMount(async () => {
    // Listen for new chat messages
    unlistenMessage = await listen<ChatMessage>("chat-message", (event) => {
      handleNewMessage(event.payload);
    });

    // Fetch existing messages
    await fetchMessages();
  });

  onCleanup(() => {
    unlistenMessage?.();
  });

  // Auto-scroll when messages change
  createEffect(() => {
    messages();
    scrollToBottom();
  });

  return (
    <div class="max-w-4xl mx-auto h-full flex flex-col">
      {/* Chat Header */}
      <div class="card mb-4">
        <div class="flex items-center justify-between">
          <div class="flex items-center gap-3">
            <span class="i-lucide-message-square text-primary-500 text-xl"></span>
            <div>
              <h2 class="text-lg font-semibold text-gray-900">会议聊天</h2>
              <p class="text-sm text-gray-500">
                {messages().length > 0
                  ? `${messages().length} 条消息`
                  : "暂无消息"}
              </p>
            </div>
          </div>
          <button
            class="btn-secondary text-sm"
            onClick={fetchMessages}
            title="刷新消息"
          >
            <span class="i-lucide-refresh-cw"></span>
          </button>
        </div>
      </div>

      {/* Messages Container */}
      <div class="card flex-1 flex flex-col min-h-0">
        <div class="flex-1 overflow-y-auto space-y-3 p-2">
          <For each={messages()}>
            {(message) => (
              <div
                class={`flex ${message.is_local ? "justify-end" : "justify-start"}`}
              >
                <div
                  class={`max-w-[75%] rounded-2xl px-4 py-2 ${
                    message.message_type === "System"
                      ? "bg-gray-100 text-gray-600 text-sm text-center w-full max-w-full rounded-lg"
                      : message.is_local
                        ? "bg-primary-500 text-white rounded-br-md"
                        : "bg-gray-100 text-gray-900 rounded-bl-md"
                  }`}
                >
                  {/* Sender name for remote messages */}
                  {!message.is_local && message.message_type !== "System" && (
                    <div class="text-xs text-gray-500 mb-1">
                      {message.from_name}
                    </div>
                  )}

                  {/* Message content */}
                  {message.message_type === "Code" ? (
                    <pre class="font-mono text-sm bg-gray-800 text-green-400 p-3 rounded-lg overflow-x-auto">
                      <code>{message.content}</code>
                    </pre>
                  ) : (
                    <p class="whitespace-pre-wrap break-words">
                      {message.content}
                    </p>
                  )}

                  {/* Timestamp */}
                  <div
                    class={`text-xs mt-1 ${
                      message.message_type === "System"
                        ? "text-gray-400"
                        : message.is_local
                          ? "text-primary-200"
                          : "text-gray-400"
                    }`}
                  >
                    {formatTime(message.timestamp)}
                  </div>
                </div>
              </div>
            )}
          </For>

          {/* Empty state */}
          {messages().length === 0 && (
            <div class="text-center py-12 text-gray-500">
              <span class="i-lucide-message-circle text-4xl mb-4 block opacity-50"></span>
              <p>暂无消息</p>
              <p class="text-sm mt-2">发送第一条消息开始聊天</p>
            </div>
          )}

          {/* Scroll anchor */}
          <div ref={messagesEndRef}></div>
        </div>

        {/* Input Area */}
        <div class="border-t border-gray-200 pt-4 mt-4">
          <div class="flex gap-3">
            <input
              type="text"
              placeholder="输入消息..."
              value={inputText()}
              onInput={(e) => setInputText(e.currentTarget.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && !e.shiftKey) {
                  e.preventDefault();
                  sendMessage();
                }
              }}
              class="flex-1 px-4 py-3 border border-gray-300 rounded-xl focus:outline-none focus:ring-2 focus:ring-primary-500 focus:border-transparent"
              disabled={isLoading()}
            />
            <button
              class="btn-primary px-6"
              onClick={sendMessage}
              disabled={!inputText().trim() || isLoading()}
            >
              {isLoading() ? (
                <span class="i-lucide-loader-2 animate-spin"></span>
              ) : (
                <span class="i-lucide-send"></span>
              )}
            </button>
          </div>
          <p class="text-xs text-gray-400 mt-2">按 Enter 发送消息</p>
        </div>
      </div>
    </div>
  );
};
