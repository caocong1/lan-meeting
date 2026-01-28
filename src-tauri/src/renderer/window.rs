// Independent render window for screen sharing viewer
// Uses winit for window management and wgpu for rendering

use super::{wgpu_renderer::WgpuRenderer, FrameFormat, RenderFrame, RendererError};
use crossbeam_channel::{Receiver, Sender};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use winit::{
    application::ApplicationHandler,
    dpi::PhysicalSize,
    event::WindowEvent as WinitWindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    platform::scancode::PhysicalKeyExtScancode,
    window::{Window, WindowAttributes, WindowId},
};

/// Events from the render window
#[derive(Debug, Clone)]
pub enum WindowEvent {
    Resized(u32, u32),
    CloseRequested,
    Focused(bool),
    KeyPressed(u32),
    MouseMoved(f64, f64),
    MouseButton(u32, bool), // button, pressed
    MouseWheel(f64, f64),
}

/// Command to the render window
enum WindowCommand {
    RenderFrame(RenderFrame),
    SetTitle(String),
    Close,
}

/// Handle to control the render window from another thread
#[derive(Clone)]
pub struct RenderWindowHandle {
    command_tx: Sender<WindowCommand>,
    event_rx: Receiver<WindowEvent>,
    is_open: Arc<AtomicBool>,
}

impl RenderWindowHandle {
    /// Send a frame to be rendered
    pub fn render_frame(&self, frame: RenderFrame) -> Result<(), RendererError> {
        if !self.is_open.load(Ordering::Relaxed) {
            return Err(RendererError::WindowError("Window closed".to_string()));
        }
        self.command_tx
            .send(WindowCommand::RenderFrame(frame))
            .map_err(|_| RendererError::WindowError("Failed to send frame".to_string()))
    }

    /// Set window title
    pub fn set_title(&self, title: &str) -> Result<(), RendererError> {
        self.command_tx
            .send(WindowCommand::SetTitle(title.to_string()))
            .map_err(|_| RendererError::WindowError("Failed to send command".to_string()))
    }

    /// Close the window
    pub fn close(&self) {
        let _ = self.command_tx.send(WindowCommand::Close);
    }

    /// Check if window is still open
    pub fn is_open(&self) -> bool {
        self.is_open.load(Ordering::Relaxed)
    }

    /// Try to receive a window event (non-blocking)
    pub fn try_recv_event(&self) -> Option<WindowEvent> {
        self.event_rx.try_recv().ok()
    }

    /// Receive a window event (blocking)
    pub fn recv_event(&self) -> Option<WindowEvent> {
        self.event_rx.recv().ok()
    }
}

/// Render window state
pub struct RenderWindow {
    title: String,
    width: u32,
    height: u32,
    command_rx: Receiver<WindowCommand>,
    event_tx: Sender<WindowEvent>,
    is_open: Arc<AtomicBool>,
    window: Option<Arc<Window>>,
    renderer: Option<WgpuRenderer>,
    current_format: FrameFormat,
}

impl RenderWindow {
    /// Create a new render window and return a handle to control it
    pub fn create(
        title: &str,
        width: u32,
        height: u32,
    ) -> Result<RenderWindowHandle, RendererError> {
        let (command_tx, command_rx) = crossbeam_channel::unbounded();
        let (event_tx, event_rx) = crossbeam_channel::unbounded();
        let is_open = Arc::new(AtomicBool::new(true));
        let is_open_clone = is_open.clone();

        let title = title.to_string();
        let title_clone = title.clone();

        // Spawn window thread
        std::thread::spawn(move || {
            let event_loop = EventLoop::new().expect("Failed to create event loop");
            event_loop.set_control_flow(ControlFlow::Poll);

            let mut app = RenderWindow {
                title: title_clone,
                width,
                height,
                command_rx,
                event_tx,
                is_open: is_open_clone,
                window: None,
                renderer: None,
                current_format: FrameFormat::BGRA,
            };

            event_loop.run_app(&mut app).ok();
        });

        Ok(RenderWindowHandle {
            command_tx,
            event_rx,
            is_open,
        })
    }

    fn process_commands(&mut self) {
        while let Ok(cmd) = self.command_rx.try_recv() {
            match cmd {
                WindowCommand::RenderFrame(frame) => {
                    self.current_format = frame.format;
                    if let Some(ref mut renderer) = self.renderer {
                        if let Err(e) = renderer.upload_frame(&frame) {
                            log::error!("Failed to upload frame: {}", e);
                        }
                    }
                    if let Some(ref window) = self.window {
                        window.request_redraw();
                    }
                }
                WindowCommand::SetTitle(title) => {
                    if let Some(ref window) = self.window {
                        window.set_title(&title);
                    }
                }
                WindowCommand::Close => {
                    self.is_open.store(false, Ordering::Relaxed);
                }
            }
        }
    }
}

impl ApplicationHandler for RenderWindow {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let window_attrs = WindowAttributes::default()
            .with_title(&self.title)
            .with_inner_size(PhysicalSize::new(self.width, self.height));

        let window = Arc::new(
            event_loop
                .create_window(window_attrs)
                .expect("Failed to create window"),
        );

        // Initialize renderer
        let window_clone = window.clone();
        let renderer = pollster::block_on(async {
            WgpuRenderer::new_with_surface(window_clone).await
        });

        match renderer {
            Ok(r) => {
                self.renderer = Some(r);
                log::info!("Render window created: {}x{}", self.width, self.height);
            }
            Err(e) => {
                log::error!("Failed to create renderer: {}", e);
            }
        }

        self.window = Some(window);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WinitWindowEvent,
    ) {
        match event {
            WinitWindowEvent::CloseRequested => {
                self.is_open.store(false, Ordering::Relaxed);
                let _ = self.event_tx.send(WindowEvent::CloseRequested);
                event_loop.exit();
            }
            WinitWindowEvent::Resized(size) => {
                self.width = size.width;
                self.height = size.height;
                if let Some(ref mut renderer) = self.renderer {
                    renderer.resize(size.width, size.height);
                }
                let _ = self.event_tx.send(WindowEvent::Resized(size.width, size.height));
            }
            WinitWindowEvent::Focused(focused) => {
                let _ = self.event_tx.send(WindowEvent::Focused(focused));
            }
            WinitWindowEvent::KeyboardInput { event, .. } => {
                if event.state.is_pressed() {
                    let _ = self.event_tx.send(WindowEvent::KeyPressed(
                        event.physical_key.to_scancode().unwrap_or(0),
                    ));
                }
            }
            WinitWindowEvent::CursorMoved { position, .. } => {
                let _ = self.event_tx.send(WindowEvent::MouseMoved(position.x, position.y));
            }
            WinitWindowEvent::MouseInput { state, button, .. } => {
                let button_id = match button {
                    winit::event::MouseButton::Left => 0,
                    winit::event::MouseButton::Right => 1,
                    winit::event::MouseButton::Middle => 2,
                    winit::event::MouseButton::Back => 3,
                    winit::event::MouseButton::Forward => 4,
                    winit::event::MouseButton::Other(id) => id as u32,
                };
                let _ = self.event_tx.send(WindowEvent::MouseButton(
                    button_id,
                    state.is_pressed(),
                ));
            }
            WinitWindowEvent::MouseWheel { delta, .. } => {
                let (dx, dy) = match delta {
                    winit::event::MouseScrollDelta::LineDelta(x, y) => (x as f64, y as f64),
                    winit::event::MouseScrollDelta::PixelDelta(pos) => (pos.x, pos.y),
                };
                let _ = self.event_tx.send(WindowEvent::MouseWheel(dx, dy));
            }
            WinitWindowEvent::RedrawRequested => {
                // Process any pending commands
                self.process_commands();

                // Render
                if let Some(ref mut renderer) = self.renderer {
                    if let Err(e) = renderer.render(self.current_format) {
                        log::error!("Render failed: {}", e);
                    }
                }
            }
            _ => {}
        }

        // Check if we should close
        if !self.is_open.load(Ordering::Relaxed) {
            event_loop.exit();
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        // Process commands even when idle
        self.process_commands();
    }
}
