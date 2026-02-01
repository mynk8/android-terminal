mod core;

use android_activity::AndroidApp;
use glutin::config::Config;
use glutin::{
    config::ConfigTemplateBuilder,
    context::{
        ContextApi, ContextAttributesBuilder, NotCurrentGlContext, PossiblyCurrentContext, Version,
    },
    display::{GetGlDisplay, GlDisplay},
    prelude::GlSurface,
    surface::{Surface as GlutinSurface, SurfaceAttributesBuilder, WindowSurface},
};
use glutin_winit::DisplayBuilder;
use raw_window_handle::HasWindowHandle;
use skia_safe::{
    ColorType, Surface,
    gpu::{
        Protected, SurfaceOrigin, backend_render_targets, direct_contexts, gl::FramebufferInfo,
        surfaces,
    },
};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::{
    ffi::CString,
    num::NonZeroU32,
    time::{Duration, Instant},
};
use winit::{
    application::ApplicationHandler,
    event::{ElementState, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy},
    keyboard::{KeyCode, PhysicalKey},
    window::{Window, WindowId},
};

use crate::core::{Parser, Pty, Renderer, Term};

#[derive(Debug, Clone)]
enum AppEvent {
    CursorBlink,
    PtyOutput(Vec<u8>),
}

const CURSOR_BLINK_MS: u64 = 500;
const PTY_POLL_MS: u64 = 10;
const DEFAULT_SHELL: &str = "/system/bin/sh";

#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default().with_max_level(log::LevelFilter::Info),
    );

    use winit::platform::android::EventLoopBuilderExtAndroid;
    let event_loop: EventLoop<AppEvent> = EventLoop::with_user_event()
        .with_android_app(app)
        .build()
        .expect("Failed to create event loop");

    let proxy = event_loop.create_proxy();
    let mut application = App::new(proxy);

    log::info!("Starting terminal emulator...");
    let _ = event_loop.run_app(&mut application);
}

struct App {
    state: Option<AppState>,
    event_proxy: EventLoopProxy<AppEvent>,
    threads_running: Arc<AtomicBool>,
    pty: Option<Arc<Pty>>,
}

impl App {
    fn new(proxy: EventLoopProxy<AppEvent>) -> Self {
        Self {
            state: None,
            event_proxy: proxy,
            threads_running: Arc::new(AtomicBool::new(false)),
            pty: None,
        }
    }

    fn start_background_threads(&mut self, rows: u16, cols: u16) {
        if self.threads_running.swap(true, Ordering::SeqCst) {
            return;
        }

        match Pty::spawn(DEFAULT_SHELL, rows, cols) {
            Ok(pty) => {
                log::info!("PTY spawned successfully");
                let pty = Arc::new(pty);
                self.pty = Some(pty.clone());

                let proxy = self.event_proxy.clone();
                let running = self.threads_running.clone();
                let pty_reader = pty.clone();
                std::thread::spawn(move || {
                    log::info!("PTY reader thread started");
                    let mut buf = [0u8; 4096];
                    while running.load(Ordering::SeqCst) {
                        match pty_reader.read(&mut buf) {
                            Ok(0) => {
                                std::thread::sleep(Duration::from_millis(PTY_POLL_MS));
                            }
                            Ok(n) => {
                                let data = buf[..n].to_vec();
                                let _ = proxy.send_event(AppEvent::PtyOutput(data));
                            }
                            Err(e) => {
                                log::error!("PTY read error: {:?}", e);
                                break;
                            }
                        }
                    }
                    log::info!("PTY reader thread stopped");
                });
            }
            Err(e) => {
                log::error!("Failed to spawn PTY: {:?}", e);
            }
        }

        let proxy = self.event_proxy.clone();
        let running = self.threads_running.clone();
        std::thread::spawn(move || {
            log::info!("Cursor blink timer started");
            while running.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(CURSOR_BLINK_MS));
                if running.load(Ordering::SeqCst) {
                    let _ = proxy.send_event(AppEvent::CursorBlink);
                }
            }
            log::info!("Cursor blink timer stopped");
        });
    }

    fn stop_background_threads(&mut self) {
        self.threads_running.store(false, Ordering::SeqCst);
    }
}

struct AppState {
    window: Window,
    #[allow(dead_code)]
    gl_config: Config,
    gl_context: PossiblyCurrentContext,
    gl_surface: GlutinSurface<WindowSurface>,
    gr_context: skia_safe::gpu::DirectContext,
    skia_surface: Surface,

    term: Term,
    renderer: Renderer,
    parser: Parser,

    cursor_visible: bool,
    last_input: Instant,

    ctrl_pressed: bool,
    shift_pressed: bool,
}

impl AppState {
    fn init(event_loop: &ActiveEventLoop) -> Self {
        let template = ConfigTemplateBuilder::new()
            .with_alpha_size(8)
            .with_depth_size(0)
            .with_stencil_size(8);

        let display_builder =
            DisplayBuilder::new().with_window_attributes(Some(Window::default_attributes()));

        let (window, gl_config) = display_builder
            .build(event_loop, template, |mut configs| configs.next().unwrap())
            .unwrap();

        let window = window.expect("Failed to create window");
        let raw_window_handle = window.window_handle().unwrap().as_raw();

        let context_attrs = ContextAttributesBuilder::new()
            .with_context_api(ContextApi::Gles(Some(Version::new(2, 0))))
            .build(Some(raw_window_handle));

        let gl_display = gl_config.display();

        let not_current = unsafe {
            gl_display
                .create_context(&gl_config, &context_attrs)
                .unwrap()
        };

        let size = window.inner_size();

        let surface_attrs = SurfaceAttributesBuilder::<WindowSurface>::new().build(
            raw_window_handle,
            NonZeroU32::new(size.width.max(1)).unwrap(),
            NonZeroU32::new(size.height.max(1)).unwrap(),
        );

        let gl_surface = unsafe {
            gl_display
                .create_window_surface(&gl_config, &surface_attrs)
                .unwrap()
        };

        let gl_context = not_current.make_current(&gl_surface).unwrap();

        gl_surface
            .set_swap_interval(&gl_context, glutin::surface::SwapInterval::DontWait)
            .unwrap_or_else(|e| log::warn!("Failed to disable VSync: {:?}", e));

        gl::load_with(|s| gl_display.get_proc_address(&CString::new(s).unwrap()));

        let interface = skia_safe::gpu::gl::Interface::new_load_with(|s| {
            gl_display.get_proc_address(&CString::new(s).unwrap())
        })
        .expect("Failed to create Skia GL interface");

        let mut gr_context =
            direct_contexts::make_gl(interface, None).expect("Failed to create Skia DirectContext");

        let fb_info = FramebufferInfo {
            fboid: 0,
            format: skia_safe::gpu::gl::Format::RGBA8.into(),
            protected: Protected::No,
        };

        let backend_rt =
            backend_render_targets::make_gl((size.width as i32, size.height as i32), 0, 8, fb_info);

        let skia_surface = surfaces::wrap_backend_render_target(
            &mut gr_context,
            &backend_rt,
            SurfaceOrigin::BottomLeft,
            ColorType::RGBA8888,
            None,
            None,
        )
        .expect("Failed to create Skia surface");

        let renderer = Renderer::new();
        let cols = (size.width as f32 / renderer.cell_w).floor() as usize;
        let rows = (size.height as f32 / renderer.cell_h).floor() as usize;
        let cols = cols.max(1);
        let rows = rows.max(1);

        log::info!("Terminal size: {}x{} cells", cols, rows);

        let term = Term::new(cols, rows);
        let parser = Parser::new();

        Self {
            window,
            gl_config,
            gl_context,
            gl_surface,
            gr_context,
            skia_surface,
            term,
            renderer,
            parser,
            cursor_visible: true,
            last_input: Instant::now(),
            ctrl_pressed: false,
            shift_pressed: false,
        }
    }

    fn cols(&self) -> u16 {
        self.term.cols as u16
    }

    fn rows(&self) -> u16 {
        self.term.rows as u16
    }

    fn resize(&mut self, width: u32, height: u32) {
        let fb_info = FramebufferInfo {
            fboid: 0,
            format: skia_safe::gpu::gl::Format::RGBA8.into(),
            protected: Protected::No,
        };

        let backend_rt =
            backend_render_targets::make_gl((width as i32, height as i32), 0, 0, fb_info);

        self.skia_surface = surfaces::wrap_backend_render_target(
            &mut self.gr_context,
            &backend_rt,
            SurfaceOrigin::BottomLeft,
            ColorType::RGBA8888,
            None,
            None,
        )
        .unwrap();

        let new_cols = (width as f32 / self.renderer.cell_w).floor() as usize;
        let new_rows = (height as f32 / self.renderer.cell_h).floor() as usize;
        let new_cols = new_cols.max(1);
        let new_rows = new_rows.max(1);

        if new_cols != self.term.cols || new_rows != self.term.rows {
            log::info!(
                "Terminal resized: {}x{} -> {}x{}",
                self.term.cols,
                self.term.rows,
                new_cols,
                new_rows
            );
            self.term = Term::new(new_cols, new_rows);
        }
    }

    fn render(&mut self) {
        let canvas = self.skia_surface.canvas();
        self.renderer
            .render(canvas, &self.term, self.cursor_visible);
        self.gr_context.flush_and_submit();
        self.gl_surface.swap_buffers(&self.gl_context).unwrap();
    }

    /// Toggle cursor blink state
    fn toggle_cursor_blink(&mut self) {
        if self.last_input.elapsed() > Duration::from_millis(CURSOR_BLINK_MS) {
            self.cursor_visible = !self.cursor_visible;
            self.term.dirty[self.term.cursor.y] = true;
        }
    }

    /// Reset cursor to visible on input
    fn reset_cursor(&mut self) {
        self.cursor_visible = true;
        self.last_input = Instant::now();
    }

    /// Process PTY output data through the parser
    fn process_pty_output(&mut self, data: &[u8]) {
        for &byte in data {
            self.parser.process(&mut self.term, byte);
        }
    }

    /// Convert physical keycode to bytes for PTY, considering modifiers
    fn keycode_to_bytes(key: &PhysicalKey, ctrl: bool, shift: bool) -> Option<Vec<u8>> {
        // Ctrl + letter = ASCII control character (1-26)
        if ctrl {
            return match key {
                PhysicalKey::Code(KeyCode::KeyA) => Some(vec![0x01]), // SOH
                PhysicalKey::Code(KeyCode::KeyB) => Some(vec![0x02]), // STX
                PhysicalKey::Code(KeyCode::KeyC) => Some(vec![0x03]), // ETX - SIGINT
                PhysicalKey::Code(KeyCode::KeyD) => Some(vec![0x04]), // EOT - EOF
                PhysicalKey::Code(KeyCode::KeyE) => Some(vec![0x05]), // ENQ
                PhysicalKey::Code(KeyCode::KeyF) => Some(vec![0x06]), // ACK
                PhysicalKey::Code(KeyCode::KeyG) => Some(vec![0x07]), // BEL
                PhysicalKey::Code(KeyCode::KeyH) => Some(vec![0x08]), // BS
                PhysicalKey::Code(KeyCode::KeyI) => Some(vec![0x09]), // HT (tab)
                PhysicalKey::Code(KeyCode::KeyJ) => Some(vec![0x0a]), // LF
                PhysicalKey::Code(KeyCode::KeyK) => Some(vec![0x0b]), // VT
                PhysicalKey::Code(KeyCode::KeyL) => Some(vec![0x0c]), // FF - clear
                PhysicalKey::Code(KeyCode::KeyM) => Some(vec![0x0d]), // CR
                PhysicalKey::Code(KeyCode::KeyN) => Some(vec![0x0e]), // SO
                PhysicalKey::Code(KeyCode::KeyO) => Some(vec![0x0f]), // SI
                PhysicalKey::Code(KeyCode::KeyP) => Some(vec![0x10]), // DLE
                PhysicalKey::Code(KeyCode::KeyQ) => Some(vec![0x11]), // DC1
                PhysicalKey::Code(KeyCode::KeyR) => Some(vec![0x12]), // DC2
                PhysicalKey::Code(KeyCode::KeyS) => Some(vec![0x13]), // DC3
                PhysicalKey::Code(KeyCode::KeyT) => Some(vec![0x14]), // DC4
                PhysicalKey::Code(KeyCode::KeyU) => Some(vec![0x15]), // NAK
                PhysicalKey::Code(KeyCode::KeyV) => Some(vec![0x16]), // SYN
                PhysicalKey::Code(KeyCode::KeyW) => Some(vec![0x17]), // ETB
                PhysicalKey::Code(KeyCode::KeyX) => Some(vec![0x18]), // CAN
                PhysicalKey::Code(KeyCode::KeyY) => Some(vec![0x19]), // EM
                PhysicalKey::Code(KeyCode::KeyZ) => Some(vec![0x1a]), // SUB - SIGTSTP
                PhysicalKey::Code(KeyCode::BracketLeft) => Some(vec![0x1b]), // ESC
                PhysicalKey::Code(KeyCode::Backslash) => Some(vec![0x1c]), // FS
                PhysicalKey::Code(KeyCode::BracketRight) => Some(vec![0x1d]), // GS
                PhysicalKey::Code(KeyCode::Digit6) => Some(vec![0x1e]), // RS (Ctrl+^)
                PhysicalKey::Code(KeyCode::Minus) => Some(vec![0x1f]), // US (Ctrl+_)
                _ => None,
            };
        }

        match key {
            // Letters a-z (handle shift for uppercase)
            PhysicalKey::Code(KeyCode::KeyA) => Some(vec![if shift { b'A' } else { b'a' }]),
            PhysicalKey::Code(KeyCode::KeyB) => Some(vec![if shift { b'B' } else { b'b' }]),
            PhysicalKey::Code(KeyCode::KeyC) => Some(vec![if shift { b'C' } else { b'c' }]),
            PhysicalKey::Code(KeyCode::KeyD) => Some(vec![if shift { b'D' } else { b'd' }]),
            PhysicalKey::Code(KeyCode::KeyE) => Some(vec![if shift { b'E' } else { b'e' }]),
            PhysicalKey::Code(KeyCode::KeyF) => Some(vec![if shift { b'F' } else { b'f' }]),
            PhysicalKey::Code(KeyCode::KeyG) => Some(vec![if shift { b'G' } else { b'g' }]),
            PhysicalKey::Code(KeyCode::KeyH) => Some(vec![if shift { b'H' } else { b'h' }]),
            PhysicalKey::Code(KeyCode::KeyI) => Some(vec![if shift { b'I' } else { b'i' }]),
            PhysicalKey::Code(KeyCode::KeyJ) => Some(vec![if shift { b'J' } else { b'j' }]),
            PhysicalKey::Code(KeyCode::KeyK) => Some(vec![if shift { b'K' } else { b'k' }]),
            PhysicalKey::Code(KeyCode::KeyL) => Some(vec![if shift { b'L' } else { b'l' }]),
            PhysicalKey::Code(KeyCode::KeyM) => Some(vec![if shift { b'M' } else { b'm' }]),
            PhysicalKey::Code(KeyCode::KeyN) => Some(vec![if shift { b'N' } else { b'n' }]),
            PhysicalKey::Code(KeyCode::KeyO) => Some(vec![if shift { b'O' } else { b'o' }]),
            PhysicalKey::Code(KeyCode::KeyP) => Some(vec![if shift { b'P' } else { b'p' }]),
            PhysicalKey::Code(KeyCode::KeyQ) => Some(vec![if shift { b'Q' } else { b'q' }]),
            PhysicalKey::Code(KeyCode::KeyR) => Some(vec![if shift { b'R' } else { b'r' }]),
            PhysicalKey::Code(KeyCode::KeyS) => Some(vec![if shift { b'S' } else { b's' }]),
            PhysicalKey::Code(KeyCode::KeyT) => Some(vec![if shift { b'T' } else { b't' }]),
            PhysicalKey::Code(KeyCode::KeyU) => Some(vec![if shift { b'U' } else { b'u' }]),
            PhysicalKey::Code(KeyCode::KeyV) => Some(vec![if shift { b'V' } else { b'v' }]),
            PhysicalKey::Code(KeyCode::KeyW) => Some(vec![if shift { b'W' } else { b'w' }]),
            PhysicalKey::Code(KeyCode::KeyX) => Some(vec![if shift { b'X' } else { b'x' }]),
            PhysicalKey::Code(KeyCode::KeyY) => Some(vec![if shift { b'Y' } else { b'y' }]),
            PhysicalKey::Code(KeyCode::KeyZ) => Some(vec![if shift { b'Z' } else { b'z' }]),

            // Numbers and shift symbols
            PhysicalKey::Code(KeyCode::Digit1) => Some(vec![if shift { b'!' } else { b'1' }]),
            PhysicalKey::Code(KeyCode::Digit2) => Some(vec![if shift { b'@' } else { b'2' }]),
            PhysicalKey::Code(KeyCode::Digit3) => Some(vec![if shift { b'#' } else { b'3' }]),
            PhysicalKey::Code(KeyCode::Digit4) => Some(vec![if shift { b'$' } else { b'4' }]),
            PhysicalKey::Code(KeyCode::Digit5) => Some(vec![if shift { b'%' } else { b'5' }]),
            PhysicalKey::Code(KeyCode::Digit6) => Some(vec![if shift { b'^' } else { b'6' }]),
            PhysicalKey::Code(KeyCode::Digit7) => Some(vec![if shift { b'&' } else { b'7' }]),
            PhysicalKey::Code(KeyCode::Digit8) => Some(vec![if shift { b'*' } else { b'8' }]),
            PhysicalKey::Code(KeyCode::Digit9) => Some(vec![if shift { b'(' } else { b'9' }]),
            PhysicalKey::Code(KeyCode::Digit0) => Some(vec![if shift { b')' } else { b'0' }]),

            // Special keys
            PhysicalKey::Code(KeyCode::Space) => Some(vec![b' ']),
            PhysicalKey::Code(KeyCode::Enter) => Some(vec![b'\n']),
            PhysicalKey::Code(KeyCode::Backspace) => Some(vec![0x7f]), // DEL
            PhysicalKey::Code(KeyCode::Tab) => Some(vec![b'\t']),
            PhysicalKey::Code(KeyCode::Escape) => Some(vec![0x1b]),

            // Punctuation with shift variants
            PhysicalKey::Code(KeyCode::Period) => Some(vec![if shift { b'>' } else { b'.' }]),
            PhysicalKey::Code(KeyCode::Comma) => Some(vec![if shift { b'<' } else { b',' }]),
            PhysicalKey::Code(KeyCode::Semicolon) => Some(vec![if shift { b':' } else { b';' }]),
            PhysicalKey::Code(KeyCode::Quote) => Some(vec![if shift { b'"' } else { b'\'' }]),
            PhysicalKey::Code(KeyCode::Slash) => Some(vec![if shift { b'?' } else { b'/' }]),
            PhysicalKey::Code(KeyCode::Backslash) => Some(vec![if shift { b'|' } else { b'\\' }]),
            PhysicalKey::Code(KeyCode::Minus) => Some(vec![if shift { b'_' } else { b'-' }]),
            PhysicalKey::Code(KeyCode::Equal) => Some(vec![if shift { b'+' } else { b'=' }]),
            PhysicalKey::Code(KeyCode::BracketLeft) => Some(vec![if shift { b'{' } else { b'[' }]),
            PhysicalKey::Code(KeyCode::BracketRight) => Some(vec![if shift { b'}' } else { b']' }]),
            PhysicalKey::Code(KeyCode::Backquote) => Some(vec![if shift { b'~' } else { b'`' }]),

            // Arrow keys (ANSI escape sequences)
            PhysicalKey::Code(KeyCode::ArrowUp) => Some(vec![0x1b, b'[', b'A']),
            PhysicalKey::Code(KeyCode::ArrowDown) => Some(vec![0x1b, b'[', b'B']),
            PhysicalKey::Code(KeyCode::ArrowRight) => Some(vec![0x1b, b'[', b'C']),
            PhysicalKey::Code(KeyCode::ArrowLeft) => Some(vec![0x1b, b'[', b'D']),

            // Home/End/Page keys
            PhysicalKey::Code(KeyCode::Home) => Some(vec![0x1b, b'[', b'H']),
            PhysicalKey::Code(KeyCode::End) => Some(vec![0x1b, b'[', b'F']),
            PhysicalKey::Code(KeyCode::PageUp) => Some(vec![0x1b, b'[', b'5', b'~']),
            PhysicalKey::Code(KeyCode::PageDown) => Some(vec![0x1b, b'[', b'6', b'~']),
            PhysicalKey::Code(KeyCode::Delete) => Some(vec![0x1b, b'[', b'3', b'~']),
            PhysicalKey::Code(KeyCode::Insert) => Some(vec![0x1b, b'[', b'2', b'~']),

            // Function keys
            PhysicalKey::Code(KeyCode::F1) => Some(vec![0x1b, b'O', b'P']),
            PhysicalKey::Code(KeyCode::F2) => Some(vec![0x1b, b'O', b'Q']),
            PhysicalKey::Code(KeyCode::F3) => Some(vec![0x1b, b'O', b'R']),
            PhysicalKey::Code(KeyCode::F4) => Some(vec![0x1b, b'O', b'S']),
            PhysicalKey::Code(KeyCode::F5) => Some(vec![0x1b, b'[', b'1', b'5', b'~']),
            PhysicalKey::Code(KeyCode::F6) => Some(vec![0x1b, b'[', b'1', b'7', b'~']),
            PhysicalKey::Code(KeyCode::F7) => Some(vec![0x1b, b'[', b'1', b'8', b'~']),
            PhysicalKey::Code(KeyCode::F8) => Some(vec![0x1b, b'[', b'1', b'9', b'~']),
            PhysicalKey::Code(KeyCode::F9) => Some(vec![0x1b, b'[', b'2', b'0', b'~']),
            PhysicalKey::Code(KeyCode::F10) => Some(vec![0x1b, b'[', b'2', b'1', b'~']),
            PhysicalKey::Code(KeyCode::F11) => Some(vec![0x1b, b'[', b'2', b'3', b'~']),
            PhysicalKey::Code(KeyCode::F12) => Some(vec![0x1b, b'[', b'2', b'4', b'~']),

            _ => None,
        }
    }
}

impl ApplicationHandler<AppEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        log::info!("App resumed, initializing...");
        if self.state.is_none() {
            self.state = Some(AppState::init(event_loop));
        }
        if let Some(state) = &self.state {
            state.window.request_redraw();
            self.start_background_threads(state.rows(), state.cols());
        }
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        log::info!("App suspended");
        self.stop_background_threads();
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(state) = &mut self.state else {
            return;
        };

        match event {
            WindowEvent::CloseRequested => {
                log::info!("Close requested");
                self.stop_background_threads();
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                log::info!("Resized to {:?}", size);
                state.resize(size.width, size.height);
                // Notify PTY of resize
                if let Some(pty) = &self.pty {
                    pty.resize(state.rows(), state.cols());
                }
                state.window.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                state.render();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                match event.physical_key {
                    PhysicalKey::Code(KeyCode::ControlLeft)
                    | PhysicalKey::Code(KeyCode::ControlRight) => {
                        state.ctrl_pressed = event.state == ElementState::Pressed;
                    }
                    PhysicalKey::Code(KeyCode::ShiftLeft)
                    | PhysicalKey::Code(KeyCode::ShiftRight) => {
                        state.shift_pressed = event.state == ElementState::Pressed;
                    }
                    _ => {}
                }

                if event.state == ElementState::Pressed {
                    if let Some(bytes) = AppState::keycode_to_bytes(
                        &event.physical_key,
                        state.ctrl_pressed,
                        state.shift_pressed,
                    ) {
                        if let Some(pty) = &self.pty {
                            let _ = pty.write(&bytes);
                        }
                        state.reset_cursor();
                    }
                }
            }
            _ => {}
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: AppEvent) {
        let Some(state) = &mut self.state else {
            return;
        };

        match event {
            AppEvent::CursorBlink => {
                state.toggle_cursor_blink();
                state.window.request_redraw();
            }
            AppEvent::PtyOutput(data) => {
                state.process_pty_output(&data);
                state.window.request_redraw();
            }
        }
    }
}
