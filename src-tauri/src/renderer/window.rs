// Independent render window for screen sharing viewer
// Uses winit for window management on Windows/Linux,
// and native AppKit window on macOS (winit requires main thread on macOS)

use super::{wgpu_renderer::WgpuRenderer, FrameFormat, RenderFrame, RendererError};
use crossbeam_channel::{Receiver, Sender};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[cfg(not(target_os = "macos"))]
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

/// Render window state (used by winit on non-macOS platforms)
#[cfg(not(target_os = "macos"))]
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

/// Render window (macOS uses native AppKit window)
#[cfg(target_os = "macos")]
pub struct RenderWindow;

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

        #[cfg(target_os = "macos")]
        Self::create_macos(title, width, height, command_rx, event_tx, is_open_clone)?;

        #[cfg(not(target_os = "macos"))]
        Self::create_winit(title, width, height, command_rx, event_tx, is_open_clone);

        Ok(RenderWindowHandle {
            command_tx,
            event_rx,
            is_open,
        })
    }

    /// Windows/Linux: Use winit EventLoop for window management
    #[cfg(not(target_os = "macos"))]
    fn create_winit(
        title: String,
        width: u32,
        height: u32,
        command_rx: Receiver<WindowCommand>,
        event_tx: Sender<WindowEvent>,
        is_open: Arc<AtomicBool>,
    ) {
        let title_clone = title.clone();
        std::thread::spawn(move || {
            log::debug!("Render window thread started for '{}'", title_clone);

            let event_loop = EventLoop::new().expect("Failed to create event loop");
            event_loop.set_control_flow(ControlFlow::Poll);
            log::debug!("EventLoop created successfully");

            let mut app = RenderWindow {
                title: title_clone,
                width,
                height,
                command_rx,
                event_tx,
                is_open,
                window: None,
                renderer: None,
                current_format: FrameFormat::BGRA,
            };

            event_loop.run_app(&mut app).ok();
        });
    }

    /// macOS: Create native AppKit window on main thread, render with wgpu on background thread.
    /// winit requires EventLoop on macOS main thread, which is occupied by Tauri,
    /// so we bypass winit and use objc2 for native window creation.
    #[cfg(target_os = "macos")]
    fn create_macos(
        title: String,
        width: u32,
        height: u32,
        command_rx: Receiver<WindowCommand>,
        event_tx: Sender<WindowEvent>,
        is_open: Arc<AtomicBool>,
    ) -> Result<(), RendererError> {
        log::debug!(
            "Creating macOS native render window: '{}' ({}x{})",
            title,
            width,
            height
        );

        // Create NSWindow on the main thread via Tauri's dispatch mechanism
        let app_handle = crate::APP_HANDLE
            .get()
            .ok_or_else(|| RendererError::WindowError("Tauri not initialized".to_string()))?;

        // Channel to receive the NSView pointer from the main thread
        let (result_tx, result_rx) =
            std::sync::mpsc::channel::<Result<(SendPtr, SendPtr), String>>();

        let title_for_main = title.clone();
        app_handle
            .run_on_main_thread(move || {
                let result = create_ns_window(&title_for_main, width, height);
                let _ = result_tx.send(result);
            })
            .map_err(|e| {
                RendererError::WindowError(format!("Failed to dispatch to main thread: {}", e))
            })?;

        // Wait for main thread to create the window
        let (ns_view, _ns_window) = result_rx
            .recv()
            .map_err(|e| {
                RendererError::WindowError(format!("Main thread channel closed: {}", e))
            })?
            .map_err(|e| RendererError::WindowError(format!("NSWindow creation failed: {}", e)))?;

        log::debug!("NSWindow created on main thread, starting render thread");

        // Convert pointers to usize for Send safety before spawning thread
        // (Rust 2021 closures capture individual fields; NonNull<c_void> is !Send)
        let ns_view_addr = ns_view.0.as_ptr() as usize;
        let ns_window_addr = _ns_window.0.as_ptr() as usize;

        // Create wgpu Instance + Surface on main thread
        // (Metal's get_metal_layer MUST be called on the UI thread)
        let (surface_tx, surface_rx) =
            std::sync::mpsc::channel::<Result<(wgpu::Instance, wgpu::Surface<'static>), String>>();

        app_handle
            .run_on_main_thread(move || {
                let result =
                    (|| -> Result<(wgpu::Instance, wgpu::Surface<'static>), String> {
                        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
                            backends: wgpu::Backends::METAL,
                            ..Default::default()
                        });

                        let ns_view_ptr =
                            std::ptr::NonNull::new(ns_view_addr as *mut std::ffi::c_void)
                                .ok_or_else(|| "NSView pointer was null".to_string())?;

                        let surface = unsafe {
                            let raw_display = raw_window_handle::RawDisplayHandle::AppKit(
                                raw_window_handle::AppKitDisplayHandle::new(),
                            );
                            let raw_window = raw_window_handle::RawWindowHandle::AppKit(
                                raw_window_handle::AppKitWindowHandle::new(ns_view_ptr),
                            );
                            instance
                                .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                                    raw_display_handle: raw_display,
                                    raw_window_handle: raw_window,
                                })
                                .map_err(|e| format!("Failed to create surface: {}", e))?
                        };

                        Ok((instance, surface))
                    })();
                let _ = surface_tx.send(result);
            })
            .map_err(|e| {
                RendererError::WindowError(format!(
                    "Failed to dispatch surface creation: {}",
                    e
                ))
            })?;

        let (instance, surface) = surface_rx
            .recv()
            .map_err(|e| {
                RendererError::WindowError(format!("Surface channel closed: {}", e))
            })?
            .map_err(|e| RendererError::WindowError(format!("Surface creation failed: {}", e)))?;

        log::debug!("wgpu Surface created on main thread");

        // Spawn render thread with pre-created instance + surface
        std::thread::spawn(move || {
            log::debug!("macOS render thread started");

            // Initialize wgpu renderer with instance + surface created on main thread
            log::info!("macOS render thread: initializing wgpu renderer...");
            let renderer = pollster::block_on(async {
                WgpuRenderer::new_with_raw_surface(instance, surface, width, height).await
            });

            let mut renderer = match renderer {
                Ok(r) => {
                    log::info!("macOS render thread: renderer READY ({}x{})", width, height);
                    r
                }
                Err(e) => {
                    log::error!("macOS render thread: FAILED to create renderer: {}", e);
                    is_open.store(false, Ordering::Relaxed);
                    return;
                }
            };

            let mut current_format = FrameFormat::BGRA;
            let mut check_counter: u32 = 0;
            let mut render_frame_count: u32 = 0;

            // Simple render loop (no winit event loop needed)
            loop {
                if !is_open.load(Ordering::Relaxed) {
                    break;
                }

                let mut has_new_frame = false;

                // Process all pending commands
                while let Ok(cmd) = command_rx.try_recv() {
                    match cmd {
                        WindowCommand::RenderFrame(frame) => {
                            current_format = frame.format;
                            if let Err(e) = renderer.upload_frame(&frame) {
                                log::error!("Render thread: Failed to upload frame: {}", e);
                            }
                            has_new_frame = true;
                            render_frame_count += 1;
                            if render_frame_count <= 5 || render_frame_count % 50 == 0 {
                                log::info!("Render thread: frame {} received and uploaded ({}x{}, {:?})",
                                    render_frame_count, frame.width, frame.height, frame.format);
                            }
                        }
                        WindowCommand::SetTitle(_title) => {
                            // TODO: dispatch to main thread to update NSWindow title
                        }
                        WindowCommand::Close => {
                            is_open.store(false, Ordering::Relaxed);
                            break;
                        }
                    }
                }

                // Render if we have new frame data
                if has_new_frame {
                    if let Err(e) = renderer.render(current_format) {
                        log::error!("Render failed: {}", e);
                    }
                }

                // Periodically check if the native window is still visible (~every 500ms)
                check_counter += 1;
                if check_counter % 500 == 0 {
                    let visible = unsafe {
                        use objc2::msg_send;
                        use objc2::runtime::AnyObject;
                        let window_ptr = ns_window_addr as *mut AnyObject;
                        let visible: bool = msg_send![window_ptr, isVisible];
                        visible
                    };
                    if !visible {
                        log::info!("macOS render window closed by user");
                        is_open.store(false, Ordering::Relaxed);
                        let _ = event_tx.send(WindowEvent::CloseRequested);
                        break;
                    }
                }

                // Brief sleep to avoid busy-waiting (1ms ~= 1000 fps max)
                std::thread::sleep(std::time::Duration::from_millis(1));
            }

            // Cleanup: close the window on the main thread
            if let Some(handle) = crate::APP_HANDLE.get() {
                let _ = handle.run_on_main_thread(move || unsafe {
                    use objc2::msg_send;
                    use objc2::runtime::AnyObject;
                    let window = ns_window_addr as *mut AnyObject;
                    let _: () = msg_send![window, close];
                    // Release the retained window (we retained it during creation)
                    let _: () = msg_send![window, release];
                });
            }

            log::info!("macOS render thread ended");
        });

        Ok(())
    }
}

// ---- macOS native window creation helpers ----

/// Wrapper to send raw pointers across threads safely.
/// SAFETY: The pointer must remain valid for the duration of the render thread.
#[cfg(target_os = "macos")]
#[derive(Clone, Copy)]
struct SendPtr(std::ptr::NonNull<std::ffi::c_void>);

#[cfg(target_os = "macos")]
unsafe impl Send for SendPtr {}

/// Create an NSWindow + NSView on the main thread using objc2.
/// Returns (NSView pointer, NSWindow pointer).
/// The NSWindow is retained (caller must release when done).
#[cfg(target_os = "macos")]
fn create_ns_window(
    title: &str,
    width: u32,
    height: u32,
) -> Result<(SendPtr, SendPtr), String> {
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
    use std::ffi::c_void;
    use std::ptr::NonNull;

    log::debug!(
        "Creating NSWindow on main thread: '{}' {}x{}",
        title,
        width,
        height
    );

    unsafe {
        // NSWindowStyleMask: Titled(1) | Closable(2) | Miniaturizable(4) | Resizable(8)
        let style_mask: usize = 1 | 2 | 4 | 8;

        let frame = NSRect::new(
            NSPoint::new(100.0, 100.0),
            NSSize::new(width as f64, height as f64),
        );

        // Create NSWindow
        let cls =
            AnyClass::get(c"NSWindow").ok_or_else(|| "NSWindow class not found".to_string())?;

        let alloc: *mut AnyObject = msg_send![cls, alloc];
        if alloc.is_null() {
            return Err("NSWindow alloc failed".to_string());
        }

        let window: *mut AnyObject = msg_send![
            alloc,
            initWithContentRect: frame,
            styleMask: style_mask,
            backing: 2usize, // NSBackingStoreBuffered
            defer: false
        ];
        if window.is_null() {
            return Err("NSWindow init failed".to_string());
        }

        // Retain the window so it stays alive (we'll release it on cleanup)
        let _: *mut AnyObject = msg_send![window, retain];

        // Set title
        let title_ns = NSString::from_str(title);
        let _: () = msg_send![window, setTitle: &*title_ns];

        // Get content view (NSView)
        let content_view: *mut AnyObject = msg_send![window, contentView];
        if content_view.is_null() {
            let _: () = msg_send![window, release];
            return Err("NSWindow contentView is null".to_string());
        }

        // Enable layer-backed view for Metal rendering
        let _: () = msg_send![content_view, setWantsLayer: true];

        // Center window on screen and make it visible
        let _: () = msg_send![window, center];
        let _: () = msg_send![window, makeKeyAndOrderFront: std::ptr::null::<AnyObject>()];

        let view_ptr = NonNull::new(content_view as *mut c_void)
            .ok_or_else(|| "Failed to get NSView pointer".to_string())?;
        let window_ptr = NonNull::new(window as *mut c_void)
            .ok_or_else(|| "Failed to get NSWindow pointer".to_string())?;

        log::debug!("NSWindow created and displayed successfully");

        Ok((SendPtr(view_ptr), SendPtr(window_ptr)))
    }
}

// ---- winit-based ApplicationHandler (non-macOS) ----

#[cfg(not(target_os = "macos"))]
impl RenderWindow {
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

#[cfg(not(target_os = "macos"))]
impl ApplicationHandler for RenderWindow {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        log::debug!(
            "EventLoop resumed, creating window '{}' ({}x{})",
            self.title, self.width, self.height
        );

        let window_attrs = WindowAttributes::default()
            .with_title(&self.title)
            .with_inner_size(PhysicalSize::new(self.width, self.height));

        let window = match event_loop.create_window(window_attrs) {
            Ok(w) => {
                log::debug!("winit window created successfully");
                Arc::new(w)
            }
            Err(e) => {
                log::error!("Failed to create winit window: {}", e);
                self.is_open.store(false, Ordering::Relaxed);
                event_loop.exit();
                return;
            }
        };

        // Initialize renderer
        log::debug!("Initializing wgpu renderer...");
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
                log::error!("Failed to create wgpu renderer: {}", e);
                self.is_open.store(false, Ordering::Relaxed);
                event_loop.exit();
                return;
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
