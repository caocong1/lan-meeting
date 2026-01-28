// Input event types for network transmission
// Serializable events that can be sent between peers

use serde::{Deserialize, Serialize};

/// Mouse button types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Back,
    Forward,
}

impl From<u32> for MouseButton {
    fn from(value: u32) -> Self {
        match value {
            0 => MouseButton::Left,
            1 => MouseButton::Right,
            2 => MouseButton::Middle,
            3 => MouseButton::Back,
            4 => MouseButton::Forward,
            _ => MouseButton::Left,
        }
    }
}

/// Keyboard modifiers
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub meta: bool, // Cmd on macOS, Win on Windows
}

impl Modifiers {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_shift(mut self) -> Self {
        self.shift = true;
        self
    }

    pub fn with_ctrl(mut self) -> Self {
        self.ctrl = true;
        self
    }

    pub fn with_alt(mut self) -> Self {
        self.alt = true;
        self
    }

    pub fn with_meta(mut self) -> Self {
        self.meta = true;
        self
    }
}

/// Input event that can be transmitted over network
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InputEvent {
    /// Mouse moved to position (relative 0.0-1.0)
    MouseMove {
        x: f32,
        y: f32,
    },

    /// Mouse button pressed
    MouseDown {
        button: MouseButton,
        x: f32,
        y: f32,
    },

    /// Mouse button released
    MouseUp {
        button: MouseButton,
        x: f32,
        y: f32,
    },

    /// Mouse wheel scrolled
    MouseScroll {
        delta_x: f32,
        delta_y: f32,
    },

    /// Key pressed (scancode for cross-platform compatibility)
    KeyDown {
        scancode: u32,
        modifiers: Modifiers,
    },

    /// Key released
    KeyUp {
        scancode: u32,
        modifiers: Modifiers,
    },

    /// Text input (for complex input methods)
    TextInput {
        text: String,
    },
}

impl InputEvent {
    /// Create mouse move event
    pub fn mouse_move(x: f32, y: f32) -> Self {
        Self::MouseMove { x, y }
    }

    /// Create mouse down event
    pub fn mouse_down(button: MouseButton, x: f32, y: f32) -> Self {
        Self::MouseDown { button, x, y }
    }

    /// Create mouse up event
    pub fn mouse_up(button: MouseButton, x: f32, y: f32) -> Self {
        Self::MouseUp { button, x, y }
    }

    /// Create mouse scroll event
    pub fn mouse_scroll(delta_x: f32, delta_y: f32) -> Self {
        Self::MouseScroll { delta_x, delta_y }
    }

    /// Create key down event
    pub fn key_down(scancode: u32, modifiers: Modifiers) -> Self {
        Self::KeyDown { scancode, modifiers }
    }

    /// Create key up event
    pub fn key_up(scancode: u32, modifiers: Modifiers) -> Self {
        Self::KeyUp { scancode, modifiers }
    }

    /// Create text input event
    pub fn text_input(text: impl Into<String>) -> Self {
        Self::TextInput { text: text.into() }
    }
}

/// Control permission state
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ControlState {
    /// No control permission
    None,
    /// Control requested, waiting for approval
    Requested,
    /// Control granted
    Granted,
    /// Control denied
    Denied,
}

/// Control request/response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlRequest {
    pub from_device_id: String,
    pub from_device_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlResponse {
    pub granted: bool,
    pub reason: Option<String>,
}
