// Input control module
// Cross-platform keyboard and mouse simulation

mod controller;
mod events;

#[cfg(target_os = "macos")]
mod macos;

pub use controller::InputController;
pub use events::*;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum InputError {
    #[error("Input simulation failed: {0}")]
    SimulationError(String),
    #[error("Permission denied for input control")]
    PermissionDenied,
    #[error("Failed to initialize input controller: {0}")]
    InitError(String),
}

/// Check if input control permission is available
pub fn has_permission() -> bool {
    #[cfg(target_os = "macos")]
    {
        macos::has_accessibility_permission()
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

/// Request input control permission
pub fn request_permission() -> bool {
    #[cfg(target_os = "macos")]
    {
        macos::request_accessibility_permission()
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}
