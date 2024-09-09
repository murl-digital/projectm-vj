use std::{
    num::NonZeroU32,
    sync::{Arc, Mutex},
    time::Instant,
};

use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    BufferSize, Device, Host, SampleFormat, Stream,
};
use egui::{ComboBox, ScrollArea};
use egui_glow::EguiGlow;
use fuzzy_matcher::skim::SkimMatcherV2;
use glutin::{
    config::{Config, ConfigTemplateBuilder, GlConfig},
    context::{ContextApi, ContextAttributesBuilder, PossiblyCurrentContext, Version},
    display::GetGlDisplay,
    prelude::{GlDisplay, NotCurrentGlContext, PossiblyCurrentGlContext},
    surface::{GlSurface, Surface, SwapInterval, WindowSurface},
};
use glutin_winit::{finalize_window, DisplayBuilder, GlWindow};
use playlist::Playlist;
use projectm::core::ProjectM;
use raw_window_handle::HasWindowHandle;
use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event::WindowEvent,
    event_loop::EventLoop,
    keyboard::{KeyCode, PhysicalKey},
    window::{Window, WindowAttributes},
};

mod playlist;

struct MainWindow {
    projectm: Arc<Mutex<ProjectM>>,
    playlist: Playlist,
    window: Window,
    gl_surface: Surface<WindowSurface>,
    frame_count: usize,
    fps: usize,
    second_counter: Instant,
}

struct ControlPanel {
    window: Window,
    gl_surface: Surface<WindowSurface>,
    gl: Arc<egui_glow::glow::Context>,
    egui_glow: EguiGlow,
    smooth_transition: bool,
}

struct VJApp {
    gl_config: Config,
    gl_context: PossiblyCurrentContext,
    main_window: MainWindow,
    control_panel: Option<ControlPanel>,
    audio_host: Host,
    input_device: Device,
    input_stream: Option<Stream>,
    preset_search: String,
}

impl ApplicationHandler for VJApp {
    fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        let egui_winit_window_builder = WindowAttributes::default()
            .with_resizable(false)
            .with_inner_size(LogicalSize {
                width: 800.0,
                height: 600.0,
            })
            .with_title("projectm_vj control panel")
            .with_visible(false);

        let egui_window =
            finalize_window(event_loop, egui_winit_window_builder, &self.gl_config).unwrap();

        let (width, height): (u32, u32) = egui_window.inner_size().into();
        let width = NonZeroU32::new(width).unwrap_or(NonZeroU32::MIN);
        let height = NonZeroU32::new(height).unwrap_or(NonZeroU32::MIN);
        let surface_attributes =
            glutin::surface::SurfaceAttributesBuilder::<glutin::surface::WindowSurface>::new()
                .build(
                    egui_window
                        .window_handle()
                        .expect("failed to get window handle")
                        .as_raw(),
                    width,
                    height,
                );

        let gl_surface = unsafe {
            self.gl_config
                .display()
                .create_window_surface(&self.gl_config, &surface_attributes)
                .unwrap()
        };

        self.gl_context.make_current(&gl_surface).unwrap();

        gl_surface
            .set_swap_interval(&self.gl_context, SwapInterval::Wait(NonZeroU32::MIN))
            .unwrap();

        #[allow(clippy::arc_with_non_send_sync)]
        let gl = Arc::new(unsafe {
            egui_glow::glow::Context::from_loader_function(|s| {
                let s = std::ffi::CString::new(s)
                    .expect("failed to construct C string from string for gl proc address");

                self.gl_config.display().get_proc_address(&s)
            })
        });

        egui_window.set_visible(true);

        let egui_glow = EguiGlow::new(event_loop, gl.clone(), None, None, true);

        self.control_panel = Some(ControlPanel {
            window: egui_window,
            gl_surface,
            gl,
            egui_glow,
            smooth_transition: false,
        });
    }

    fn window_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                self.main_window.projectm.lock().unwrap().destroy();
                event_loop.exit();
            }
            WindowEvent::Resized(size) if window_id == self.main_window.window.id() => {
                let projectm = self.main_window.projectm.lock().unwrap();
                projectm.set_window_size(size.width as usize, size.height as usize);
            }
            WindowEvent::RedrawRequested if window_id == self.main_window.window.id() => {
                self.input_stream.as_ref().inspect(|s| s.play().unwrap());
                self.gl_context
                    .make_current(&self.main_window.gl_surface)
                    .unwrap();
                let projectm = self.main_window.projectm.lock().unwrap();
                projectm.render_frame();
                self.main_window
                    .gl_surface
                    .swap_buffers(&self.gl_context)
                    .unwrap();

                self.main_window.window.request_redraw();

                self.main_window.frame_count += 1;
                let now = Instant::now();

                if now
                    .checked_duration_since(self.main_window.second_counter)
                    .is_some_and(|d| d.as_secs_f64() >= 1.0)
                {
                    self.main_window.fps = self.main_window.frame_count;
                    projectm.set_fps(self.main_window.fps as u32);
                    self.main_window.second_counter = now;
                    self.main_window.frame_count = 0;
                }
            }
            WindowEvent::RedrawRequested
                if self
                    .control_panel
                    .as_ref()
                    .is_some_and(|c| c.window.id() == window_id) =>
            {
                if let Some(control_panel) = self.control_panel.as_mut() {
                    let mut quit = false;

                    control_panel
                        .egui_glow
                        .run(&control_panel.window, |egui_ctx| {
                            egui::SidePanel::left("my_side_panel").show(egui_ctx, |ui| {
                                ui.heading("Hello World!");
                                ui.label(format!("FPS: {}", self.main_window.fps));
                                if ui.button("Quit").clicked() {
                                    quit = true;
                                }
                                ComboBox::from_label("Audio Device")
                                    .selected_text(self.input_device.name().unwrap())
                                    .show_ui(ui, |ui| {
                                        for device in self.audio_host.input_devices().unwrap() {
                                            if ui.button(device.name().unwrap()).clicked() {
                                                self.input_device = device;
                                                if let Some(stream) = self.input_stream.take() {
                                                    drop(stream);
                                                }

                                                let default_config = self
                                                    .input_device
                                                    .supported_input_configs()
                                                    .unwrap()
                                                    .find(|c| {
                                                        c.channels() == 2
                                                            && c.sample_format()
                                                                == SampleFormat::F32
                                                    })
                                                    .unwrap();

                                                let pm = self.main_window.projectm.clone();

                                                println!("{:?}", default_config.buffer_size());

                                                let mut config: cpal::StreamConfig = default_config
                                                    .with_sample_rate(cpal::SampleRate(44100))
                                                    .into();

                                                config.buffer_size = BufferSize::Fixed(512);

                                                let stream = self
                                                    .input_device
                                                    .build_input_stream(
                                                        &config,
                                                        move |data, _info| {
                                                            let pm = pm.lock().unwrap();
                                                            pm.pcm_add_float(data.to_vec(), 2);
                                                        },
                                                        |_| {},
                                                        None,
                                                    )
                                                    .unwrap();

                                                self.input_stream = Some(stream);
                                            }
                                        }
                                    });

                                ui.toggle_value(&mut control_panel.smooth_transition, "SMOOTH")
                            });

                            egui::CentralPanel::default().show(egui_ctx, |ui| {
                                ui.label(format!(
                                    "index is {}",
                                    self.main_window.playlist.current_index()
                                ));

                                ui.text_edit_singleline(&mut self.preset_search);

                                ScrollArea::vertical().show(ui, |ui| {
                                    let mut index_to_play = None;
                                    for (i, name) in
                                        self.main_window.playlist.presets().enumerate().filter(
                                            |(_, name)| {
                                                SkimMatcherV2::default()
                                                    .fuzzy(name, &self.preset_search, false)
                                                    .is_some()
                                            },
                                        )
                                    {
                                        if ui
                                            .selectable_label(
                                                i == self.main_window.playlist.current_index(),
                                                format!("{i} | {name}"),
                                            )
                                            .clicked()
                                        {
                                            index_to_play = Some(i);
                                        }
                                    }

                                    if let Some(i) = index_to_play {
                                        self.main_window.playlist.play_index(
                                            &self.main_window.projectm.lock().unwrap(),
                                            i,
                                            control_panel.smooth_transition,
                                        );
                                    }
                                });
                            });
                        });

                    if quit {
                        self.main_window.projectm.lock().unwrap().destroy();
                        event_loop.exit();
                    } else {
                        control_panel.window.request_redraw();
                        event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);
                    }

                    {
                        self.gl_context
                            .make_current(&control_panel.gl_surface)
                            .unwrap();
                        unsafe {
                            use egui_glow::glow::HasContext as _;
                            control_panel.gl.clear_color(0.2, 0.2, 0.2, 1.0);
                            control_panel.gl.clear(egui_glow::glow::COLOR_BUFFER_BIT);
                        }

                        // draw things behind egui here

                        control_panel.egui_glow.paint(&control_panel.window);

                        // draw things on top of egui here

                        control_panel
                            .gl_surface
                            .swap_buffers(&self.gl_context)
                            .unwrap();
                        control_panel.window.set_visible(true);
                    }
                }
            }
            event
                if self
                    .control_panel
                    .as_ref()
                    .is_some_and(|c| c.window.id() == window_id) =>
            {
                if let Some(control_panel) = self.control_panel.as_mut() {
                    let event_response = control_panel
                        .egui_glow
                        .on_window_event(&control_panel.window, &event);

                    if event_response.repaint {
                        control_panel.window.request_redraw();
                    }
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let projectm = self.main_window.projectm.lock().unwrap();
                if event.state.is_pressed() {
                    match event.physical_key {
                        PhysicalKey::Code(KeyCode::KeyR) => {
                            self.main_window.playlist.play_random(
                                &projectm,
                                self.control_panel
                                    .as_ref()
                                    .map_or(false, |c| c.smooth_transition),
                            );
                        }
                        PhysicalKey::Code(KeyCode::KeyT) => {
                            if let Some(c) = self.control_panel.as_mut() {
                                c.smooth_transition = !c.smooth_transition;
                            }
                        }
                        _ => (),
                    }
                }
            }
            _ => (),
        }
    }
}

fn main() {
    let event_loop = EventLoop::builder().build().unwrap();
    let window_attributes = Window::default_attributes()
        .with_title("projectm_vj")
        .with_inner_size(LogicalSize::new(1024.0, 768.0));

    let template = ConfigTemplateBuilder::new();

    let display_builder = DisplayBuilder::new().with_window_attributes(Some(window_attributes));

    let (window, gl_config) = display_builder
        .build(&event_loop, template, |configs| {
            configs
                .reduce(|accum, config| {
                    if config.num_samples() > accum.num_samples() {
                        config
                    } else {
                        accum
                    }
                })
                .unwrap()
        })
        .unwrap();

    let raw_window_handle = window
        .as_ref()
        .map(|window| window.window_handle().unwrap().as_raw());

    let gl_display = gl_config.display();
    let context_attributes = ContextAttributesBuilder::new()
        .with_context_api(ContextApi::OpenGl(Some(Version { major: 3, minor: 3 })))
        .build(raw_window_handle);

    let not_current_gl_context = unsafe {
        gl_display
            .create_context(&gl_config, &context_attributes)
            .unwrap()
    };

    let window = window.unwrap();

    let attrs = window.build_surface_attributes(Default::default()).unwrap();
    let gl_surface = unsafe {
        gl_display
            .create_window_surface(&gl_config, &attrs)
            .unwrap()
    };

    let gl_context = not_current_gl_context.make_current(&gl_surface).unwrap();

    gl_surface
        .set_swap_interval(&gl_context, SwapInterval::Wait(NonZeroU32::new(1).unwrap()))
        .unwrap();

    let pm = ProjectM::create();

    let mut playlist = Playlist::default();
    playlist.add_dir("presets".into());

    let size = window.inner_size();
    pm.set_window_size(size.width as usize, size.height as usize);
    pm.set_soft_cut_duration(0.5);

    let audio_host = cpal::default_host();
    let input_device = audio_host.default_input_device().unwrap();

    let mut app = VJApp {
        gl_context,
        gl_config,
        main_window: MainWindow {
            projectm: Arc::new(Mutex::new(pm)),
            playlist,
            window,
            gl_surface,
            frame_count: 0,
            fps: 0,
            second_counter: Instant::now(),
        },
        control_panel: None,
        audio_host,
        input_device,
        input_stream: None,
        preset_search: String::default(),
    };

    event_loop.run_app(&mut app).unwrap();
}
