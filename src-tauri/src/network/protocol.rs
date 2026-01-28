// Communication protocol
// Binary message format for efficient transmission

use super::NetworkError;
use bytes::{Buf, BufMut, BytesMut};
use serde::{Deserialize, Serialize};

/// Magic bytes for protocol identification
pub const MAGIC: [u8; 2] = [0x4C, 0x4D]; // "LM"
pub const VERSION: u8 = 1;

/// Maximum message size (16MB)
pub const MAX_MESSAGE_SIZE: usize = 16 * 1024 * 1024;

/// Header size: magic(2) + version(1) + type(1) + length(4)
pub const HEADER_SIZE: usize = 8;

/// Message type IDs for efficient encoding
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MessageType {
    // Connection management (0x00-0x0F)
    Handshake = 0x00,
    HandshakeAck = 0x01,
    Disconnect = 0x02,
    Heartbeat = 0x03,
    HeartbeatAck = 0x04,

    // Screen sharing (0x10-0x1F)
    ScreenOffer = 0x10,
    ScreenRequest = 0x11,
    ScreenStart = 0x12,
    ScreenFrame = 0x13,
    ScreenStop = 0x14,

    // Remote control (0x20-0x2F)
    ControlRequest = 0x20,
    ControlGrant = 0x21,
    ControlRevoke = 0x22,
    InputEvent = 0x23,

    // Chat (0x30-0x3F)
    ChatMessage = 0x30,

    // File transfer (0x40-0x4F)
    FileOffer = 0x40,
    FileAccept = 0x41,
    FileReject = 0x42,
    FileChunk = 0x43,
    FileComplete = 0x44,
    FileCancel = 0x45,
}

impl TryFrom<u8> for MessageType {
    type Error = NetworkError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x00 => Ok(Self::Handshake),
            0x01 => Ok(Self::HandshakeAck),
            0x02 => Ok(Self::Disconnect),
            0x03 => Ok(Self::Heartbeat),
            0x04 => Ok(Self::HeartbeatAck),
            0x10 => Ok(Self::ScreenOffer),
            0x11 => Ok(Self::ScreenRequest),
            0x12 => Ok(Self::ScreenStart),
            0x13 => Ok(Self::ScreenFrame),
            0x14 => Ok(Self::ScreenStop),
            0x20 => Ok(Self::ControlRequest),
            0x21 => Ok(Self::ControlGrant),
            0x22 => Ok(Self::ControlRevoke),
            0x23 => Ok(Self::InputEvent),
            0x30 => Ok(Self::ChatMessage),
            0x40 => Ok(Self::FileOffer),
            0x41 => Ok(Self::FileAccept),
            0x42 => Ok(Self::FileReject),
            0x43 => Ok(Self::FileChunk),
            0x44 => Ok(Self::FileComplete),
            0x45 => Ok(Self::FileCancel),
            _ => Err(NetworkError::ProtocolError(format!(
                "Unknown message type: 0x{:02X}",
                value
            ))),
        }
    }
}

/// Message types for the protocol
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Message {
    // Connection management
    Handshake {
        device_id: String,
        name: String,
        version: String,
        capabilities: Vec<String>,
    },
    HandshakeAck {
        device_id: String,
        name: String,
        version: String,
        accepted: bool,
        reason: Option<String>,
    },
    Disconnect {
        reason: String,
    },
    Heartbeat {
        timestamp: u64,
    },
    HeartbeatAck {
        timestamp: u64,
        latency_ms: u32,
    },

    // Screen sharing
    ScreenOffer {
        displays: Vec<DisplayInfo>,
    },
    ScreenRequest {
        display_id: u32,
        preferred_fps: u8,
        preferred_quality: u8,
    },
    ScreenStart {
        width: u32,
        height: u32,
        fps: u8,
        codec: String,
    },
    ScreenFrame {
        timestamp: u64,
        frame_type: FrameType,
        sequence: u32,
        data: Vec<u8>,
    },
    ScreenStop,

    // Remote control
    ControlRequest {
        from_user: String,
    },
    ControlGrant {
        to_user: String,
    },
    ControlRevoke,
    InputEvent {
        event_type: InputEventType,
        x: f32,
        y: f32,
        data: InputData,
    },

    // Chat
    ChatMessage {
        from: String,
        content: String,
        timestamp: u64,
    },

    // File transfer
    FileOffer {
        file_id: String,
        name: String,
        size: u64,
        checksum: String,
    },
    FileAccept {
        file_id: String,
    },
    FileReject {
        file_id: String,
    },
    FileChunk {
        file_id: String,
        offset: u64,
        data: Vec<u8>,
    },
    FileComplete {
        file_id: String,
    },
    FileCancel {
        file_id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisplayInfo {
    pub id: u32,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub primary: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum FrameType {
    KeyFrame,
    DeltaFrame,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum InputEventType {
    MouseMove,
    MouseDown,
    MouseUp,
    MouseScroll,
    KeyDown,
    KeyUp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InputData {
    Mouse {
        button: MouseButton,
    },
    Scroll {
        delta_x: f32,
        delta_y: f32,
    },
    Key {
        key_code: u32,
        modifiers: Modifiers,
    },
    None,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub meta: bool,
}

impl Message {
    /// Get the message type ID
    pub fn message_type(&self) -> MessageType {
        match self {
            Message::Handshake { .. } => MessageType::Handshake,
            Message::HandshakeAck { .. } => MessageType::HandshakeAck,
            Message::Disconnect { .. } => MessageType::Disconnect,
            Message::Heartbeat { .. } => MessageType::Heartbeat,
            Message::HeartbeatAck { .. } => MessageType::HeartbeatAck,
            Message::ScreenOffer { .. } => MessageType::ScreenOffer,
            Message::ScreenRequest { .. } => MessageType::ScreenRequest,
            Message::ScreenStart { .. } => MessageType::ScreenStart,
            Message::ScreenFrame { .. } => MessageType::ScreenFrame,
            Message::ScreenStop => MessageType::ScreenStop,
            Message::ControlRequest { .. } => MessageType::ControlRequest,
            Message::ControlGrant { .. } => MessageType::ControlGrant,
            Message::ControlRevoke => MessageType::ControlRevoke,
            Message::InputEvent { .. } => MessageType::InputEvent,
            Message::ChatMessage { .. } => MessageType::ChatMessage,
            Message::FileOffer { .. } => MessageType::FileOffer,
            Message::FileAccept { .. } => MessageType::FileAccept,
            Message::FileReject { .. } => MessageType::FileReject,
            Message::FileChunk { .. } => MessageType::FileChunk,
            Message::FileComplete { .. } => MessageType::FileComplete,
            Message::FileCancel { .. } => MessageType::FileCancel,
        }
    }
}

/// Encode a message to bytes
/// Format: MAGIC(2) + VERSION(1) + TYPE(1) + LENGTH(4) + PAYLOAD
pub fn encode(msg: &Message) -> Result<Vec<u8>, NetworkError> {
    let payload = bincode::serialize(msg)
        .map_err(|e| NetworkError::ProtocolError(format!("Serialization error: {}", e)))?;

    if payload.len() > MAX_MESSAGE_SIZE {
        return Err(NetworkError::ProtocolError(format!(
            "Message too large: {} bytes (max {})",
            payload.len(),
            MAX_MESSAGE_SIZE
        )));
    }

    let len = payload.len() as u32;
    let msg_type = msg.message_type() as u8;

    let mut buf = Vec::with_capacity(HEADER_SIZE + payload.len());
    buf.extend_from_slice(&MAGIC);
    buf.push(VERSION);
    buf.push(msg_type);
    buf.extend_from_slice(&len.to_be_bytes());
    buf.extend_from_slice(&payload);

    Ok(buf)
}

/// Decode bytes to a message
pub fn decode(data: &[u8]) -> Result<Message, NetworkError> {
    if data.len() < HEADER_SIZE {
        return Err(NetworkError::ProtocolError(format!(
            "Data too short: {} bytes (need at least {})",
            data.len(),
            HEADER_SIZE
        )));
    }

    // Verify magic
    if data[0..2] != MAGIC {
        return Err(NetworkError::ProtocolError(format!(
            "Invalid magic bytes: {:02X}{:02X}",
            data[0], data[1]
        )));
    }

    // Verify version
    if data[2] != VERSION {
        return Err(NetworkError::ProtocolError(format!(
            "Unsupported protocol version: {} (expected {})",
            data[2], VERSION
        )));
    }

    // Get message type (for validation)
    let _msg_type = MessageType::try_from(data[3])?;

    // Get payload length
    let len = u32::from_be_bytes([data[4], data[5], data[6], data[7]]) as usize;

    if len > MAX_MESSAGE_SIZE {
        return Err(NetworkError::ProtocolError(format!(
            "Message too large: {} bytes (max {})",
            len, MAX_MESSAGE_SIZE
        )));
    }

    if data.len() < HEADER_SIZE + len {
        return Err(NetworkError::ProtocolError(format!(
            "Incomplete message: have {} bytes, need {}",
            data.len(),
            HEADER_SIZE + len
        )));
    }

    bincode::deserialize(&data[HEADER_SIZE..HEADER_SIZE + len])
        .map_err(|e| NetworkError::ProtocolError(format!("Deserialization error: {}", e)))
}

/// Streaming message codec for handling partial reads
pub struct MessageCodec {
    buffer: BytesMut,
}

impl Default for MessageCodec {
    fn default() -> Self {
        Self::new()
    }
}

impl MessageCodec {
    pub fn new() -> Self {
        Self {
            buffer: BytesMut::with_capacity(64 * 1024), // 64KB initial buffer
        }
    }

    /// Feed data into the codec
    pub fn feed(&mut self, data: &[u8]) {
        self.buffer.put_slice(data);
    }

    /// Try to decode a complete message from the buffer
    pub fn decode(&mut self) -> Result<Option<Message>, NetworkError> {
        if self.buffer.len() < HEADER_SIZE {
            return Ok(None); // Need more data
        }

        // Verify magic
        if self.buffer[0..2] != MAGIC {
            // Invalid data, try to find next valid header
            if let Some(pos) = self.find_magic() {
                self.buffer.advance(pos);
            } else {
                self.buffer.clear();
            }
            return Err(NetworkError::ProtocolError("Invalid magic bytes".to_string()));
        }

        // Get payload length
        let len = u32::from_be_bytes([
            self.buffer[4],
            self.buffer[5],
            self.buffer[6],
            self.buffer[7],
        ]) as usize;

        if len > MAX_MESSAGE_SIZE {
            // Skip this message
            self.buffer.advance(HEADER_SIZE);
            return Err(NetworkError::ProtocolError(format!(
                "Message too large: {}",
                len
            )));
        }

        let total_len = HEADER_SIZE + len;
        if self.buffer.len() < total_len {
            return Ok(None); // Need more data
        }

        // Decode the message
        let msg_data = self.buffer.split_to(total_len);
        let msg = decode(&msg_data)?;

        Ok(Some(msg))
    }

    /// Find the next magic bytes in the buffer
    fn find_magic(&self) -> Option<usize> {
        self.buffer
            .windows(2)
            .position(|w| w == MAGIC)
    }

    /// Encode a message and return the bytes
    pub fn encode(&self, msg: &Message) -> Result<Vec<u8>, NetworkError> {
        encode(msg)
    }

    /// Clear the buffer
    pub fn clear(&mut self) {
        self.buffer.clear();
    }

    /// Get buffer length
    pub fn buffer_len(&self) -> usize {
        self.buffer.len()
    }
}

/// Create a handshake message
pub fn create_handshake(device_id: &str, name: &str) -> Message {
    Message::Handshake {
        device_id: device_id.to_string(),
        name: name.to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        capabilities: vec![
            "screen-share".to_string(),
            "remote-control".to_string(),
            "chat".to_string(),
            "file-transfer".to_string(),
        ],
    }
}

/// Create a handshake acknowledgment
pub fn create_handshake_ack(device_id: &str, name: &str, accepted: bool, reason: Option<String>) -> Message {
    Message::HandshakeAck {
        device_id: device_id.to_string(),
        name: name.to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        accepted,
        reason,
    }
}

/// Create a heartbeat message
pub fn create_heartbeat() -> Message {
    use std::time::{SystemTime, UNIX_EPOCH};

    Message::Heartbeat {
        timestamp: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0),
    }
}

/// Create a heartbeat acknowledgment
pub fn create_heartbeat_ack(original_timestamp: u64) -> Message {
    use std::time::{SystemTime, UNIX_EPOCH};

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    Message::HeartbeatAck {
        timestamp: now,
        latency_ms: (now.saturating_sub(original_timestamp)) as u32,
    }
}
