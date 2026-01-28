// Input controller - cross-platform input simulation
// Uses enigo for keyboard/mouse simulation

use super::{InputError, InputEvent, Modifiers, MouseButton};
use enigo::{
    Axis, Button, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings,
};
use parking_lot::Mutex;

/// Input controller for remote control
pub struct InputController {
    enigo: Mutex<Enigo>,
    screen_width: u32,
    screen_height: u32,
}

impl InputController {
    /// Create a new input controller
    pub fn new(screen_width: u32, screen_height: u32) -> Result<Self, InputError> {
        let enigo = Enigo::new(&Settings::default())
            .map_err(|e| InputError::InitError(format!("Failed to create Enigo: {}", e)))?;

        Ok(Self {
            enigo: Mutex::new(enigo),
            screen_width,
            screen_height,
        })
    }

    /// Update screen dimensions (for coordinate mapping)
    pub fn set_screen_size(&mut self, width: u32, height: u32) {
        self.screen_width = width;
        self.screen_height = height;
    }

    /// Execute an input event
    pub fn execute(&self, event: &InputEvent) -> Result<(), InputError> {
        match event {
            InputEvent::MouseMove { x, y } => self.mouse_move(*x, *y),
            InputEvent::MouseDown { button, x, y } => {
                self.mouse_move(*x, *y)?;
                self.mouse_down(*button)
            }
            InputEvent::MouseUp { button, x, y } => {
                self.mouse_move(*x, *y)?;
                self.mouse_up(*button)
            }
            InputEvent::MouseScroll { delta_x, delta_y } => self.mouse_scroll(*delta_x, *delta_y),
            InputEvent::KeyDown { scancode, modifiers } => self.key_down(*scancode, *modifiers),
            InputEvent::KeyUp { scancode, modifiers } => self.key_up(*scancode, *modifiers),
            InputEvent::TextInput { text } => self.text_input(text),
        }
    }

    /// Move mouse to absolute position (0.0-1.0 relative coordinates)
    fn mouse_move(&self, x: f32, y: f32) -> Result<(), InputError> {
        let abs_x = (x * self.screen_width as f32) as i32;
        let abs_y = (y * self.screen_height as f32) as i32;

        let mut enigo = self.enigo.lock();
        enigo
            .move_mouse(abs_x, abs_y, Coordinate::Abs)
            .map_err(|e| InputError::SimulationError(format!("Mouse move failed: {}", e)))?;

        Ok(())
    }

    /// Press mouse button
    fn mouse_down(&self, button: MouseButton) -> Result<(), InputError> {
        let btn = match button {
            MouseButton::Left => Button::Left,
            MouseButton::Right => Button::Right,
            MouseButton::Middle => Button::Middle,
            MouseButton::Back => Button::Back,
            MouseButton::Forward => Button::Forward,
        };

        let mut enigo = self.enigo.lock();
        enigo
            .button(btn, Direction::Press)
            .map_err(|e| InputError::SimulationError(format!("Mouse down failed: {}", e)))?;

        Ok(())
    }

    /// Release mouse button
    fn mouse_up(&self, button: MouseButton) -> Result<(), InputError> {
        let btn = match button {
            MouseButton::Left => Button::Left,
            MouseButton::Right => Button::Right,
            MouseButton::Middle => Button::Middle,
            MouseButton::Back => Button::Back,
            MouseButton::Forward => Button::Forward,
        };

        let mut enigo = self.enigo.lock();
        enigo
            .button(btn, Direction::Release)
            .map_err(|e| InputError::SimulationError(format!("Mouse up failed: {}", e)))?;

        Ok(())
    }

    /// Mouse scroll
    fn mouse_scroll(&self, delta_x: f32, delta_y: f32) -> Result<(), InputError> {
        let mut enigo = self.enigo.lock();

        if delta_x.abs() > 0.01 {
            enigo
                .scroll(delta_x as i32, Axis::Horizontal)
                .map_err(|e| InputError::SimulationError(format!("Scroll X failed: {}", e)))?;
        }

        if delta_y.abs() > 0.01 {
            enigo
                .scroll(delta_y as i32, Axis::Vertical)
                .map_err(|e| InputError::SimulationError(format!("Scroll Y failed: {}", e)))?;
        }

        Ok(())
    }

    /// Press a key by scancode
    fn key_down(&self, scancode: u32, modifiers: Modifiers) -> Result<(), InputError> {
        let mut enigo = self.enigo.lock();

        // Press modifiers first
        self.press_modifiers(&mut enigo, modifiers, Direction::Press)?;

        // Press the key
        if let Some(key) = scancode_to_key(scancode) {
            enigo
                .key(key, Direction::Press)
                .map_err(|e| InputError::SimulationError(format!("Key down failed: {}", e)))?;
        }

        Ok(())
    }

    /// Release a key by scancode
    fn key_up(&self, scancode: u32, modifiers: Modifiers) -> Result<(), InputError> {
        let mut enigo = self.enigo.lock();

        // Release the key
        if let Some(key) = scancode_to_key(scancode) {
            enigo
                .key(key, Direction::Release)
                .map_err(|e| InputError::SimulationError(format!("Key up failed: {}", e)))?;
        }

        // Release modifiers
        self.press_modifiers(&mut enigo, modifiers, Direction::Release)?;

        Ok(())
    }

    /// Type text directly
    fn text_input(&self, text: &str) -> Result<(), InputError> {
        let mut enigo = self.enigo.lock();
        enigo
            .text(text)
            .map_err(|e| InputError::SimulationError(format!("Text input failed: {}", e)))?;

        Ok(())
    }

    /// Press or release modifier keys
    fn press_modifiers(
        &self,
        enigo: &mut Enigo,
        modifiers: Modifiers,
        direction: Direction,
    ) -> Result<(), InputError> {
        if modifiers.shift {
            enigo
                .key(Key::Shift, direction)
                .map_err(|e| InputError::SimulationError(format!("Shift failed: {}", e)))?;
        }
        if modifiers.ctrl {
            enigo
                .key(Key::Control, direction)
                .map_err(|e| InputError::SimulationError(format!("Ctrl failed: {}", e)))?;
        }
        if modifiers.alt {
            enigo
                .key(Key::Alt, direction)
                .map_err(|e| InputError::SimulationError(format!("Alt failed: {}", e)))?;
        }
        if modifiers.meta {
            enigo
                .key(Key::Meta, direction)
                .map_err(|e| InputError::SimulationError(format!("Meta failed: {}", e)))?;
        }

        Ok(())
    }
}

/// Convert scancode to enigo Key
/// Scancodes follow USB HID usage tables for cross-platform compatibility
fn scancode_to_key(scancode: u32) -> Option<Key> {
    // Common scancodes (based on USB HID)
    match scancode {
        // Letters (USB HID 0x04-0x1D)
        0x04 => Some(Key::Unicode('a')),
        0x05 => Some(Key::Unicode('b')),
        0x06 => Some(Key::Unicode('c')),
        0x07 => Some(Key::Unicode('d')),
        0x08 => Some(Key::Unicode('e')),
        0x09 => Some(Key::Unicode('f')),
        0x0A => Some(Key::Unicode('g')),
        0x0B => Some(Key::Unicode('h')),
        0x0C => Some(Key::Unicode('i')),
        0x0D => Some(Key::Unicode('j')),
        0x0E => Some(Key::Unicode('k')),
        0x0F => Some(Key::Unicode('l')),
        0x10 => Some(Key::Unicode('m')),
        0x11 => Some(Key::Unicode('n')),
        0x12 => Some(Key::Unicode('o')),
        0x13 => Some(Key::Unicode('p')),
        0x14 => Some(Key::Unicode('q')),
        0x15 => Some(Key::Unicode('r')),
        0x16 => Some(Key::Unicode('s')),
        0x17 => Some(Key::Unicode('t')),
        0x18 => Some(Key::Unicode('u')),
        0x19 => Some(Key::Unicode('v')),
        0x1A => Some(Key::Unicode('w')),
        0x1B => Some(Key::Unicode('x')),
        0x1C => Some(Key::Unicode('y')),
        0x1D => Some(Key::Unicode('z')),

        // Numbers (USB HID 0x1E-0x27)
        0x1E => Some(Key::Unicode('1')),
        0x1F => Some(Key::Unicode('2')),
        0x20 => Some(Key::Unicode('3')),
        0x21 => Some(Key::Unicode('4')),
        0x22 => Some(Key::Unicode('5')),
        0x23 => Some(Key::Unicode('6')),
        0x24 => Some(Key::Unicode('7')),
        0x25 => Some(Key::Unicode('8')),
        0x26 => Some(Key::Unicode('9')),
        0x27 => Some(Key::Unicode('0')),

        // Special keys
        0x28 => Some(Key::Return),     // Enter
        0x29 => Some(Key::Escape),     // Escape
        0x2A => Some(Key::Backspace),  // Backspace
        0x2B => Some(Key::Tab),        // Tab
        0x2C => Some(Key::Space),      // Space
        0x2D => Some(Key::Unicode('-')), // Minus
        0x2E => Some(Key::Unicode('=')), // Equals
        0x2F => Some(Key::Unicode('[')), // Left bracket
        0x30 => Some(Key::Unicode(']')), // Right bracket
        0x31 => Some(Key::Unicode('\\')), // Backslash
        0x33 => Some(Key::Unicode(';')), // Semicolon
        0x34 => Some(Key::Unicode('\'')), // Quote
        0x35 => Some(Key::Unicode('`')), // Grave
        0x36 => Some(Key::Unicode(',')), // Comma
        0x37 => Some(Key::Unicode('.')), // Period
        0x38 => Some(Key::Unicode('/')), // Slash

        // Function keys
        0x3A => Some(Key::F1),
        0x3B => Some(Key::F2),
        0x3C => Some(Key::F3),
        0x3D => Some(Key::F4),
        0x3E => Some(Key::F5),
        0x3F => Some(Key::F6),
        0x40 => Some(Key::F7),
        0x41 => Some(Key::F8),
        0x42 => Some(Key::F9),
        0x43 => Some(Key::F10),
        0x44 => Some(Key::F11),
        0x45 => Some(Key::F12),

        // Navigation keys
        0x49 => Some(Key::Other(0x49)), // Insert - use raw
        0x4A => Some(Key::Home),
        0x4B => Some(Key::PageUp),
        0x4C => Some(Key::Delete),
        0x4D => Some(Key::End),
        0x4E => Some(Key::PageDown),
        0x4F => Some(Key::RightArrow),
        0x50 => Some(Key::LeftArrow),
        0x51 => Some(Key::DownArrow),
        0x52 => Some(Key::UpArrow),

        // Modifiers (usually handled separately)
        0xE0 => Some(Key::Control), // Left Control
        0xE1 => Some(Key::Shift),   // Left Shift
        0xE2 => Some(Key::Alt),     // Left Alt
        0xE3 => Some(Key::Meta),    // Left Meta
        0xE4 => Some(Key::Control), // Right Control
        0xE5 => Some(Key::Shift),   // Right Shift
        0xE6 => Some(Key::Alt),     // Right Alt
        0xE7 => Some(Key::Meta),    // Right Meta

        _ => {
            log::trace!("Unknown scancode: 0x{:02X}", scancode);
            None
        }
    }
}
