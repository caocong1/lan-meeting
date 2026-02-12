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
    ResolutionRequested(u32, u32, u32), // (target_width, target_height, bitrate) from toolbar
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

        // Read default resolution/bitrate indices from settings
        let (default_res_idx, default_br_idx) = crate::commands::get_default_streaming_indices();

        // Create floating toolbar on main thread (using child NSPanel for reliable rendering over Metal)
        let (toolbar_tx, toolbar_rx) =
            std::sync::mpsc::channel::<Result<(usize, usize, usize), String>>();

        let window_addr_for_toolbar = ns_window_addr;
        app_handle
            .run_on_main_thread(move || {
                let result = create_toolbar_panel(window_addr_for_toolbar, width, default_res_idx, default_br_idx);
                let _ = toolbar_tx.send(result);
            })
            .map_err(|e| {
                RendererError::WindowError(format!("Failed to dispatch toolbar creation: {}", e))
            })?;

        let (toolbar_panel_addr, res_popup_addr, br_popup_addr) = toolbar_rx
            .recv()
            .map_err(|e| {
                RendererError::WindowError(format!("Toolbar channel closed: {}", e))
            })?
            .map_err(|e| RendererError::WindowError(format!("Toolbar creation failed: {}", e)))?;

        log::debug!("Floating toolbar panel created on main thread (res={}, br={})", default_res_idx, default_br_idx);

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
        let is_open_for_panic = is_open.clone();
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
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
            let mut last_surface_w: u32 = width;
            let mut last_surface_h: u32 = height;

            // Toolbar state (initialized from settings defaults)
            let mut toolbar_visible = false;
            let mut last_mouse_x: f64 = -1.0;
            let mut last_mouse_y: f64 = -1.0;
            let mut last_mouse_move_time = std::time::Instant::now();
            let mut last_selected_resolution: isize = default_res_idx as isize;
            let mut last_selected_bitrate: isize = default_br_idx as isize;
            let toolbar_hide_delay = std::time::Duration::from_secs(3);

            // Simple render loop (no winit event loop needed)
            loop {
                if !is_open.load(Ordering::Relaxed) {
                    break;
                }

                let mut has_new_frame = false;

                // Process all pending commands - only keep the latest frame
                let mut latest_frame: Option<RenderFrame> = None;
                let mut stale_count: u32 = 0;
                while let Ok(cmd) = command_rx.try_recv() {
                    match cmd {
                        WindowCommand::RenderFrame(frame) => {
                            if latest_frame.is_some() {
                                stale_count += 1;
                            }
                            latest_frame = Some(frame);
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

                // Upload only the latest frame, skip stale ones
                if let Some(frame) = latest_frame {
                    current_format = frame.format;
                    render_frame_count += 1;
                    if stale_count > 0 {
                        log::info!("Render thread: dropped {} stale frames, rendering frame {}",
                            stale_count, render_frame_count);
                    }
                    if let Err(e) = renderer.upload_frame(&frame) {
                        log::error!("Render thread: Failed to upload frame: {}", e);
                    }
                    has_new_frame = true;
                    if render_frame_count <= 5 || render_frame_count % 50 == 0 {
                        log::info!("Render thread: frame {} uploaded ({}x{}, {:?})",
                            render_frame_count, frame.width, frame.height, frame.format);
                    }
                }

                // Detect window resize by querying NSView backing size
                if has_new_frame {
                    let (pixel_w, pixel_h) = unsafe {
                        use objc2::msg_send;
                        use objc2::runtime::AnyObject;

                        let view = ns_view_addr as *mut AnyObject;
                        let window_ptr = ns_window_addr as *mut AnyObject;

                        // Get logical bounds of the view
                        let bounds: objc2_foundation::NSRect = msg_send![view, bounds];
                        // Get backing scale factor (Retina)
                        let scale: f64 = msg_send![window_ptr, backingScaleFactor];

                        let pw = (bounds.size.width * scale) as u32;
                        let ph = (bounds.size.height * scale) as u32;
                        (pw.max(1), ph.max(1))
                    };

                    if pixel_w != last_surface_w || pixel_h != last_surface_h {
                        log::info!("Render thread: window resized {}x{} -> {}x{}",
                            last_surface_w, last_surface_h, pixel_w, pixel_h);
                        renderer.resize(pixel_w, pixel_h);
                        last_surface_w = pixel_w;
                        last_surface_h = pixel_h;
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

                // Toolbar: mouse tracking + auto-hide + resolution polling
                if check_counter % 10 == 0 { // every ~10ms
                    let (mouse_in_window, mouse_x, mouse_y) = unsafe {
                        use objc2::msg_send;
                        use objc2::runtime::AnyObject;
                        let window_ptr = ns_window_addr as *mut AnyObject;
                        let view = ns_view_addr as *mut AnyObject;

                        let mouse_loc: objc2_foundation::NSPoint =
                            msg_send![window_ptr, mouseLocationOutsideOfEventStream];
                        let bounds: objc2_foundation::NSRect = msg_send![view, bounds];

                        let inside = mouse_loc.x >= 0.0
                            && mouse_loc.y >= 0.0
                            && mouse_loc.x <= bounds.size.width
                            && mouse_loc.y <= bounds.size.height;

                        (inside, mouse_loc.x, mouse_loc.y)
                    };

                    // Detect mouse movement
                    let mouse_moved = (mouse_x - last_mouse_x).abs() > 1.0
                        || (mouse_y - last_mouse_y).abs() > 1.0;
                    if mouse_moved && mouse_in_window {
                        last_mouse_move_time = std::time::Instant::now();
                    }
                    last_mouse_x = mouse_x;
                    last_mouse_y = mouse_y;

                    let should_show = mouse_in_window
                        && last_mouse_move_time.elapsed() < toolbar_hide_delay;

                    // Update toolbar panel visibility on state change
                    if should_show != toolbar_visible {
                        toolbar_visible = should_show;
                        if let Some(handle) = crate::APP_HANDLE.get() {
                            let panel_addr = toolbar_panel_addr;
                            let win_addr = ns_window_addr;
                            let show = should_show;
                            let _ = handle.run_on_main_thread(move || unsafe {
                                use objc2::msg_send;
                                use objc2::runtime::AnyObject;
                                use objc2_foundation::{NSPoint, NSRect, NSSize};
                                let panel = panel_addr as *mut AnyObject;
                                if show {
                                    // Reposition panel to stay centered at top of main window
                                    let main_win = win_addr as *mut AnyObject;
                                    let main_frame: NSRect = msg_send![main_win, frame];
                                    let content_rect: NSRect = msg_send![
                                        main_win,
                                        contentRectForFrameRect: main_frame
                                    ];
                                    let toolbar_w: f64 = 320.0;
                                    let toolbar_h: f64 = 36.0;
                                    let px = content_rect.origin.x
                                        + (content_rect.size.width - toolbar_w) / 2.0;
                                    let py = content_rect.origin.y
                                        + content_rect.size.height - toolbar_h - 8.0;
                                    let panel_frame = NSRect::new(
                                        NSPoint::new(px, py),
                                        NSSize::new(toolbar_w, toolbar_h),
                                    );
                                    let _: () = msg_send![panel, setFrame: panel_frame, display: false];
                                    let _: () = msg_send![panel, orderFront: std::ptr::null::<AnyObject>()];
                                } else {
                                    let _: () = msg_send![panel, orderOut: std::ptr::null::<AnyObject>()];
                                }
                            });
                        }
                    }

                    // Poll both NSPopUpButtons (~every 100ms)
                    if check_counter % 100 == 0 {
                        let res_selected: isize = unsafe {
                            use objc2::msg_send;
                            use objc2::runtime::AnyObject;
                            let popup = res_popup_addr as *mut AnyObject;
                            msg_send![popup, indexOfSelectedItem]
                        };
                        let br_selected: isize = unsafe {
                            use objc2::msg_send;
                            use objc2::runtime::AnyObject;
                            let popup = br_popup_addr as *mut AnyObject;
                            msg_send![popup, indexOfSelectedItem]
                        };

                        // Send event if either dropdown changed
                        if (res_selected != last_selected_resolution || br_selected != last_selected_bitrate)
                            && res_selected >= 0 && br_selected >= 0
                        {
                            last_selected_resolution = res_selected;
                            last_selected_bitrate = br_selected;

                            let res_opts = &crate::simple_streaming::RESOLUTION_OPTIONS;
                            let br_opts = &crate::simple_streaming::BITRATE_OPTIONS;
                            if let (Some(res), Some(br)) = (
                                res_opts.get(res_selected as usize),
                                br_opts.get(br_selected as usize),
                            ) {
                                log::info!("Toolbar: {} + {}",
                                    res.label, br.label);
                                let _ = event_tx.send(WindowEvent::ResolutionRequested(
                                    res.target_width, res.target_height, br.bitrate,
                                ));
                            }
                        }
                    }
                }

                // Brief sleep to avoid busy-waiting (1ms ~= 1000 fps max)
                std::thread::sleep(std::time::Duration::from_millis(1));
            }

            // Cleanup: close the toolbar panel and window on the main thread
            if let Some(handle) = crate::APP_HANDLE.get() {
                let _ = handle.run_on_main_thread(move || unsafe {
                    use objc2::msg_send;
                    use objc2::runtime::AnyObject;
                    // Close toolbar panel first
                    let panel = toolbar_panel_addr as *mut AnyObject;
                    let _: () = msg_send![panel, orderOut: std::ptr::null::<AnyObject>()];
                    let _: () = msg_send![panel, close];
                    let window = ns_window_addr as *mut AnyObject;
                    let _: () = msg_send![window, close];
                    // Release the retained window (we retained it during creation)
                    let _: () = msg_send![window, release];
                });
            }

            log::info!("macOS render thread ended");
            })); // end catch_unwind

            if let Err(panic_info) = result {
                let msg = if let Some(s) = panic_info.downcast_ref::<&str>() {
                    s.to_string()
                } else if let Some(s) = panic_info.downcast_ref::<String>() {
                    s.clone()
                } else {
                    "unknown panic".to_string()
                };
                log::error!("macOS render thread PANICKED: {}", msg);
                is_open_for_panic.store(false, Ordering::Relaxed);
            }
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

/// Create a floating toolbar as a child NSPanel window.
/// Using a child window ensures reliable rendering over Metal/wgpu content,
/// since subviews of the Metal content view may be hidden by the CAMetalLayer.
/// Returns (panel_addr, resolution_popup_addr, bitrate_popup_addr) as usize.
/// Must be called on the main thread.
#[cfg(target_os = "macos")]
fn create_toolbar_panel(window_addr: usize, _window_width: u32, default_res_idx: usize, default_br_idx: usize) -> Result<(usize, usize, usize), String> {
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

    unsafe {
        let main_window = window_addr as *mut AnyObject;

        // Compute panel position from main window's content area
        let main_frame: NSRect = msg_send![main_window, frame];
        let content_rect: NSRect = msg_send![main_window, contentRectForFrameRect: main_frame];

        let toolbar_w: f64 = 320.0;
        let toolbar_h: f64 = 36.0;
        let panel_x = content_rect.origin.x + (content_rect.size.width - toolbar_w) / 2.0;
        let panel_y = content_rect.origin.y + content_rect.size.height - toolbar_h - 8.0;

        let panel_frame = NSRect::new(
            NSPoint::new(panel_x, panel_y),
            NSSize::new(toolbar_w, toolbar_h),
        );

        // Create borderless, non-activating NSPanel
        let panel_cls = AnyClass::get(c"NSPanel").ok_or("NSPanel class not found")?;
        let panel_alloc: *mut AnyObject = msg_send![panel_cls, alloc];
        // NSWindowStyleMaskBorderless = 0, NSWindowStyleMaskNonactivatingPanel = 128
        let style_mask: usize = 128;
        let panel: *mut AnyObject = msg_send![
            panel_alloc,
            initWithContentRect: panel_frame,
            styleMask: style_mask,
            backing: 2usize,
            defer: false
        ];
        if panel.is_null() {
            return Err("NSPanel alloc failed".to_string());
        }

        // Panel configuration
        let _: () = msg_send![panel, setOpaque: false];
        let _: () = msg_send![panel, setHasShadow: false];
        // Clicking the panel should not steal focus from the main window
        let _: () = msg_send![panel, setBecomesKeyOnlyIfNeeded: true];

        // Set semi-transparent dark background on the panel window itself
        let ns_color_cls = AnyClass::get(c"NSColor").ok_or("NSColor not found")?;
        let bg_color: *mut AnyObject = msg_send![
            ns_color_cls,
            colorWithRed: 0.0f64,
            green: 0.0f64,
            blue: 0.0f64,
            alpha: 0.7f64
        ];
        let _: () = msg_send![panel, setBackgroundColor: bg_color];

        // Round corners via panel's contentView layer
        let panel_content: *mut AnyObject = msg_send![panel, contentView];
        let _: () = msg_send![panel_content, setWantsLayer: true];
        let layer: *mut AnyObject = msg_send![panel_content, layer];
        if !layer.is_null() {
            let _: () = msg_send![layer, setCornerRadius: 10.0f64];
            let _: () = msg_send![layer, setMasksToBounds: true];
        }

        // Add as child window of main window (NSWindowAbove = 1)
        let _: () = msg_send![main_window, addChildWindow: panel, ordered: 1isize];

        // Create popup buttons on the panel's content view
        let popup_cls = AnyClass::get(c"NSPopUpButton").ok_or("NSPopUpButton not found")?;
        let font_cls = AnyClass::get(c"NSFont").ok_or("NSFont not found")?;
        let font: *mut AnyObject = msg_send![font_cls, systemFontOfSize: 12.0f64];

        // --- Resolution dropdown (left side) ---
        let popup_w: f64 = 140.0;
        let res_frame = NSRect::new(
            NSPoint::new(10.0, 4.0),
            NSSize::new(popup_w, 28.0),
        );
        let res_alloc: *mut AnyObject = msg_send![popup_cls, alloc];
        let res_popup: *mut AnyObject = msg_send![
            res_alloc,
            initWithFrame: res_frame,
            pullsDown: false
        ];
        if res_popup.is_null() {
            return Err("Resolution NSPopUpButton alloc failed".to_string());
        }
        let _: () = msg_send![res_popup, setFont: font];

        for opt in &crate::simple_streaming::RESOLUTION_OPTIONS {
            let ns_title = NSString::from_str(opt.label);
            let _: () = msg_send![res_popup, addItemWithTitle: &*ns_title];
        }
        let res_idx = (default_res_idx as isize).min(crate::simple_streaming::RESOLUTION_OPTIONS.len() as isize - 1);
        let _: () = msg_send![res_popup, selectItemAtIndex: res_idx];

        // --- Bitrate dropdown (right side) ---
        let br_frame = NSRect::new(
            NSPoint::new(10.0 + popup_w + 10.0, 4.0),
            NSSize::new(popup_w, 28.0),
        );
        let br_alloc: *mut AnyObject = msg_send![popup_cls, alloc];
        let br_popup: *mut AnyObject = msg_send![
            br_alloc,
            initWithFrame: br_frame,
            pullsDown: false
        ];
        if br_popup.is_null() {
            return Err("Bitrate NSPopUpButton alloc failed".to_string());
        }
        let _: () = msg_send![br_popup, setFont: font];

        for opt in &crate::simple_streaming::BITRATE_OPTIONS {
            let ns_title = NSString::from_str(opt.label);
            let _: () = msg_send![br_popup, addItemWithTitle: &*ns_title];
        }
        let br_idx = (default_br_idx as isize).min(crate::simple_streaming::BITRATE_OPTIONS.len() as isize - 1);
        let _: () = msg_send![br_popup, selectItemAtIndex: br_idx];

        // Add both popups to panel's content view
        let _: () = msg_send![panel_content, addSubview: res_popup];
        let _: () = msg_send![panel_content, addSubview: br_popup];

        // Initially hidden (orderOut removes from screen)
        let _: () = msg_send![panel, orderOut: std::ptr::null::<AnyObject>()];

        log::debug!("Floating toolbar panel created with resolution + bitrate dropdowns");

        Ok((panel as usize, res_popup as usize, br_popup as usize))
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
