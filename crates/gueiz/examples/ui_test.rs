use std::error::Error;
use std::sync::Arc;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowAttributes, WindowId};
use gueiz::gpu::camera::ScaleMode;
use gueiz::gpu::msaa::DEFAULT_MULTISAMPLE;
use gueiz::gpu::renderer::{Renderer, RendererBackend, SurfaceSize};
use gueiz::winit_gui::WinitGui;
use gueiz_2d::draw_manager::{DrawManager, DrawManagerDescriptor};
use gueiz_2d::gui::button::{Button, ButtonFont, ButtonTheme};
use gueiz_2d::gui::check_box::{Axis, CheckBox, CheckBoxFont, CheckBoxShape, CheckBoxTheme};
use gueiz_2d::gui::color::{Color, ColorFont, ColorTheme, Rgba};
use gueiz_2d::gui::dropdown::{Dropdown, DropdownFont, DropdownTheme};
use gueiz_2d::gui::radio::{Radio, RadioFont, RadioShape, RadioTheme};
use gueiz_2d::gui::context_menu::{ContextMenuFont, ContextMenuLabel, ContextMenuLabels, ContextMenuLayout, ContextMenuSlicer, ContextMenuStyle, ContextMenuTheme, INPUT_TEXT_CONTEXT_MENU};
use gueiz_2d::gui::image_label::{ImageLabel, ImageLabelTexture, ImageLabelTheme};
use gueiz_2d::gui::input_text::{InputText, InputTextFont, InputTextTheme};
use gueiz_2d::gui::input_textarea::InputTextarea;
use gueiz_2d::gui::label::{Label, LabelFont, LabelTheme};
use gueiz_2d::gui::number::{Number, NumberFont, NumberTheme};
use gueiz_2d::gui::range::{Range, RangeTheme};
use gueiz_2d::gui::window_frame::{Gap, Quad, QuadType, ThemeColor, WindowFont, WindowFrame, WindowTheme};
use gueiz_2d::resource::Resources;

fn main() -> Result<(), Box<dyn Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let renderer_backend = match std::env::args().nth(1) {
        Some(argument) => argument.parse::<RendererBackend>()?,
        None => RendererBackend::Auto,
    };

    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Wait);
    event_loop.run_app(Application::new(renderer_backend))?;

    Ok(())
}

struct Application {
    renderer: Renderer,
    window: Option<Arc<dyn Window>>,
    scene: Option<Scene>,
    /// winit の出来事を gui へ渡し、指の形と IME を窓に映す。
    winit_gui: WinitGui,
}

impl Application {
    fn new(renderer_backend: RendererBackend) -> Self {
        let mut renderer = Renderer::new(renderer_backend);
        renderer.set_sample_count(DEFAULT_MULTISAMPLE);

        Self {
            renderer,
            window: None,
            scene: None,
            winit_gui: WinitGui::new(),
        }
    }

    fn request_redraw(&self) {
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }

    fn draw_frame(&mut self) {
        let Some(scene) = self.scene.as_mut() else {
            return;
        };

        // 指で動かしたぶんを、描く前に 1 回だけ組み直す。
        scene.window_frame.update(&mut scene.draw_manager, &scene.resources);

        if let Err(error) = scene.draw_manager.prepare() {
            log::error!("failed to prepare the frame: {error}");
            return;
        }

        self.renderer.render(wgpu::Color{
            r: 0.13,
            g: 0.14,
            b: 0.16,
            a: 1.0,
        }, |render_pass| {
            scene.draw_manager.draw(render_pass);
        });
    }
}

impl ApplicationHandler for Application {
    fn can_create_surfaces(&mut self, event_loop: &dyn ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let attributes = WindowAttributes::default()
            .with_title("gueiz 2d")
            .with_surface_size(LogicalSize::new(640.0, 480.0));

        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::<dyn Window>::from(window),
            Err(error) => {
                log::error!("failed to create the window: {error}");
                event_loop.exit();
                return;
            }
        };

        let size = window.surface_size();
        let surface_size = SurfaceSize::new(size.width, size.height);
        let scale_factor = window.scale_factor() as f32;

        log::info!("surface {}x{} (scale {scale_factor})", size.width, size.height);

        if let Err(error) = self.renderer.create_surface(window.clone(), surface_size) {
            log::error!("failed to create the surface: {error}");
            event_loop.exit();
            return;
        }

        match Scene::new(&self.renderer, surface_size, scale_factor) {
            Ok(scene) => self.scene = Some(scene),
            Err(error) => {
                log::error!("failed to build the scene: {error}");
                event_loop.exit();
                return;
            }
        }

        self.window = Some(window);
    }

    fn window_event(&mut self, event_loop: &dyn ActiveEventLoop, _window_id: WindowId, event: WindowEvent) {
        match &event {
            WindowEvent::CloseRequested => event_loop.exit(),

            WindowEvent::SurfaceResized(size) => {
                let surface_size = SurfaceSize::new(size.width, size.height);
                self.renderer.resize(surface_size);

                if let Some(scene) = self.scene.as_mut() {
                    scene.resize(surface_size);
                }
            }

            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                let Some(window) = self.window.as_ref() else {
                    return;
                };

                let size = window.surface_size();
                let surface_size = SurfaceSize::new(size.width, size.height);

                self.renderer.resize(surface_size);

                if let Some(scene) = self.scene.as_mut() {
                    scene.rescale(surface_size, *scale_factor as f32);
                }

                window.request_redraw();
            }

            WindowEvent::RedrawRequested => self.draw_frame(),

            // macOS では最初の描画が窓の出る前に来て、`Occluded` で飛ばされる。
            // 常時回していないので、見えるようになったら描き直しを頼む。
            WindowEvent::Occluded(false) => self.request_redraw(),

            _ => {}
        }

        // 指とキーは gui へ。指の形と IME もここで窓に映る。
        let (Some(window), Some(scene)) = (self.window.as_ref(), self.scene.as_mut()) else {
            return;
        };

        let response = self.winit_gui.window_event(
            window.as_ref(),
            &event,
            &mut scene.window_frame,
            &mut scene.draw_manager,
            &scene.resources,
        );

        // 常時回していないので、gui の見た目が変わったら描き直しを頼む。
        if response.redraw {
            window.request_redraw();
        }
    }

    // キャレットの点滅など、gui が時間で動くあいだは、その時刻に起きて描き直す。
    fn about_to_wait(&mut self, event_loop: &dyn ActiveEventLoop) {
        if let (Some(window), Some(scene)) = (self.window.as_ref(), self.scene.as_ref()) {
            self.winit_gui.about_to_wait(event_loop, window.as_ref(), &scene.window_frame);
        }
    }

    fn destroy_surfaces(&mut self, _event_loop: &dyn ActiveEventLoop) {
        self.scene = None;
        self.renderer.destroy_surface();
        self.window = None;
    }
}

struct Scene {
    draw_manager: DrawManager,
    /// 窓が使う書体などを預かる。窓は取っ手だけを持つ。
    resources: Resources,
    window_frame: WindowFrame,
    scale_factor: f32,
}

impl Scene {
    fn new(
        renderer: &Renderer,
        surface_size: SurfaceSize,
        scale_factor: f32,
    ) -> Result<Self, Box<dyn Error>> {
        let (Some(device), Some(queue), Some(format)) = (
            renderer.device(),
            renderer.queue(),
            renderer.surface_format(),
        ) else {
            return Err("the renderer has no surface yet".into());
        };

        let descriptor = DrawManagerDescriptor {
            sample_count: renderer.sample_count(),
            ..DrawManagerDescriptor::default()
        };

        let mut draw_manager = DrawManager::new(device, queue, format, &descriptor)?;

        let font_path = if cfg!(target_os = "windows") {
            "C:/Windows/Fonts/YuGothM.ttc".to_string()
        } else {
            "/System/Library/Fonts/AquaKana.ttc".to_string()
        };

        let mut resources = Resources::new();
        let font_handle = resources.load_font_file("font", font_path)?;

        let image = image::open("images/image1.png")?.into_rgba8();
        let (image_width, image_height) = image.dimensions();
        let texture_handle = resources.load_texture("image", image_width, image_height, image.as_raw())?;
        resources.commit_textures(device, queue, &mut draw_manager)?;

        let mut window_frame = WindowFrame::new(
            "TestWindowObject1".to_string(),
            ScaleMode::Stretch,
            "TestWindow1".to_string(),
            Quad {
                quad_type: QuadType::Rounded {radius: 10.0},
                x: 100.0,
                y: 100.0,
                width: 500.0,
                // 複数行の欄まで入るように。
                height: 360.0,
            },
            30.0,
            Gap {
                x: 5.0,
                y: 5.0,
            },
            WindowFont {
                base_font: font_handle,
                base_font_size: 13.0,
            },
            WindowTheme {
                frame_fill_color: ThemeColor {
                    r: 0.1,
                    g: 0.1,
                    b: 0.1,
                    a: 1.0,
                },
                frame_outline_color: ThemeColor {
                    r: 0.5,
                    g: 0.5,
                    b: 0.5,
                    a: 1.0,
                },
                title_bar_fill_color: ThemeColor {
                    r: 0.1,
                    g: 0.1,
                    b: 0.1,
                    a: 1.0,
                },
                title_bar_slice_line_color: ThemeColor {
                    r: 0.24,
                    g: 0.24,
                    b: 0.24,
                    a: 1.0,
                },
                text_title_color: ThemeColor {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 1.0,
                }
            }
        );
        window_frame.add_window_item(
            Box::new(
                Label::new(
                    "TextLabelObject1".to_string(),
                    "TestLabel1".to_string(),
                    32.0,
                    Gap {
                        x: 0.0,
                        y: 0.0,
                    },
                    LabelFont {
                        base_font: font_handle,
                    },
                    LabelTheme {
                        label_color: ThemeColor {
                            r: 1.0,
                            g: 1.0,
                            b: 1.0,
                            a: 1.0,
                        },
                    }
                )
            )
        );
        // パレットは下の部品の上に重なって開く。窓に収まるよう、いちばん上のほうに置く。
        window_frame.add_window_item(Box::new(
            Color::new(
                "ColorObject1".to_string(),
                60.0,
                28.0,
                Gap {
                    x: 0.0,
                    y: 5.0,
                },
                ColorFont {
                    base_font: font_handle,
                    base_font_size: 13.0,
                },
                ColorTheme {
                    swatch_outline_color: ThemeColor {
                        r: 0.35,
                        g: 0.35,
                        b: 0.38,
                        a: 1.0,
                    },
                    focus_outline_color: ThemeColor {
                        r: 0.36,
                        g: 0.56,
                        b: 0.92,
                        a: 1.0,
                    },
                    panel_fill_color: ThemeColor {
                        r: 0.12,
                        g: 0.12,
                        b: 0.14,
                        a: 1.0,
                    },
                    panel_outline_color: ThemeColor {
                        r: 0.35,
                        g: 0.35,
                        b: 0.38,
                        a: 1.0,
                    },
                    marker_color: ThemeColor {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: 1.0,
                    },
                    marker_shadow_color: ThemeColor {
                        r: 0.0,
                        g: 0.0,
                        b: 0.0,
                        a: 0.6,
                    },
                    field_fill_color: ThemeColor {
                        r: 0.06,
                        g: 0.06,
                        b: 0.07,
                        a: 1.0,
                    },
                    field_outline_color: ThemeColor {
                        r: 0.35,
                        g: 0.35,
                        b: 0.38,
                        a: 1.0,
                    },
                    label_color: ThemeColor {
                        r: 0.65,
                        g: 0.65,
                        b: 0.68,
                        a: 1.0,
                    },
                    text_color: ThemeColor {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: 1.0,
                    },
                },
            )
            .value(Rgba::from_hex("#4a90e2").unwrap_or_default())
            .on_input(move |value| {
                log::info!("the color was picked: {}", value.to_hex());
            })
            .on_change(move |value| {
                log::info!("the color was changed: {}", value.to_hex());
            }),
        ));
        let mut click_count = 0;
        window_frame.add_window_item(
            Box::new(
                Button::new(
                    "ButtonObject1".to_string(),
                    "Click me".to_string(),
                    120.0,
                    32.0,
                    QuadType::Rounded { radius: 6.0 },
                    Gap {
                        x: 0.0,
                        y: 5.0,
                    },
                    ButtonFont {
                        base_font: font_handle,
                        base_font_size: 14.0,
                    },
                    ButtonTheme {
                        fill_color: ThemeColor {
                            r: 0.22,
                            g: 0.36,
                            b: 0.62,
                            a: 1.0,
                        },
                        hover_fill_color: ThemeColor {
                            r: 0.28,
                            g: 0.44,
                            b: 0.74,
                            a: 1.0,
                        },
                        pressed_fill_color: ThemeColor {
                            r: 0.16,
                            g: 0.27,
                            b: 0.48,
                            a: 1.0,
                        },
                        text_color: ThemeColor {
                            r: 1.0,
                            g: 1.0,
                            b: 1.0,
                            a: 1.0,
                        },
                    }
                )
                .on_click(move || {
                    click_count += 1;
                    log::info!("the button was clicked ({click_count})");
                })
            )
        );
        // 一覧は下の部品の上に重なって開く。窓に収まるよう、見せる行を 5 行にする。
        window_frame.add_window_item(Box::new(
            Dropdown::new(
                "DropdownObject1".to_string(),
                200.0,
                28.0,
                QuadType::Rounded { radius: 4.0 },
                Gap {
                    x: 0.0,
                    y: 5.0,
                },
                DropdownFont {
                    base_font: font_handle,
                    base_font_size: 14.0,
                },
                DropdownTheme {
                    fill_color: ThemeColor {
                        r: 0.06,
                        g: 0.06,
                        b: 0.07,
                        a: 1.0,
                    },
                    outline_color: ThemeColor {
                        r: 0.35,
                        g: 0.35,
                        b: 0.38,
                        a: 1.0,
                    },
                    focus_outline_color: ThemeColor {
                        r: 0.36,
                        g: 0.56,
                        b: 0.92,
                        a: 1.0,
                    },
                    text_color: ThemeColor {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: 1.0,
                    },
                    arrow_color: ThemeColor {
                        r: 0.75,
                        g: 0.75,
                        b: 0.78,
                        a: 1.0,
                    },
                    list_fill_color: ThemeColor {
                        r: 0.12,
                        g: 0.12,
                        b: 0.14,
                        a: 1.0,
                    },
                    list_outline_color: ThemeColor {
                        r: 0.35,
                        g: 0.35,
                        b: 0.38,
                        a: 1.0,
                    },
                    highlight_fill_color: ThemeColor {
                        r: 0.24,
                        g: 0.40,
                        b: 0.72,
                        a: 1.0,
                    },
                    highlight_text_color: ThemeColor {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: 1.0,
                    },
                    scrollbar_color: ThemeColor {
                        r: 0.45,
                        g: 0.45,
                        b: 0.48,
                        a: 1.0,
                    },
                },
            )
            .options(["Apple", "Banana", "Blueberry", "Cherry", "Date", "Elderberry", "Fig", "Grape", "Kiwi", "Lemon"])
            .max_visible(5)
            .on_change(move |index, option| {
                log::info!("the dropdown was changed: {index} ({option})");
            }),
        ));
        let check_box_theme = || CheckBoxTheme {
            outline_color: ThemeColor { r: 0.8, g: 0.8, b: 0.8, a: 1.0 },
            enable_color: ThemeColor { r: 0.28, g: 0.44, b: 0.74, a: 1.0 },
            disable_color: ThemeColor { r: 0.0, g: 0.0, b: 0.0, a: 0.0 },
            text_color: ThemeColor { r: 1.0, g: 1.0, b: 1.0, a: 1.0 },
        };
        let mut check_box = CheckBox::new(
            "CheckBoxObject1".to_string(),
            Axis::Row,
            Gap {
                x: 0.0,
                y: 5.0,
            },
            CheckBoxFont {
                base_font: font_handle,
                base_font_size: 13.0,
            },
        )
        .on_click(move |check_boxes, changed, enables, disables| {
            log::info!("the check box {changed:?} was clicked: enables {enables:?}, disables {disables:?}");
        });
        check_box
            .add_check_box_item("check_box1".to_string(), 20.0, 20.0, Gap { x: 0.0, y: 0.0 }, 4.0, CheckBoxShape::Circle, false, check_box_theme())
            .add_check_box_item("check_box2".to_string(), 20.0, 20.0, Gap { x: 12.0, y: 0.0 }, 4.0, CheckBoxShape::Rect, true, check_box_theme())
            .add_check_box_item("check_box3".to_string(), 20.0, 20.0, Gap { x: 12.0, y: 0.0 }, 4.0, CheckBoxShape::RectRounded, false, check_box_theme());
        window_frame.add_window_item(Box::new(check_box));
        let radio_theme = || RadioTheme {
            outline_color: ThemeColor { r: 0.8, g: 0.8, b: 0.8, a: 1.0 },
            enable_color: ThemeColor { r: 0.28, g: 0.44, b: 0.74, a: 1.0 },
            disable_color: ThemeColor { r: 0.0, g: 0.0, b: 0.0, a: 0.0 },
            text_color: ThemeColor { r: 1.0, g: 1.0, b: 1.0, a: 1.0 },
        };
        let mut radio = Radio::new(
            "RadioObject1".to_string(),
            Axis::Column,
            Gap {
                x: 0.0,
                y: 10.0,
            },
            RadioFont {
                base_font: font_handle,
                base_font_size: 13.0,
            },
        )
        .on_click(move |_radios, selected, previous| {
            log::info!("the radio {selected} was selected (previous: {previous:?})");
        });
        radio
            .add_radio_item("radio1".to_string(), 20.0, 20.0, Gap { x: 0.0, y: 0.0 }, 5.0, RadioShape::Circle, true, radio_theme())
            .add_radio_item("radio2".to_string(), 20.0, 20.0, Gap { x: 0.0, y: 12.0 }, 5.0, RadioShape::Circle, false, radio_theme())
            .add_radio_item("radio3".to_string(), 20.0, 20.0, Gap { x: 0.0, y: 12.0 }, 5.0, RadioShape::Circle, false, radio_theme());
        window_frame.add_window_item(Box::new(radio));

        let input_text_theme = InputTextTheme {
            fill_color: ThemeColor {
                r: 0.06,
                g: 0.06,
                b: 0.07,
                a: 1.0,
            },
            outline_color: ThemeColor {
                r: 0.35,
                g: 0.35,
                b: 0.38,
                a: 1.0,
            },
            focus_outline_color: ThemeColor {
                r: 0.36,
                g: 0.56,
                b: 0.92,
                a: 1.0,
            },
            text_color: ThemeColor {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0,
            },
            placeholder_color: ThemeColor {
                r: 0.5,
                g: 0.5,
                b: 0.52,
                a: 1.0,
            },
            caret_color: ThemeColor {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0,
            },
            selection_color: ThemeColor {
                r: 0.24,
                g: 0.40,
                b: 0.72,
                a: 1.0,
            },
        };
        window_frame.add_window_item(Box::new(
            Number::<i32>::new(
                "Number".to_string(),
                "number".to_string(),
                200.0,
                28.0,
                8.0,
                QuadType::Rounded {
                    radius: 4.0
                },
                Gap {
                    x: 0.0,
                    y: 5.0
                },
                NumberFont {
                    base_font: font_handle,
                    base_font_size: 14.0
                },
                NumberTheme {
                    input_text_theme,
                    spin_fill_color: ThemeColor {
                        r: 0.15,
                        g: 0.15,
                        b: 0.15,
                        a: 1.0,
                    },
                    spin_hover_fill_color: ThemeColor {
                        r: 0.25,
                        g: 0.25,
                        b: 0.25,
                        a: 1.0,
                    },
                    spin_pressed_fill_color: ThemeColor {
                        r: 0.35,
                        g: 0.35,
                        b: 0.35,
                        a: 1.0,
                    },
                    arrow_color: ThemeColor {
                        r: 0.9,
                        g: 0.9,
                        b: 0.9,
                        a: 1.0,
                    },
                },
            )
            .on_input(move |value| {
                log::info!("the number was typed: {value:?}");
            })
            .on_change(move |value| {
                log::info!("the number was changed: {value:?}");
            }),
        ));
        window_frame.add_window_item(Box::new(
            Range::<i32>::new(
                "Range".to_string(),
                240.0,
                20.0,
                Gap {
                    x: 0.0,
                    y: 10.0,
                },
                RangeTheme {
                    track_color: ThemeColor {
                        r: 0.25,
                        g: 0.25,
                        b: 0.28,
                        a: 1.0,
                    },
                    fill_color: ThemeColor {
                        r: 0.28,
                        g: 0.44,
                        b: 0.74,
                        a: 1.0,
                    },
                    thumb_color: ThemeColor {
                        r: 0.85,
                        g: 0.85,
                        b: 0.88,
                        a: 1.0,
                    },
                    thumb_hover_color: ThemeColor {
                        r: 0.95,
                        g: 0.95,
                        b: 0.97,
                        a: 1.0,
                    },
                    thumb_pressed_color: ThemeColor {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: 1.0,
                    },
                    focus_ring_color: ThemeColor {
                        r: 0.36,
                        g: 0.56,
                        b: 0.92,
                        a: 1.0,
                    },
                },
            )
            .min(0)
            .max(100)
            .step(1)
            .value(50)
            .on_input(move |value| {
                log::info!("the range was moved: {value}");
            })
            .on_change(move |value| {
                log::info!("the range was changed: {value}");
            }),
        ));
        window_frame.add_window_item(
            Box::new(
                InputText::new(
                    "InputTextObject1".to_string(),
                    "Type here...".to_string(),
                    240.0,
                    28.0,
                    8.0,
                    QuadType::Rounded { radius: 4.0 },
                    Gap {
                        x: 0.0,
                        y: 5.0,
                    },
                    InputTextFont {
                        base_font: font_handle,
                        base_font_size: 14.0,
                    },
                    input_text_theme,
                )
                .on_submit(|text| log::info!("submitted: {text}"))
            )
        );
        window_frame.add_window_item(
            Box::new(
                InputText::new(
                    "PasswordObject1".to_string(),
                    "Password".to_string(),
                    240.0,
                    28.0,
                    8.0,
                    QuadType::Rounded { radius: 4.0 },
                    Gap {
                        x: 0.0,
                        y: 5.0,
                    },
                    InputTextFont {
                        base_font: font_handle,
                        base_font_size: 14.0,
                    },
                    input_text_theme,
                )
                .password()
                .on_submit(|text| log::info!("password submitted ({} characters)", text.chars().count()))
            )
        );
        window_frame.add_window_item(
            Box::new(
                InputTextarea::new(
                    "TextareaObject1".to_string(),
                    "Write something...\n(Ctrl+Enter to submit)".to_string(),
                    320.0,
                    110.0,
                    8.0,
                    QuadType::Rounded { radius: 4.0 },
                    Gap {
                        x: 0.0,
                        y: 5.0,
                    },
                    InputTextFont {
                        base_font: font_handle,
                        base_font_size: 14.0,
                    },
                    InputTextTheme {
                        fill_color: ThemeColor {
                            r: 0.06,
                            g: 0.06,
                            b: 0.07,
                            a: 1.0,
                        },
                        outline_color: ThemeColor {
                            r: 0.35,
                            g: 0.35,
                            b: 0.38,
                            a: 1.0,
                        },
                        focus_outline_color: ThemeColor {
                            r: 0.36,
                            g: 0.56,
                            b: 0.92,
                            a: 1.0,
                        },
                        text_color: ThemeColor {
                            r: 1.0,
                            g: 1.0,
                            b: 1.0,
                            a: 1.0,
                        },
                        placeholder_color: ThemeColor {
                            r: 0.5,
                            g: 0.5,
                            b: 0.52,
                            a: 1.0,
                        },
                        caret_color: ThemeColor {
                            r: 1.0,
                            g: 1.0,
                            b: 1.0,
                            a: 1.0,
                        },
                        selection_color: ThemeColor {
                            r: 0.24,
                            g: 0.40,
                            b: 0.72,
                            a: 1.0,
                        },
                    }
                )
                .on_submit(|text| log::info!("textarea submitted ({} lines)", text.lines().count()))
            )
        );
        window_frame.add_window_item(
            Box::new(
                ImageLabel::new(
                    "ImageLabelObject1".to_string(),
                    image_width as f32,
                    image_height as f32,
                    Quad {
                        quad_type: QuadType::Rounded { radius: 6.0 },
                        x: 0.0,
                        y: 0.0,
                        width: 1.0,
                        height: 1.0,
                    },
                    Gap {
                        x: 0.0,
                        y: 5.0,
                    },
                    ImageLabelTheme {
                        texture_color: ThemeColor {
                            r: 1.0,
                            g: 1.0,
                            b: 1.0,
                            a: 1.0,
                        }
                    },
                    ImageLabelTexture {
                        texture_handle,
                    }
                )
            )
        );
        // 右クリックのメニュー。足さなければ、英語のいつものもの（ContextMenuLabels::text_input）になる。
        // 1 つの項目に要るもの（字・右の列・選べる条件・押したときにすること・右に開くメニュー）は、
        // すべてその項目に書く。
        let slicer = ContextMenuSlicer {
            color: ThemeColor {
                r: 0.32,
                g: 0.33,
                b: 0.36,
                a: 1.0,
            },
        };

        let mut case_menu = ContextMenuLabels::new("CaseMenu");
        case_menu
            .add_context_menu_item(
                ContextMenuLabel::new("大文字に")
                    .enabled(|state| state.has_selection && state.editable)
                    .on_event(|event| {
                        if let Some(text) = event.selected_text() {
                            event.replace_selection(&text.to_uppercase());
                        }
                    }),
            )
            .add_context_menu_item(
                ContextMenuLabel::new("小文字に")
                    .enabled(|state| state.has_selection && state.editable)
                    .on_event(|event| {
                        if let Some(text) = event.selected_text() {
                            event.replace_selection(&text.to_lowercase());
                        }
                    }),
            );

        let mut input_text_menu = ContextMenuLabels::new(INPUT_TEXT_CONTEXT_MENU);
        input_text_menu
            .add_context_menu_item(ContextMenuLabel::cut("切り取り"))
            .add_context_menu_item(ContextMenuLabel::copy("コピー"))
            .add_context_menu_item(ContextMenuLabel::paste("貼り付け"))
            .add_context_menu_item(slicer)
            .add_context_menu_item(ContextMenuLabel::select_all("すべて選択"))
            .add_context_menu_item(slicer)
            .add_context_menu_item(
                // 選んでいないときは消す（薄くするのが既定）。
                ContextMenuLabel::new("大文字・小文字")
                    .sub_menu(case_menu)
                    .enabled(|state| state.has_selection && state.editable)
                    .hide_when_disabled(),
            );
        window_frame.add_context_menu_labels(input_text_menu);
        // 右クリックのメニューの見た目。決めなければ窓のテーマから作る。
        window_frame.set_context_menu_style(ContextMenuStyle {
            font: ContextMenuFont {
                base_font: font_handle,
                base_font_size: 13.0,
            },
            theme: ContextMenuTheme {
                fill_color: ThemeColor {
                    r: 0.16,
                    g: 0.17,
                    b: 0.19,
                    a: 1.0,
                },
                outline_color: ThemeColor {
                    r: 0.32,
                    g: 0.33,
                    b: 0.36,
                    a: 1.0,
                },
                text_color: ThemeColor {
                    r: 0.95,
                    g: 0.95,
                    b: 0.95,
                    a: 1.0,
                },
                sub_title_color: ThemeColor {
                    r: 0.62,
                    g: 0.63,
                    b: 0.66,
                    a: 1.0,
                },
                highlight_color: ThemeColor {
                    r: 0.24,
                    g: 0.40,
                    b: 0.72,
                    a: 1.0,
                },
                shadow_color: ThemeColor {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: 0.0,
                },
                disabled_alpha: 0.35,
            },
            layout: ContextMenuLayout {
                quad_type: QuadType::Rounded { radius: 6.0 },
                padding: [14.0, 4.0],
                item_height: Some(26.0),
                highlight_inset: 4.0,
                highlight_radius: 4.0,
                shadow_offset: Some([0.0, 0.0]),
                ..ContextMenuLayout::default()
            },
        });
        window_frame.create_object(
            &mut draw_manager,
            &resources,
            surface_size,
            surface_size.to_logical(scale_factor),
        );

        Ok(Self {
            draw_manager,
            resources,
            window_frame,
            scale_factor,
        })
    }

    fn resize(&mut self, surface_size: SurfaceSize) {
        self.window_frame.resize(
            &mut self.draw_manager,
            surface_size,
            surface_size.to_logical(self.scale_factor),
        );
    }

    fn rescale(&mut self, surface_size: SurfaceSize, scale_factor: f32) {
        self.scale_factor = scale_factor;
        self.resize(surface_size);
    }
}