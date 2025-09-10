use std::{
    num::NonZeroU32,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::Result;
use glutin::surface::GlSurface;
use tokio::sync::mpsc;
use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event::{KeyEvent, WindowEvent},
    event_loop::EventLoop,
    keyboard::{KeyCode, PhysicalKey},
    window::Window,
};

use super::StreamFrame;
use crate::shared::{OpenGLRenderer, setup_opengl_context};

const WINDOW_INITIAL_SIZE: (u32, u32) = (1280, 720);
const TARGET_FPS: u64 = 60;

struct GuiWindow {
    window: Option<Arc<Window>>,
    frame_rx: mpsc::Receiver<StreamFrame>,
    current_frame: Option<StreamFrame>,
    gl_context: Option<glutin::context::PossiblyCurrentContext>,
    gl_surface: Option<glutin::surface::Surface<glutin::surface::WindowSurface>>,
    renderer: Option<OpenGLRenderer>,
    is_fullscreen: bool,
    last_frame_time: Instant,
    frame_duration: Duration,
}

impl GuiWindow {
    fn new(frame_rx: mpsc::Receiver<StreamFrame>) -> Self {
        Self {
            window: None,
            frame_rx,
            current_frame: None,
            gl_context: None,
            gl_surface: None,
            renderer: None,
            is_fullscreen: false,
            last_frame_time: Instant::now(),
            frame_duration: Duration::from_millis(1000 / TARGET_FPS),
        }
    }

    fn toggle_fullscreen(&mut self) {
        if let Some(window) = &self.window {
            if self.is_fullscreen {
                window.set_fullscreen(None);
            } else {
                window.set_fullscreen(Some(winit::window::Fullscreen::Borderless(None)));
            }
            self.is_fullscreen = !self.is_fullscreen;
        }
    }

    fn should_render(&mut self) -> bool {
        let now = Instant::now();
        if now.duration_since(self.last_frame_time) >= self.frame_duration {
            self.last_frame_time = now;
            true
        } else {
            false
        }
    }
}

impl ApplicationHandler for GuiWindow {
    fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("Video Stream")
                        .with_active(true)
                        .with_resizable(true)
                        .with_inner_size(LogicalSize::new(
                            WINDOW_INITIAL_SIZE.0,
                            WINDOW_INITIAL_SIZE.1,
                        ))
                        .with_window_level(winit::window::WindowLevel::AlwaysOnTop)
                        .with_decorations(true)
                        .with_visible(true),
                )
                .unwrap(),
        );

        // Initialize OpenGL context
        let (gl_context, gl_surface) = setup_opengl_context(window.clone());

        self.window = Some(window.clone());
        self.gl_context = Some(gl_context);
        self.gl_surface = Some(gl_surface);
        self.renderer = Some(OpenGLRenderer::new().unwrap());

        window.request_redraw();

        println!("GUI window created. Press F11 to toggle fullscreen.");
    }

    fn window_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: winit::event::WindowEvent,
    ) {
        // poll latest frames
        if let Ok(frame) = self.frame_rx.try_recv() {
            self.current_frame = Some(frame);
        }

        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                if let (Some(gl_surface), Some(gl_context)) = (&self.gl_surface, &self.gl_context) {
                    gl_surface.resize(
                        gl_context,
                        NonZeroU32::new(size.width).unwrap_or(NonZeroU32::new(1).unwrap()),
                        NonZeroU32::new(size.height).unwrap_or(NonZeroU32::new(1).unwrap()),
                    );
                }
            }
            WindowEvent::RedrawRequested => {
                if self.should_render() {
                    if let (
                        Some(window),
                        Some(frame),
                        Some(renderer),
                        Some(gl_context),
                        Some(gl_surface),
                    ) = (
                        &self.window,
                        &self.current_frame,
                        &mut self.renderer,
                        &self.gl_context,
                        &self.gl_surface,
                    ) {
                        // update texture with new frame data
                        renderer.update_texture(&frame.data, frame.width, frame.height);

                        let window_size = window.inner_size();

                        renderer.render(window_size.width, window_size.height);

                        if let Err(err) = gl_surface.swap_buffers(gl_context) {
                            eprintln!("Failed to swap buffers: {}", err);
                        }
                    }
                }

                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        physical_key: PhysicalKey::Code(KeyCode::F11),
                        state: winit::event::ElementState::Pressed,
                        ..
                    },
                ..
            } => {
                self.toggle_fullscreen();
            }
            _ => (),
        }
    }
}

pub fn run_gui(frame_rx: mpsc::Receiver<StreamFrame>) -> Result<()> {
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);
    let mut gui_window = GuiWindow::new(frame_rx);
    let _ = event_loop.run_app(&mut gui_window);
    Ok(())
}
