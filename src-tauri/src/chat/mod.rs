// Chat module
// Real-time text messaging between peers

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// Maximum messages to keep in history
const MAX_HISTORY_SIZE: usize = 1000;

/// Chat message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    /// Message ID
    pub id: String,
    /// Sender device ID
    pub from_device_id: String,
    /// Sender display name
    pub from_name: String,
    /// Message content
    pub content: String,
    /// Timestamp (Unix milliseconds)
    pub timestamp: u64,
    /// Whether this is a local message
    pub is_local: bool,
    /// Message type
    pub message_type: MessageType,
}

/// Message type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageType {
    /// Regular text message
    Text,
    /// Code snippet
    Code,
    /// System notification
    System,
}

impl ChatMessage {
    /// Create a new text message
    pub fn new(from_device_id: &str, from_name: &str, content: &str, is_local: bool) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            from_device_id: from_device_id.to_string(),
            from_name: from_name.to_string(),
            content: content.to_string(),
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
            is_local,
            message_type: MessageType::Text,
        }
    }

    /// Create a code message
    pub fn code(from_device_id: &str, from_name: &str, content: &str, is_local: bool) -> Self {
        let mut msg = Self::new(from_device_id, from_name, content, is_local);
        msg.message_type = MessageType::Code;
        msg
    }

    /// Create a system message
    pub fn system(content: &str) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            from_device_id: "system".to_string(),
            from_name: "System".to_string(),
            content: content.to_string(),
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
            is_local: true,
            message_type: MessageType::System,
        }
    }
}

/// Chat manager for handling message history and state
pub struct ChatManager {
    /// Message history
    messages: RwLock<VecDeque<ChatMessage>>,
    /// Callback for new messages
    on_message: RwLock<Option<Box<dyn Fn(&ChatMessage) + Send + Sync>>>,
}

impl Default for ChatManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ChatManager {
    /// Create a new chat manager
    pub fn new() -> Self {
        Self {
            messages: RwLock::new(VecDeque::with_capacity(MAX_HISTORY_SIZE)),
            on_message: RwLock::new(None),
        }
    }

    /// Add a message to history
    pub fn add_message(&self, message: ChatMessage) {
        // Notify callback
        if let Some(ref callback) = *self.on_message.read() {
            callback(&message);
        }

        // Add to history
        let mut messages = self.messages.write();
        if messages.len() >= MAX_HISTORY_SIZE {
            messages.pop_front();
        }
        messages.push_back(message);
    }

    /// Get all messages
    pub fn get_messages(&self) -> Vec<ChatMessage> {
        self.messages.read().iter().cloned().collect()
    }

    /// Get messages after a timestamp
    pub fn get_messages_after(&self, timestamp: u64) -> Vec<ChatMessage> {
        self.messages
            .read()
            .iter()
            .filter(|m| m.timestamp > timestamp)
            .cloned()
            .collect()
    }

    /// Clear message history
    pub fn clear(&self) {
        self.messages.write().clear();
    }

    /// Set callback for new messages
    pub fn set_on_message<F>(&self, callback: F)
    where
        F: Fn(&ChatMessage) + Send + Sync + 'static,
    {
        *self.on_message.write() = Some(Box::new(callback));
    }

    /// Get message count
    pub fn message_count(&self) -> usize {
        self.messages.read().len()
    }
}

/// Global chat manager instance
static CHAT_MANAGER: once_cell::sync::Lazy<Arc<ChatManager>> =
    once_cell::sync::Lazy::new(|| Arc::new(ChatManager::new()));

/// Get the global chat manager
pub fn get_chat_manager() -> Arc<ChatManager> {
    CHAT_MANAGER.clone()
}

/// Add a local message (sent by us)
pub fn send_message(content: &str, device_id: &str, device_name: &str) -> ChatMessage {
    let message = ChatMessage::new(device_id, device_name, content, true);
    get_chat_manager().add_message(message.clone());
    message
}

/// Add a remote message (received from peer)
pub fn receive_message(from_device_id: &str, from_name: &str, content: &str, timestamp: u64) {
    let mut message = ChatMessage::new(from_device_id, from_name, content, false);
    message.timestamp = timestamp;
    get_chat_manager().add_message(message);
}

/// Add a system notification
pub fn add_system_message(content: &str) {
    let message = ChatMessage::system(content);
    get_chat_manager().add_message(message);
}
