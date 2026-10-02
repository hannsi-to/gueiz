//! `gueiz_2d::gui` の部品を**全部並べて、本当に出ているか目で見る**例。
//!
//! 単体テストは「命令が正しく並んだか」までしか見ません。実際に画面に出て、
//! 押せて、打てて、送れるかは動かさないと分かりません。これはそのための例です。
//!
//! ```sh
//! cargo run -p gueiz --example ui_item
//! ```
//!
//! # 置いてあるもの
//!
//! | 部品 | 触り方 |
//! |---|---|
//! | [`Button`] | 押す。乗る・押す・焦点で色が変わる。無効なものは反応しない |
//! | [`TextField`] | 打つ。矢印・Shift+矢印・ドラッグで選ぶ。隠す欄も並べてある |
//! | [`Dropdown`] | 押して開く。矢印と Enter でも選べる |
//! | [`ScrollArea`] | 車輪で送る。つまみを掴んで引く。**入れ子**にしてある |
//! | [`Stack`] | [`Length::Grow`] で余りを 1 : 2 : 1 に分けている |
//! | [`WindowFrame`] | 題名の帯を掴んで動かす |
//!
//! 押した結果は端末に出ます。**画面と端末の両方で確かめられます。**
//!
//! # 確かめどころ
//!
//! - **送り箱の中のドロップダウンを開く。** 一覧が箱の外へ突き抜けて、
//!   後ろのボタンより手前に出ること（浮かせる層）
//! - 開いたまま車輪で送る。一覧が消えること（持ち主が見えなくなったら浮かない）
//! - 入れ子の送り箱を端まで送る。そこから先は**外側**が送られること
//! - Tab で焦点が順に回ること
//!
//! # 書体
//!
//! 仮名を持つ書体を先に探します。見つからなければ Arial に落ちますが、
//! **Arial は仮名を持たないので日本語の札が消えます**（[`gueiz_2d::text`] は
//! 持っていない字を黙って飛ばします）。端末にどれを読んだか出ます。
//!
//! [`Button`]: gueiz_2d::gui::button::Button
//! [`TextField`]: gueiz_2d::gui::text_field::TextField
//! [`Dropdown`]: gueiz_2d::gui::dropdown::Dropdown
//! [`ScrollArea`]: gueiz_2d::gui::scroll_area::ScrollArea
//! [`Stack`]: gueiz_2d::gui::container::Stack
//! [`Length::Grow`]: gueiz_2d::gui::layout::Length::Grow
//! [`WindowFrame`]: gueiz_2d::gui::window_frame::WindowFrame

use std::error::Error;
use std::sync::Arc;

use gueiz_2d::camera::Camera;
use gueiz_2d::draw_manager::{DrawManager, DrawManagerDescriptor};
use gueiz_2d::font::Font;
use gueiz_2d::gui::button::Button;
use gueiz_2d::gui::color::Color;
use gueiz_2d::gui::container::{CrossAlign, Floating, Stack};
use gueiz_2d::gui::context::FontMeasure;
use gueiz_2d::gui::dropdown::Dropdown;
use gueiz_2d::gui::event::{
    InputEvent, Key as GuiKey, Modifiers as GuiModifiers, NamedKey as GuiNamedKey, PointerButton,
    ScrollDelta,
};
use gueiz_2d::gui::geometry::{Align, Corners, Insets, Point, Size};
use gueiz_2d::gui::id::{Tag, WidgetId};
use gueiz_2d::gui::layout::{Constraints, Length};
use gueiz_2d::gui::painter::{Painter, TextLayoutOptions};
use gueiz_2d::gui::render::GuiRenderer;
use gueiz_2d::gui::scroll_area::ScrollArea;
use gueiz_2d::gui::text_field::TextField;
use gueiz_2d::gui::theme::{Metrics, Role, StateKey, Style, Theme};
use gueiz_2d::gui::widget::{ActionKind, Behavior};
use gueiz_2d::gui::window_frame::WindowFrame;
use gueiz_2d::gui::{Gui, MeasureContext, PaintContext, Widget};
use gueiz_2d::renderer::{Renderer, RendererBackend, SurfaceSize};
use gueiz_2d::wgpu;
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle,
};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ButtonSource, ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key as WinitKey, NamedKey as WinitNamedKey};
use winit::window::{Window, WindowAttributes, WindowId};

/// 見出し用の役。組み込みとぶつからない番号から取る。
const HEADING: Role = Role::custom(0);

/// 全体を送る箱。`--shot-scroll` で撮る位置を決めるのに引く。
const PAGE: Tag = Tag::new("page");

/// 札を付けておく部品。押された先が分かるように端末へ名前を出す。
const TAGS: &[(Tag, &str)] = &[
    (Tag::new("ok"), "OK ボタン"),
    (Tag::new("cancel"), "やめるボタン"),
    (Tag::new("name"), "名前の欄"),
    (Tag::new("password"), "合言葉の欄"),
    (Tag::new("quality"), "画質のドロップダウン"),
];

fn main() -> Result<(), Box<dyn Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let font = load_font().ok_or("仮名を持つ書体も Arial も見つかりませんでした")?;

    println!("gueiz GUI の部品を並べます。閉じるか Esc で終わります。\n");
    println!("  ・ボタンを押す / 欄に打つ / ドロップダウンを開く");
    println!("  ・Tab で焦点を回す");
    println!("  ・車輪で送る。つまみを掴んで引く");
    println!("  ・題名の帯を掴んで窓を動かす");
    println!("\n  `-- --report` を付けると、1 フレーム目の中身を数えて出します。\n");

    // `--shot <path>` なら窓を開かず、1 枚描いて BMP に書き出して終わる。
    // 目で見られないとき（手元に画面が無い、CI）用。
    if let Some(path) = shot_path() {
        shoot(font, &path, shot_scale())?;
        return Ok(());
    }

    let event_loop = EventLoop::new()?;
    // 入力があったときだけ描き直す。常時回す必要はない。
    event_loop.set_control_flow(ControlFlow::Wait);
    event_loop.run_app(Gallery::new(font))?;

    Ok(())
}

/// `--shot <path>` の書き出し先。
fn shot_path() -> Option<String> {
    let mut arguments = std::env::args();

    while let Some(argument) = arguments.next() {
        if argument == "--shot" {
            return Some(arguments.next().unwrap_or_else(|| String::from("ui_item.bmp")));
        }
    }

    None
}

/// `--shot-scale <n>`。画面の倍率に当たる。既定は 1。
///
/// 大きくすると、同じ組み立てのまま字が大きく描かれます。
/// **崩れているのが形のせいか、細かさのせいかを分けるのに使います**
/// （形が壊れていれば倍率を上げても崩れたまま）。
fn shot_scale() -> u32 {
    let mut arguments = std::env::args();

    while let Some(argument) = arguments.next() {
        if argument == "--shot-scale" {
            return arguments
                .next()
                .and_then(|value| value.parse().ok())
                .unwrap_or(1)
                .clamp(1, 8);
        }
    }

    1
}

/// `--shot-scroll <px>`。撮る前に全体を送る量。
///
/// 窓に収まらない下のほうを撮るのに使います。
fn shot_scroll() -> f32 {
    let mut arguments = std::env::args();

    while let Some(argument) = arguments.next() {
        if argument == "--shot-scroll" {
            return arguments
                .next()
                .and_then(|value| value.parse().ok())
                .unwrap_or(0.0);
        }
    }

    0.0
}

/// 画面を使わずに 1 枚描いて BMP に書き出す。
///
/// 横幅は 768（= 64 の倍数）にしています。`copy_texture_to_buffer` は
/// 1 行のバイト数が 256 の倍数であることを求めるので、`幅 * 4` が
/// 256 で割り切れる値にしておく必要があります。倍率を掛けても倍数のままです。
fn shoot(font: &'static Font<'static>, path: &str, scale: u32) -> Result<(), Box<dyn Error>> {
    // 論理ピクセル。倍率を掛けたぶんが実際の画素になる。
    const LOGICAL_WIDTH: u32 = 768;
    const LOGICAL_HEIGHT: u32 = 640;

    let width = LOGICAL_WIDTH * scale;
    let height = LOGICAL_HEIGHT * scale;
    const FORMAT: gueiz_2d::texture::TextureFormat =
        gueiz_2d::texture::TextureFormat::Bgra8UnormSrgb;

    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("ui_item shot"),
        required_features: wgpu::Features::INDIRECT_FIRST_INSTANCE & adapter.features(),
        ..Default::default()
    }))?;

    let mut gui = Gui::new();
    gui.set_theme(theme());
    build_ui(&mut gui);

    let viewport = Size::new(LOGICAL_WIDTH as f32, LOGICAL_HEIGHT as f32);
    gui.layout(viewport, &FontMeasure::new(font));

    // 送ってから撮る。送り量は測ったあとでないと頭打ちが決まらない。
    let scroll = shot_scroll();

    if let Some(page) = gui.tree().find(PAGE).filter(|_| scroll > 0.0) {
        if let Some(area) = gui.tree_mut().get_as_mut::<ScrollArea>(page) {
            area.set_offset(scroll);
        }

        gui.tree_mut().request_layout(page);
        gui.layout(viewport, &FontMeasure::new(font));
    }

    let mut draw_manager =
        DrawManager::new(&device, &queue, FORMAT, &DrawManagerDescriptor::default())?;
    let mut renderer = GuiRenderer::new();

    renderer.sync(
        &mut draw_manager,
        gui.paint(),
        Camera::orthographic_2d(viewport.width, viewport.height),
        Some(font),
    );
    draw_manager.prepare(&device, &queue)?;

    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("ui_item target"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT.into(),
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ui_item readback"),
        size: (width * height * 4) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("ui_item shot"),
    });

    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ui"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.043,
                        g: 0.047,
                        b: 0.055,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });

        draw_manager.draw(&mut pass);
    }

    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );

    queue.submit(Some(encoder.finish()));

    readback.map_async(wgpu::MapMode::Read, .., |_| {});
    device.poll(wgpu::PollType::wait_indefinitely())?;

    let mapped = readback.get_mapped_range(..)?;
    let pixels = mapped.to_vec();
    drop(mapped);
    readback.unmap();

    std::fs::write(path, to_bmp(width, height, &pixels))?;

    println!("{path} に {width} x {height} で書き出しました（倍率 {scale}）。");

    Ok(())
}

/// BGRA の並びを 32 ビットの BMP にする。
///
/// 高さを負にすると上から下へ並べる形になるので、読み戻した順のまま書ける。
/// 外の道具を増やさずに済むので、確認用はこれで十分です。
fn to_bmp(width: u32, height: u32, pixels: &[u8]) -> Vec<u8> {
    const HEADER: u32 = 14 + 40;

    let size = HEADER + width * height * 4;
    let mut out = Vec::with_capacity(size as usize);

    out.extend_from_slice(b"BM");
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&HEADER.to_le_bytes());

    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(width as i32).to_le_bytes());
    // 負の高さ = 上から下。
    out.extend_from_slice(&(-(height as i32)).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(width * height * 4).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());

    // 不透明にしておく。透明なまま書くと見る道具によって色が変わる。
    for chunk in pixels.chunks_exact(4) {
        out.extend_from_slice(&[chunk[0], chunk[1], chunk[2], 0xff]);
    }

    out
}

/// `wgpu` に渡す窓の持ち手。
///
/// 描き先は `'static` な持ち手を欲しがるので、窓を [`Arc`] で持って渡します。
#[derive(Clone)]
struct SurfaceHandle {
    window: Arc<dyn Window>,
}

impl HasDisplayHandle for SurfaceHandle {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        self.window.rwh_06_display_handle().display_handle()
    }
}

impl HasWindowHandle for SurfaceHandle {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        self.window.rwh_06_window_handle().window_handle()
    }
}

struct Gallery {
    font: &'static Font<'static>,
    window: Option<Arc<dyn Window>>,
    renderer: Renderer,
    /// 描き先ができてから作る。
    gpu: Option<Gpu>,
    gui: Gui,
    gui_renderer: GuiRenderer,
    /// 論理ピクセルの画面の大きさ。
    viewport: Size,
    scale_factor: f32,
    /// 直近のポインタの位置。釦が押されたときに要る。
    pointer: Point,
    built: bool,
    /// `--report` が付いていたら、最初のフレームで中身を数えて出す。
    report: bool,
}

/// 描き先と、それに紐づくもの。
struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    draw_manager: DrawManager,
}

impl Gallery {
    fn new(font: &'static Font<'static>) -> Self {
        Self {
            font,
            window: None,
            renderer: Renderer::new(RendererBackend::default()),
            gpu: None,
            gui: Gui::new(),
            gui_renderer: GuiRenderer::new(),
            viewport: Size::ZERO,
            scale_factor: 1.0,
            pointer: Point::ZERO,
            built: false,
            report: std::env::args().any(|argument| argument == "--report"),
        }
    }

    /// 1 フレーム描く。
    fn draw(&mut self) {
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };

        // 測る周回のあいだだけ書体を借りる。
        self.gui.layout(self.viewport, &FontMeasure::new(self.font));

        let camera = Camera::orthographic_2d(self.viewport.width, self.viewport.height);
        let list = self.gui.paint();

        self.gui_renderer
            .sync(&mut gpu.draw_manager, list, camera, Some(self.font));

        if let Err(error) = gpu.draw_manager.prepare(&gpu.device, &gpu.queue) {
            log::error!("支度に失敗しました: {error}");
            return;
        }

        let draw_manager = &gpu.draw_manager;

        self.renderer.render(
            wgpu::Color {
                r: 0.043,
                g: 0.047,
                b: 0.055,
                a: 1.0,
            },
            |pass| draw_manager.draw(pass),
        );

        if self.report {
            self.report = false;
            self.print_report();
        }
    }

    /// 本当に形が出ているかを数えて出す。目で見られないときの確かめ用。
    fn print_report(&self) {
        let Some(gpu) = self.gpu.as_ref() else {
            return;
        };

        let list = self.gui.display_list();

        let triangles: usize = gpu
            .draw_manager
            .names()
            .map(String::from)
            .collect::<Vec<_>>()
            .into_iter()
            .filter_map(|name| gpu.draw_manager.object(&name))
            .map(|object| object.triangles().len())
            .sum();

        println!("\n--- 1 フレーム目の中身 ---");
        println!("画面            : {:.0} x {:.0} 論理px（倍率 {}）", self.viewport.width, self.viewport.height, self.scale_factor);
        println!("ウィジェット     : {}", self.gui.tree().len());
        println!("描画命令        : {}", list.len());
        println!("図形            : {}", gpu.draw_manager.object_count());
        println!("三角形の頂点     : {triangles}");
        println!("ドロー          : {}", gpu.draw_manager.draw_count());

        // 命令の種類ごとに数える。どの部品も出ているかが分かる。
        let mut rects = 0;
        let mut borders = 0;
        let mut texts = 0;
        let mut lines = 0;
        let mut clipped = 0;

        for command in list.commands() {
            use gueiz_2d::gui::painter::Primitive;

            match &command.primitive {
                Primitive::Rect { .. } => rects += 1,
                Primitive::Border { .. } => borders += 1,
                Primitive::Text { .. } => texts += 1,
                Primitive::Polyline { .. } => lines += 1,
                _ => {}
            }

            if command.clip.is_some() {
                clipped += 1;
            }
        }

        println!(
            "命令の内訳       : 塗り {rects} / 枠線 {borders} / 字 {texts} / 折れ線 {lines}（切り抜きあり {clipped}）"
        );

        if triangles == 0 {
            println!("\n*** 三角形が 1 つも出ていません。どこかで止まっています。 ***");
        } else {
            println!("\n形は出ています。窓に何も見えないなら、色か配置の問題です。");
        }
    }

    /// 溜まった伝達を端末へ出す。
    fn report_actions(&mut self) {
        for action in self.gui.drain_actions() {
            let name = TAGS
                .iter()
                .find(|(tag, _)| action.tag == Some(*tag))
                .map(|(_, name)| *name);

            let Some(name) = name else {
                // 札を付けていない部品。背景を押したなど。出すと騒がしい。
                continue;
            };

            match &action.kind {
                ActionKind::Clicked { button } => {
                    println!("{name}: 押された（{button:?}）");
                }
                ActionKind::ValueChanged => {
                    println!("{name}: 変わった → {}", self.describe(action.widget));
                }
                ActionKind::Submitted => {
                    println!("{name}: 確定 → {}", self.describe(action.widget));
                }
                ActionKind::Cancelled => println!("{name}: 取り消し"),
                ActionKind::Custom(code) => println!("{name}: 自前の伝達 {code}"),
            }
        }
    }

    /// その部品のいまの値を字にする。
    fn describe(&self, id: WidgetId) -> String {
        if let Some(field) = self.gui.tree().get_as::<TextField>(id) {
            return format!("{:?}", field.text());
        }

        if let Some(dropdown) = self.gui.tree().get_as::<Dropdown>(id) {
            return format!("{:?}", dropdown.selected_item());
        }

        String::from("(値なし)")
    }

    /// 物理ピクセルを論理ピクセルに直す。
    fn to_logical(&self, x: f64, y: f64) -> Point {
        Point::new(
            x as f32 / self.scale_factor,
            y as f32 / self.scale_factor,
        )
    }

    fn resize(&mut self, width: u32, height: u32, scale_factor: f32) {
        self.scale_factor = scale_factor.max(0.1);
        self.viewport = Size::new(
            width as f32 / self.scale_factor,
            height as f32 / self.scale_factor,
        );

        self.gui.set_scale_factor(self.scale_factor);
        self.renderer.resize(SurfaceSize::new(width, height));
    }
}

impl ApplicationHandler for Gallery {
    fn can_create_surfaces(&mut self, event_loop: &dyn ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let attributes = WindowAttributes::default()
            .with_title("gueiz GUI items")
            .with_surface_size(LogicalSize::new(760.0, 620.0));

        let window: Arc<dyn Window> = match event_loop.create_window(attributes) {
            Ok(window) => Arc::from(window),
            Err(error) => {
                log::error!("窓を開けませんでした: {error}");
                event_loop.exit();
                return;
            }
        };

        let size = window.surface_size();
        let scale_factor = window.scale_factor() as f32;

        let handle = SurfaceHandle {
            window: Arc::clone(&window),
        };

        if let Err(error) = self
            .renderer
            .create_surface(handle, SurfaceSize::new(size.width, size.height))
        {
            log::error!("描き先を作れませんでした: {error}");
            event_loop.exit();
            return;
        }

        let (Some(device), Some(queue), Some(format)) = (
            self.renderer.device().cloned(),
            self.renderer.queue().cloned(),
            self.renderer.surface_format(),
        ) else {
            log::error!("描き先が揃いませんでした");
            event_loop.exit();
            return;
        };

        let draw_manager = match DrawManager::new(&device, &queue, format, &DrawManagerDescriptor::default())
        {
            Ok(draw_manager) => draw_manager,
            Err(error) => {
                log::error!("DrawManager を作れませんでした: {error}");
                event_loop.exit();
                return;
            }
        };

        self.gpu = Some(Gpu {
            device,
            queue,
            draw_manager,
        });

        self.resize(size.width, size.height, scale_factor);

        if !self.built {
            self.gui.set_theme(theme());
            build_ui(&mut self.gui);
            // 入力を流す前に 1 度測っておく。当たり判定は置き場所を見るので、
            // 測っていないと最初の 1 回がどこにも当たらない。
            self.gui.layout(self.viewport, &FontMeasure::new(self.font));
            self.built = true;
        }

        window.request_redraw();
        self.window = Some(window);
    }

    fn window_event(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        // 入力を GUI の型へ詰め替える。**ここが窓と GUI の境目。**
        let input = match &event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
                return;
            }

            WindowEvent::SurfaceResized(size) => {
                let scale_factor = self
                    .window
                    .as_ref()
                    .map(|window| window.scale_factor() as f32)
                    .unwrap_or(1.0);

                self.resize(size.width, size.height, scale_factor);
                self.gui.request_layout();

                None
            }

            WindowEvent::RedrawRequested => {
                self.draw();
                return;
            }

            WindowEvent::PointerMoved { position, .. } => {
                self.pointer = self.to_logical(position.x, position.y);

                Some(InputEvent::PointerMoved {
                    position: self.pointer,
                })
            }

            WindowEvent::PointerLeft { .. } => Some(InputEvent::PointerLeft),

            WindowEvent::PointerButton {
                state,
                position,
                button,
                ..
            } => {
                let Some(gui_button) = to_gui_button(button) else {
                    return;
                };

                self.pointer = self.to_logical(position.x, position.y);

                Some(match state {
                    ElementState::Pressed => InputEvent::PointerPressed {
                        position: self.pointer,
                        button: gui_button,
                    },
                    ElementState::Released => InputEvent::PointerReleased {
                        position: self.pointer,
                        button: gui_button,
                    },
                })
            }

            WindowEvent::MouseWheel { delta, .. } => {
                let delta = match delta {
                    MouseScrollDelta::LineDelta(x, y) => ScrollDelta::Lines { x: *x, y: *y },
                    MouseScrollDelta::PixelDelta(position) => ScrollDelta::Pixels {
                        x: position.x as f32 / self.scale_factor,
                        y: position.y as f32 / self.scale_factor,
                    },
                };

                Some(InputEvent::Scrolled {
                    position: self.pointer,
                    delta,
                })
            }

            WindowEvent::ModifiersChanged(modifiers) => {
                Some(InputEvent::ModifiersChanged(to_gui_modifiers(modifiers)))
            }

            WindowEvent::KeyboardInput { event, .. } => {
                // Esc で終わる。GUI に渡す前に見る。
                if event.state == ElementState::Pressed
                    && event.logical_key == WinitKey::Named(WinitNamedKey::Escape)
                    && self.gui.focused().is_none()
                {
                    event_loop.exit();
                    return;
                }

                // Tab は焦点の移動に使う。部品に渡さない。
                if event.state == ElementState::Pressed
                    && event.logical_key == WinitKey::Named(WinitNamedKey::Tab)
                {
                    if self.gui.modifiers().shift() {
                        self.gui.focus_previous();
                    } else {
                        self.gui.focus_next();
                    }

                    if let Some(window) = self.window.as_ref() {
                        window.request_redraw();
                    }

                    return;
                }

                let key = to_gui_key(&event.logical_key);

                if event.state == ElementState::Pressed {
                    // 鍵と、確定した字は別に流す。打った字は `text` のほう。
                    if let Some(key) = key {
                        self.gui.handle_input(InputEvent::KeyPressed {
                            key,
                            repeat: event.repeat,
                        });
                    }

                    if let Some(text) = &event.text {
                        self.gui.handle_input(InputEvent::Text(text.to_string()));
                    }
                } else if let Some(key) = key {
                    self.gui.handle_input(InputEvent::KeyReleased { key });
                }

                None
            }

            _ => None,
        };

        if let Some(input) = input {
            self.gui.handle_input(input);
        }

        self.report_actions();

        // 触られたら描き直す。
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }
}

// --- 入力の詰め替え ---

fn to_gui_button(source: &ButtonSource) -> Option<PointerButton> {
    Some(match source {
        ButtonSource::Mouse(MouseButton::Left) => PointerButton::Primary,
        ButtonSource::Mouse(MouseButton::Right) => PointerButton::Secondary,
        ButtonSource::Mouse(MouseButton::Middle) => PointerButton::Middle,
        // 触りと筆は主ボタンとして扱う。
        ButtonSource::Touch { .. } | ButtonSource::TabletTool { .. } => PointerButton::Primary,
        ButtonSource::Unknown(code) => PointerButton::Other(*code),
        // 戻る・進むなどは GUI では扱わない。
        ButtonSource::Mouse(_) => return None,
    })
}

fn to_gui_modifiers(modifiers: &winit::event::Modifiers) -> GuiModifiers {
    let state = modifiers.state();
    let mut result = GuiModifiers::empty();

    result.set(GuiModifiers::SHIFT, state.shift_key());
    result.set(GuiModifiers::CONTROL, state.control_key());
    result.set(GuiModifiers::ALT, state.alt_key());
    // macOS の Command / Windows の Windows 鍵。
    result.set(GuiModifiers::SUPER, state.meta_key());

    result
}

fn to_gui_key(key: &WinitKey) -> Option<GuiKey> {
    let named = |named| Some(GuiKey::Named(named));

    match key {
        WinitKey::Character(text) => {
            // 1 文字だけ取る。合わせ字は `Event::Text` のほうで届く。
            let character = text.chars().next()?;

            // **winit では間隔は文字として来る。** GUI 側は名前付きで持っている
            // （押下として扱うため）ので、ここで読み替える。
            if character == ' ' {
                return named(GuiNamedKey::Space);
            }

            Some(GuiKey::Character(character.to_ascii_lowercase()))
        }

        WinitKey::Named(name) => match name {
            WinitNamedKey::ArrowUp => named(GuiNamedKey::ArrowUp),
            WinitNamedKey::ArrowDown => named(GuiNamedKey::ArrowDown),
            WinitNamedKey::ArrowLeft => named(GuiNamedKey::ArrowLeft),
            WinitNamedKey::ArrowRight => named(GuiNamedKey::ArrowRight),
            WinitNamedKey::Home => named(GuiNamedKey::Home),
            WinitNamedKey::End => named(GuiNamedKey::End),
            WinitNamedKey::PageUp => named(GuiNamedKey::PageUp),
            WinitNamedKey::PageDown => named(GuiNamedKey::PageDown),
            WinitNamedKey::Backspace => named(GuiNamedKey::Backspace),
            WinitNamedKey::Delete => named(GuiNamedKey::Delete),
            WinitNamedKey::Enter => named(GuiNamedKey::Enter),
            WinitNamedKey::Tab => named(GuiNamedKey::Tab),
            WinitNamedKey::Escape => named(GuiNamedKey::Escape),
            WinitNamedKey::Insert => named(GuiNamedKey::Insert),
            WinitNamedKey::Shift => named(GuiNamedKey::Shift),
            WinitNamedKey::Control => named(GuiNamedKey::Control),
            WinitNamedKey::Alt => named(GuiNamedKey::Alt),
            WinitNamedKey::Meta => named(GuiNamedKey::Super),
            WinitNamedKey::CapsLock => named(GuiNamedKey::CapsLock),
            WinitNamedKey::F1 => named(GuiNamedKey::F1),
            WinitNamedKey::F2 => named(GuiNamedKey::F2),
            WinitNamedKey::F3 => named(GuiNamedKey::F3),
            WinitNamedKey::F4 => named(GuiNamedKey::F4),
            WinitNamedKey::F5 => named(GuiNamedKey::F5),
            WinitNamedKey::F6 => named(GuiNamedKey::F6),
            WinitNamedKey::F7 => named(GuiNamedKey::F7),
            WinitNamedKey::F8 => named(GuiNamedKey::F8),
            WinitNamedKey::F9 => named(GuiNamedKey::F9),
            WinitNamedKey::F10 => named(GuiNamedKey::F10),
            WinitNamedKey::F11 => named(GuiNamedKey::F11),
            WinitNamedKey::F12 => named(GuiNamedKey::F12),
            // ここに無い鍵は落とす。部品が見ていないので困らない。
            _ => None,
        },

        _ => None,
    }
}

// --- 書体 ---

/// 仮名を持つ書体を先に探す。無ければ Arial。
fn load_font() -> Option<&'static Font<'static>> {
    const CANDIDATES: &[&str] = &[
        // 仮名を持つもの。
        "/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc",
        "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
        "C:/Windows/Fonts/YuGothM.ttc",
        "C:/Windows/Fonts/meiryo.ttc",
        "C:/Windows/Fonts/msgothic.ttc",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        // 仮名を持たない。日本語の札は消えるが、形は見える。
        "/System/Library/Fonts/Supplemental/Arial.ttf",
        "C:/Windows/Fonts/arial.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    ];

    for path in CANDIDATES {
        let Ok(data) = std::fs::read(path) else {
            continue;
        };

        // 書体はずっと使うので漏らす。借りているバイト列の寿命を気にせず持ち回せる。
        let Ok(font) = Font::from_bytes(data.leak()) else {
            continue;
        };

        let kana = font.glyph('あ').is_some();

        println!("書体: {path}（仮名 {}）", if kana { "あり" } else { "なし ― 日本語の札は出ません" });

        return Some(Box::leak(Box::new(font)));
    }

    None
}

// --- 見た目 ---

/// この例だけの配色。**ライブラリは既定の配色を持ちません。**
fn theme() -> Theme {
    let mut theme = Theme::new();

    theme.set_metrics(Metrics {
        spacing: 8.0,
        text_size: 15.0,
        control_height: 30.0,
        line_height_factor: 1.35,
    });

    let control = Style::BARE
        .background(Color::hex(0x24262e))
        .border(Color::hex(0x3a3d48), 1.0)
        .corners(Corners::all(5.0))
        .foreground(Color::hex(0xe8e9ee))
        .text_size(15.0)
        .padding(Insets::symmetric(6.0, 12.0));

    theme.set(Role::CONTROL, StateKey::Normal, control);
    theme.set(
        Role::CONTROL,
        StateKey::Hovered,
        control.background(Color::hex(0x2f323c)),
    );
    theme.set(
        Role::CONTROL,
        StateKey::Pressed,
        control.background(Color::hex(0x1b1d23)),
    );
    // 焦点の輪。**登録したから出る**（Button と TextField がこれを重ねる）。
    theme.set(
        Role::CONTROL,
        StateKey::Focused,
        control.border(Color::hex(0x6aa9ff), 2.0),
    );
    theme.set(
        Role::CONTROL,
        StateKey::Disabled,
        control
            .background(Color::hex(0x1c1e24))
            .border(Color::hex(0x2a2c33), 1.0)
            .foreground(Color::hex(0x55585f)),
    );

    // 窓と一覧の下敷き。
    theme.set(
        Role::SURFACE,
        StateKey::Normal,
        Style::BARE
            .background(Color::hex(0x171920))
            .border(Color::hex(0x3a3d48), 1.0)
            .corners(Corners::all(8.0))
            .foreground(Color::hex(0xe8e9ee))
            .text_size(15.0)
            .padding(Insets::all(10.0)),
    );

    // 題名の帯、選んだ範囲、キャレット、送りのつまみ。
    theme.set(
        Role::ACCENT,
        StateKey::Normal,
        Style::BARE
            .background(Color::hex(0x2b3042))
            .foreground(Color::hex(0x6aa9ff))
            .corners(Corners::all(4.0))
            .text_size(14.0)
            .padding(Insets::symmetric(0.0, 10.0)),
    );
    theme.set(
        Role::ACCENT,
        StateKey::Hovered,
        Style::BARE
            .background(Color::hex(0x20232b))
            .foreground(Color::hex(0x8dbcff))
            .corners(Corners::all(4.0)),
    );
    theme.set(
        Role::ACCENT,
        StateKey::Pressed,
        Style::BARE
            .background(Color::hex(0x20232b))
            .foreground(Color::hex(0xb6d4ff))
            .corners(Corners::all(4.0)),
    );

    // 案内の字。
    theme.set(
        Role::TEXT,
        StateKey::Normal,
        Style::BARE
            .foreground(Color::hex(0x6e737f))
            .text_size(15.0),
    );

    // 見出し。
    theme.set(
        HEADING,
        StateKey::Normal,
        Style::BARE
            .foreground(Color::hex(0x8dbcff))
            .text_size(13.0),
    );

    theme
}

// --- 組み立て ---

fn build_ui(gui: &mut Gui) {
    // 根は浮かせる台。**これが無いと `WindowFrame` の位置が効かず、
    // 掴んで動かせません**（置き場所を決めるのは親なので）。
    let desktop = gui.set_root(Box::new(Floating::new()));

    let frame = gui.tree_mut().add_child(
        desktop,
        Box::new(
            WindowFrame::new(Point::new(16.0, 16.0), Size::new(720.0, 570.0))
                .title("gueiz GUI items ― 帯を掴んで動かせます")
                .movable(true)
                .padding(Insets::ZERO),
        ),
    );

    // 全体を送れるようにする。入れ子の送り箱もこの中に置く。
    let scroll = gui
        .tree_mut()
        .add_child(frame, Box::new(ScrollArea::vertical()));
    gui.tree_mut().set_tag(scroll, PAGE);

    let page = gui.tree_mut().add_child(
        scroll,
        Box::new(Stack::column().gap(6.0).padding(Insets::all(14.0))),
    );

    // --- Button ---
    heading(gui, page, "Button ― 乗る・押す・焦点・無効");

    let row = gui
        .tree_mut()
        .add_child(page, Box::new(Stack::row().gap(8.0)));

    let ok = gui
        .tree_mut()
        .add_child(row, Box::new(Button::new("OK").min_width(80.0)));
    gui.tree_mut().set_tag(ok, Tag::new("ok"));

    let cancel = gui
        .tree_mut()
        .add_child(row, Box::new(Button::new("やめる").min_width(80.0)));
    gui.tree_mut().set_tag(cancel, Tag::new("cancel"));

    let disabled = gui
        .tree_mut()
        .add_child(row, Box::new(Button::new("無効").min_width(80.0)));
    gui.tree_mut().set_disabled(disabled, true);

    // --- TextField ---
    heading(gui, page, "TextField ― 打つ・選ぶ・隠す");

    let name = gui.tree_mut().add_child(
        page,
        Box::new(TextField::new().placeholder("名前を入れてください").min_width(300.0)),
    );
    gui.tree_mut().set_tag(name, Tag::new("name"));

    let password = gui.tree_mut().add_child(
        page,
        Box::new(
            TextField::with_text("himitsu")
                .mask('●')
                .min_width(300.0),
        ),
    );
    gui.tree_mut().set_tag(password, Tag::new("password"));

    // --- Dropdown ---
    heading(gui, page, "Dropdown ― 開くと切り抜きを突き抜けて手前に出る");

    let quality = gui.tree_mut().add_child(
        page,
        Box::new(
            Dropdown::new(["低", "中", "高", "最高", "自動"])
                .selected(1)
                .min_width(200.0),
        ),
    );
    gui.tree_mut().set_tag(quality, Tag::new("quality"));

    // --- 入れ子の ScrollArea ---
    heading(gui, page, "ScrollArea（入れ子）― 端まで送ると外側に渡る");

    let inner = gui
        .tree_mut()
        .add_child(page, Box::new(ScrollArea::vertical()));

    let list = gui
        .tree_mut()
        .add_child(inner, Box::new(Stack::column().gap(4.0).padding(Insets::all(4.0))));

    for index in 0..12 {
        gui.tree_mut()
            .add_child(list, Box::new(Button::new(format!("行 {index}"))));
    }

    // **丈を決めてやらないと内側は送りません。** 外側は上限なしで測るので。
    if let Some(stack) = gui.tree_mut().get_as_mut::<Stack>(page) {
        stack.set_length(inner, Length::Fixed(140.0));
    }

    // --- Stack / Length::Grow ---
    heading(gui, page, "Stack ― Length::Grow で余りを 1 : 2 : 1 に分ける");

    let grow = gui
        .tree_mut()
        .add_child(page, Box::new(Stack::row().gap(8.0)));

    let weights = [1.0, 2.0, 1.0];
    let mut children = Vec::new();

    for weight in weights {
        children.push(
            gui.tree_mut()
                .add_child(grow, Box::new(Button::new(format!("Grow({weight})")))),
        );
    }

    if let Some(stack) = gui.tree_mut().get_as_mut::<Stack>(grow) {
        for (child, weight) in children.into_iter().zip(weights) {
            stack.set_length(child, Length::Grow(weight));
        }
    }

    // --- 寄せ ---
    heading(gui, page, "Stack ― CrossAlign::Stretch で横いっぱい");

    let stretched = gui.tree_mut().add_child(
        page,
        Box::new(
            Stack::column()
                .gap(6.0)
                .cross_align(CrossAlign::Stretch),
        ),
    );

    gui.tree_mut()
        .add_child(stretched, Box::new(Button::new("伸びるボタン")));
    gui.tree_mut()
        .add_child(stretched, Box::new(TextField::new().placeholder("伸びる欄")));

    // --- 三角形分割の確かめ ---
    heading(gui, page, "字形 ― かつて崩れていた字と、崩れていなかった字");

    // 穴が 2 つ以上あって、1 点目に戻って閉じる輪郭を持つ字。
    // 通路が 1 つの頂点に集まって耳刈り取りが詰まっていた。
    label(gui, page, "崩れていた: 8 面 B 0 ∞ ☃ ⽢ ⽿");
    // 本体と少しだけ重なる飾りを持つ字。飾りが穴と取り違えられて消えていた。
    label(gui, page, "飾りが消えていた: Ç Å Ą Ę Ų ŀ Ş Ţ Ǫ");
    // もともと通っていた字。崩れ出していないことの確かめ。
    label(gui, page, "もともと通っていた: 軸 個 図 絞 辺 鬱 纏 躇 驚");

    // 下に余白を足して、送れることを分かりやすくする。
    heading(gui, page, "（ここまで。車輪で送れます）");
}

/// 見出しを 1 行足す。
fn heading(gui: &mut Gui, parent: WidgetId, text: &str) {
    gui.tree_mut()
        .add_child(parent, Box::new(Label::new(text).role(HEADING)));
}

/// 本文を 1 行足す。
fn label(gui: &mut Gui, parent: WidgetId, text: &str) {
    gui.tree_mut()
        .add_child(parent, Box::new(Label::new(text).role(Role::CONTROL)));
}

/// 字を 1 行出すだけの部品。
///
/// ライブラリにはまだ無いので、この例の中で持っています。
/// 本当に要るようになったら `gui` へ移すところです。
struct Label {
    text: String,
    role: Role,
}

impl Label {
    fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            role: Role::TEXT,
        }
    }

    fn role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }

    fn options(&self, size: f32) -> TextLayoutOptions {
        TextLayoutOptions {
            size,
            horizontal: Align::Start,
            vertical: Align::Center,
            wrap: false,
        }
    }
}

impl Widget for Label {
    fn measure(&mut self, context: &mut MeasureContext<'_>, constraints: Constraints) -> Size {
        let style = context.theme().style(self.role, context.state());
        let text = context.measure_text(&self.text, self.options(style.text_size), None);

        constraints.constrain(Size::new(
            text.width,
            context.line_height(style.text_size) + 6.0,
        ))
    }

    fn paint(&self, context: &PaintContext<'_>, painter: &mut Painter<'_>) {
        let style = context.theme().style(self.role, context.state());

        painter.text(
            context.rect(),
            self.text.as_str(),
            self.options(style.text_size),
            style.foreground,
        );
    }

    fn behavior(&self) -> Behavior {
        // 飾り。押しても素通りして後ろに当たる。
        Behavior::DECORATION
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::event::MouseButton;
    use winit::keyboard::{ModifiersKeys, ModifiersState};

    /// 間隔は winit では**文字として**来る。GUI 側は名前付きで持っているので、
    /// ここを取り違えるとボタンが Space で押せなくなる。
    #[test]
    fn space_arrives_as_a_character_and_is_renamed() {
        let space = WinitKey::Character(" ".into());

        assert_eq!(
            to_gui_key(&space),
            Some(GuiKey::Named(GuiNamedKey::Space)),
            "間隔は名前付きに読み替える"
        );
    }

    #[test]
    fn characters_are_lowercased_for_shortcuts() {
        // `Cmd+A` は Shift 無しでも大文字で来ることがある。
        assert_eq!(
            to_gui_key(&WinitKey::Character("A".into())),
            Some(GuiKey::Character('a'))
        );
        assert_eq!(
            to_gui_key(&WinitKey::Character("a".into())),
            Some(GuiKey::Character('a'))
        );
    }

    #[test]
    fn the_named_keys_the_widgets_look_at_all_map() {
        // 部品が見ている鍵が 1 つでも落ちると、その操作だけ効かなくなる。
        let pairs = [
            (WinitNamedKey::Enter, GuiNamedKey::Enter),
            (WinitNamedKey::Escape, GuiNamedKey::Escape),
            (WinitNamedKey::Backspace, GuiNamedKey::Backspace),
            (WinitNamedKey::Delete, GuiNamedKey::Delete),
            (WinitNamedKey::ArrowLeft, GuiNamedKey::ArrowLeft),
            (WinitNamedKey::ArrowRight, GuiNamedKey::ArrowRight),
            (WinitNamedKey::ArrowUp, GuiNamedKey::ArrowUp),
            (WinitNamedKey::ArrowDown, GuiNamedKey::ArrowDown),
            (WinitNamedKey::Home, GuiNamedKey::Home),
            (WinitNamedKey::End, GuiNamedKey::End),
            (WinitNamedKey::Tab, GuiNamedKey::Tab),
        ];

        for (from, to) in pairs {
            assert_eq!(
                to_gui_key(&WinitKey::Named(from)),
                Some(GuiKey::Named(to)),
                "{from:?} が落ちている"
            );
        }
    }

    #[test]
    fn mouse_buttons_map_and_the_side_buttons_are_dropped() {
        assert_eq!(
            to_gui_button(&ButtonSource::Mouse(MouseButton::Left)),
            Some(PointerButton::Primary)
        );
        assert_eq!(
            to_gui_button(&ButtonSource::Mouse(MouseButton::Right)),
            Some(PointerButton::Secondary)
        );
        assert_eq!(
            to_gui_button(&ButtonSource::Mouse(MouseButton::Middle)),
            Some(PointerButton::Middle)
        );
        // 戻る・進むは GUI に流さない。
        assert_eq!(to_gui_button(&ButtonSource::Mouse(MouseButton::Back)), None);
    }

    #[test]
    fn the_command_key_lands_on_super() {
        // macOS の Command は winit では `meta`。GUI 側は `SUPER` で持っている。
        let modifiers = winit::event::Modifiers::new(
            ModifiersState::META | ModifiersState::SHIFT,
            ModifiersKeys::empty(),
        );

        let translated = to_gui_modifiers(&modifiers);

        assert!(translated.contains(GuiModifiers::SUPER));
        assert!(translated.shift());
        assert!(!translated.contains(GuiModifiers::CONTROL));
    }

    /// 窓を開けない環境でも、組み立てとレイアウトは確かめられる。
    #[test]
    fn the_whole_gallery_lays_out_without_a_window() {
        use gueiz_2d::gui::context::NoTextMeasure;

        let mut gui = Gui::new();
        gui.set_theme(theme());
        build_ui(&mut gui);

        gui.layout(Size::new(760.0, 620.0), &NoTextMeasure);

        // 部品が全部入っていて、形が出ている。
        assert!(gui.tree().len() > 30, "{}", gui.tree().len());
        assert!(!gui.paint().is_empty());

        // 札を付けたものは全部引ける。
        for (tag, name) in TAGS {
            assert!(gui.tree().find(*tag).is_some(), "{name} が見つからない");
        }
    }
}
