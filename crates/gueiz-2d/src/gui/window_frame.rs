use std::os::unix::raw::off_t;
use gueiz_gpu::camera::ScaleMode;
use gueiz_gpu::renderer::{FrameOutcome, SurfaceSize};
use gueiz_gpu::vertex::Vertex;
use crate::draw_manager::DrawManager;
use crate::effect::Block;
use crate::gui::window_item::WindowItemArguments2;
use crate::instance::create_instance;
use crate::object::Object;
use crate::objects::lines::{CurveRepresentationType, Lines};
use crate::objects::polygon_rounded::CornerType;
use crate::objects::rect::Rect;
use crate::objects::rect_rounded::RectRounded;
use crate::paint_type::{JointType, PaintType};
use crate::resource::{FontHandle, Resources};
use crate::text::{measure, TextAlign, TextArea, TextLayoutData, TextLocation, TextRenderer, TextStyle};

const LINE_WIDTH: f32 = 1.0;
const HANDLE_OUTSIDE: f32 = 6.0;
const HANDLE_INSIDE: f32 = 3.0;

pub struct WindowFrame {
    title: String,
    text_size: f32,
    frame_quad: Quad,
    title_bar_height: f32,
    gap: Gap,
    window_theme: WindowTheme,
    window_font: WindowFont,
    scale_mode: ScaleMode,
    register_name: RegisterName,
    min_width: f32,
    built_origin: [f32; 2],
    drag: Option<Drag>,
    resize: Option<Resize>,
    pending: Pending,
    applied_clip: Option<[Block; 2]>,
}

impl WindowFrame {
    pub fn new(register_name_string: String, title: String, text_size: f32, frame_quad: Quad, title_bar_height: f32, gap: Gap, window_theme: WindowTheme, window_font: WindowFont, scale_mode: ScaleMode) -> Self {
        Self {
            title,
            text_size,
            frame_quad,
            title_bar_height,
            gap,
            window_theme,
            window_font,
            scale_mode,
            register_name: RegisterName {
                frame: Some(register_name_string),
                title_bar: None,
                frame_outline: None,
                title_bar_slice_line: None,
                text_title: None,
            },
            min_width: 120.0,
            built_origin: [0.0, 0.0],
            drag: None,
            resize: None,
            pending: Pending::default(),
            applied_clip: None,
        }
    }

    pub fn create_object(&mut self, draw_manager: &mut DrawManager, resources: &Resources, surface_size: SurfaceSize, design: [f32; 2]) {
        self.min_width = self.measure_min_width(resources);
        self.frame_quad.width = self.frame_quad.width.max(self.min_width);
        self.built_origin = [self.frame_quad.x, self.frame_quad.y];

        let register_name_frame = self.register_name.frame.as_ref().unwrap();
        self.register_name.title_bar = Some(format!("{register_name_frame}-title_bar"));
        self.register_name.frame_outline = Some(format!("{register_name_frame}-frame_outline"));
        self.register_name.title_bar_slice_line = Some(format!("{register_name_frame}-title_bar_slice_line"));
        self.register_name.text_title = Some(format!("{register_name_frame}-text_title"));

        let objects = self.build_shapes();
        for mut object in objects {
            object.camera_for(surface_size, design, self.scale_mode);
            object.instance(create_instance());
            draw_manager.register(object);
        }

        let name = |name: &Option<String>| name.clone().unwrap_or_default();
        let mut text_title = TextRenderer::new(&name(&self.register_name.text_title));
        text_title.camera_for(surface_size, design, self.scale_mode);
        text_title.color(self.window_theme.text_title_color.r,self.window_theme.text_title_color.g,self.window_theme.text_title_color.b,self.window_theme.text_title_color.a);
        text_title.text_layout_data(self.text_title_layout_data());
        if self.title.is_empty() {
            return;
        }
        let Some(font) = resources.font(self.window_font.base_font) else {
            log::warn!("the title font is not in the resources; the title is not drawn");
            return;
        };

        text_title.register_draw_manager(draw_manager, &font).expect("Failed to register draw manager");
        draw_manager.add_text_renderer(text_title);
    }

    pub fn update(&mut self, draw_manager: &mut DrawManager, resources: &Resources) -> bool {
        let pending = std::mem::take(&mut self.pending);

        if pending.rebuild {
            self.rebuild_all(draw_manager, resources);
        } else if pending.offset {
            self.apply_offset(draw_manager)
        }

        pending.rebuild || pending.offset
    }

    fn rebuild_all(&mut self, draw_manager: &mut DrawManager, resources: &Resources) {
        self.rebuild_shapes(draw_manager);
        self.relayout_title(draw_manager, resources);
        self.rebuild_items(draw_manager, resources);

        self.apply_offset(draw_manager);
    }

    fn rebuild_items(&mut self, draw_manager: &mut DrawManager, resources: &Resources) {
        // let [item_x, mut item_y] = self.item_origin();
        // let width = self.item_width();
        // let mut counter = 0;
        // for window_item in &mut self.window_items {
        //     let item_size = window_item.rebuild(
        //         WindowItemArguments2 {
        //             draw_manager,
        //             resources: &resources,
        //             register_name: &self.register_name.frame.as_ref().unwrap(),
        //             counter,
        //             item_x,
        //             item_y,
        //             width,
        //             scale_mode: self.scale_mode,
        //         }
        //     );
        //     item_y += item_size.1;
        //     counter += 1;
        // }
    }

    fn relayout_title(&mut self, draw_manager: &mut DrawManager, resources: &Resources) {
        let layout_data = self.text_title_layout_data();

        if let Some(font) = resources.font(self.window_font.base_font) {
            draw_manager.with_text_renderer(self.register_name.frame.as_ref().unwrap(), |text_title, draw_manager| {
                text_title.clear(draw_manager);
                text_title.text_layout_data(layout_data);

                if let Err(error) = text_title.register_draw_manager(draw_manager, &font) {
                    log::warn!("failed to lay out the title again: {error}");
                }
            });
        }
    }

    fn rebuild_shapes(&mut self, draw_manager: &mut DrawManager) {
        let names = [
            self.register_name.frame.as_ref(),
            self.register_name.title_bar.as_ref(),
            self.register_name.frame_outline.as_ref(),
            self.register_name.title_bar_slice_line.as_ref(),
        ];

        for (name, built) in names.into_iter().zip(self.build_shapes()) {
            let Some(object) = name.and_then(|name| draw_manager.object_mut(name)) else {
                continue;
            };

            object.begin(built.paint_type());
            for vertex in built.vertices() {
                object.put_vertex(*vertex);
            }
            object.end();
        }
    }

    fn apply_offset(&mut self, draw_manager: &mut DrawManager) {
        let x = self.frame_quad.x - self.built_origin[0];
        let y = self.frame_quad.y - self.built_origin[1];

        for name in self.object_names(draw_manager) {
            if let Some(object) = draw_manager.object_mut(&name) {
                object.translate(x, y, 0.0);
            }
        }

        self.apply_clip(draw_manager);
    }

    fn apply_clip(&mut self, draw_manager: &mut DrawManager) {
        let blocks = self.clip_blocks();
        let applied = self.applied_clip;

        // for window_item in &self.window_items {
        //     for name in window_item.object_names() {
        //         let Some(object) = draw_manager.object_mut(&name) else {
        //             continue;
        //         };
        //
        //         for (index, block) in blocks.into_iter().enumerate() {
        //             let old = applied.map(|applied| applied[index]);
        //             object.replace_effect(|effect| Some(*effect) == old, block);
        //         }
        //     }
        // }

        self.applied_clip = Some(blocks);
    }

    fn clip_blocks(&self) -> [Block; 2] {
        let top = self.frame_quad.y + self.title_bar_height;

        [
            Block::ClipRect {
                min: [self.frame_quad.x, self.frame_quad.y],
                max: [self.frame_quad.x + self.frame_quad.width, self.frame_quad.y + self.frame_quad.height],
                radius: self.frame_quad.quad_type.radius(),
                softness: 0.0,
                invert: false,
            },
            Block::ClipHalfPlane {
                normal: [0.0, -1.0],
                distance: -top,
                softness: 0.0,
                invert: false,
            },
        ]
    }

    fn text_title_layout_data(&self) -> TextLayoutData {
        let [x, y] = self.built_origin;

        TextLayoutData {
            text: self.title.clone(),
            style: TextStyle::new(self.text_size),
            text_location: TextLocation::Area {
                area: TextArea::new(x + self.gap.x, y, self.frame_quad.width - (self.gap.x * 2.0), self.title_bar_height)
                    .vertical(TextAlign::Center),
            },
        }
    }

    fn build_shapes(&self) -> Vec<Object> {
        let [x, y] = self.built_origin;
        let width = self.frame_quad.width;
        let height = self.frame_quad.height;
        let theme = &self.window_theme;

        let name = |name: &Option<String>| name.clone().unwrap_or_default();
        let vertex = |x: f32, y: f32, color: &ThemeColor| {
            Vertex::new_position_color(x, y, 0.0, color.r, color.g, color.b, color.a)
        };

        let frame = match self.frame_quad.quad_type {
            QuadType::Normal => {
                Rect::new(&name(&self.register_name.frame))
                    .paint_type(PaintType::Fill)
                    .vertex1(vertex(x, y + self.title_bar_height, &theme.frame_fill_color))
                    .vertex2(vertex(x + width, y + self.title_bar_height, &theme.frame_fill_color))
                    .vertex3(vertex(x + width, y + height, &theme.frame_fill_color))
                    .vertex4(vertex(x, y + height, &theme.frame_fill_color))
                    .end()
            }
            QuadType::Rounded { radius } => {
                RectRounded::new(&name(&self.register_name.frame))
                    .paint_type(PaintType::Fill)
                    .vertex1(CornerType::None {}, vertex(x, y + self.title_bar_height, &theme.frame_fill_color))
                    .vertex2(CornerType::None {}, vertex(x + width, y + self.title_bar_height, &theme.frame_fill_color))
                    .vertex3(CornerType::Circle {radius}, vertex(x + width, y + height, &theme.frame_fill_color))
                    .vertex4(CornerType::Circle {radius}, vertex(x, y + height, &theme.frame_fill_color))
                    .end()
            }
        };

        let title_bar = match self.frame_quad.quad_type {
            QuadType::Normal => {
                Rect::new(&name(&self.register_name.title_bar))
                    .paint_type(PaintType::Fill)
                    .vertex1(vertex(x, y, &theme.title_bar_fill_color))
                    .vertex2(vertex(x + width, y, &theme.title_bar_fill_color))
                    .vertex3(vertex(x + width, y + self.title_bar_height, &theme.title_bar_fill_color))
                    .vertex4(vertex(x, y + self.title_bar_height, &theme.title_bar_fill_color))
                    .end()
            }
            QuadType::Rounded { radius} => {
                RectRounded::new(&name(&self.register_name.title_bar))
                    .paint_type(PaintType::Fill)
                    .vertex1(CornerType::Circle {radius}, vertex(x, y, &theme.title_bar_fill_color))
                    .vertex2(CornerType::Circle {radius}, vertex(x + width, y, &theme.title_bar_fill_color))
                    .vertex3(CornerType::None {}, vertex(x + width, y + self.title_bar_height, &theme.title_bar_fill_color))
                    .vertex4(CornerType::None {}, vertex(x, y + self.title_bar_height, &theme.title_bar_fill_color))
                    .end()
            }
        };

        let frame_outline = match self.frame_quad.quad_type {
            QuadType::Normal => {
                Rect::new(&name(&self.register_name.frame_outline))
                    .paint_type(PaintType::Stroke {line_width: LINE_WIDTH, joint_type: JointType::None, strip: false, dash: None})
                    .from(vertex(x, y, &theme.frame_outline_color))
                    .to_wh(vertex(width, height, &theme.frame_outline_color))
                    .end()
            }
            QuadType::Rounded { radius } => {
                RectRounded::new(&name(&self.register_name.frame_outline))
                    .paint_type(PaintType::Stroke {line_width: LINE_WIDTH, joint_type: JointType::None, strip: false, dash: None})
                    .from(CornerType::Circle {radius},vertex(x, y, &theme.frame_outline_color))
                    .to_wh(vertex(width, height, &theme.frame_outline_color))
                    .end()
            }
        };

        let title_bar_slice_line = Lines::new(&name(&self.register_name.title_bar_slice_line))
            .paint_type(PaintType::Stroke {line_width: LINE_WIDTH, joint_type: JointType::None, strip: false, dash: None})
            .curve_type(CurveRepresentationType::Normal)
            .point(vertex(x + self.gap.x, y + self.title_bar_height, &theme.title_bar_slice_line_color))
            .last_point(vertex(x - self.gap.x + width, y + self.title_bar_height, &theme.title_bar_slice_line_color))
            .end();

        vec![frame, title_bar, frame_outline, title_bar_slice_line]
    }

    pub fn measure_min_width(&self, resources: &Resources) -> f32 {
        if self.title.is_empty() {
            return self.min_width;
        }

        let Some(font) = resources.font(self.window_font.base_font) else {
            return self.min_width;
        };

        let size = measure(&font, &self.title, &TextStyle::new(self.text_size));
        let title_width = size.width.max(size.ink.right);
        let corner_size = self.frame_quad.quad_type.radius() * 2.0;

        (title_width + self.gap.x * 2.0).max(corner_size)
    }

    fn to_world(&self, draw_manager: &DrawManager, x: f32, y: f32) -> Option<[f32; 2]> {
        let name = self.register_name.title_bar.as_ref()?;
        let object = draw_manager.object(name.as_str())?;

        object.view_camera().screen_to_world(x, y)
    }

    pub fn mouse_left_pressed(&mut self, draw_manager: &mut DrawManager, x: f32, y: f32) -> bool {
        let Some(world) = self.to_world(draw_manager, x, y) else {
            return false;
        };

        if let Some(handle) = self.resize_handle_at(draw_manager, x, y) {
            self.resize = Some(Resize {
                handle,
                start_quad: self.frame_quad,
                start: world,
            });

            return true;
        }

        if self.frame_quad.hover(x, y) {
            self.drag = Some(Drag {
                grab: [world[0] - self.frame_quad.x, world[1] - self.frame_quad.y],
            })
        }

        true
    }

    pub fn mouse_left_released(&mut self, draw_manager: &mut DrawManager, x: f32, y: f32) -> bool {
        let dragged = self.drag.take().is_some();
        let resized = self.resize.take().is_some();

        dragged || resized
    }

    pub fn mouse_right_pressed(&mut self, draw_manager: &mut DrawManager, x: f32, y: f32) -> bool {
        false
    }

    pub fn mouse_right_released(&mut self, draw_manager: &mut DrawManager, x: f32, y: f32) -> bool {
        false
    }

    pub fn mouse_moved(&mut self, draw_manager: &mut DrawManager, x: f32, y: f32) -> bool {
        if self.resize.is_some() {
            return self.resize_moved(draw_manager, x, y);
        }

        let Some(drag) = self.drag.as_ref() else {
            return false;
        };

        let grab = drag.grab;

        let Some(world) = self.to_world(draw_manager, x, y) else {
            return false;
        };

        self.frame_quad.x = world[0] - grab[0];
        self.frame_quad.y = world[1] - grab[1];

        self.pending.offset = true;

        true
    }

    pub fn resize_handle_at(&self, draw_manager: &mut DrawManager, x: f32, y: f32) -> Option<ResizeHandle> {
        let [world_x, world_y] = self.to_world(draw_manager, x, y)?;

        let quad = &self.frame_quad;
        let (left, top) = (quad.x, quad.y);
        let (right, bottom) = (quad.x + quad.width, quad.y + quad.height);

        if world_x < left - HANDLE_OUTSIDE
            || world_x > right + HANDLE_OUTSIDE
            || world_y < top - HANDLE_OUTSIDE
            || world_y > bottom + HANDLE_OUTSIDE
        {
            return None;
        }

        let near_left = world_x <= left + HANDLE_INSIDE;
        let near_right = world_x >= right - HANDLE_INSIDE;
        let near_top = world_y <= top + HANDLE_INSIDE;
        let near_bottom = world_y >= bottom - HANDLE_INSIDE;

        let handle = match (near_left, near_right, near_top, near_bottom) {
            (true, _, true, _) => ResizeHandle::TopLeft,
            (_, true, true, _) => ResizeHandle::TopRight,
            (true, _, _, true) => ResizeHandle::BottomLeft,
            (_, true, _, true) => ResizeHandle::BottomRight,
            (true, _, _, _) => ResizeHandle::Left,
            (_, true, _, _) => ResizeHandle::Right,
            (_, _, true, _) => ResizeHandle::Top,
            (_, _, _, true) => ResizeHandle::Bottom,
            _ => return None,
        };

        if quad.hover(world_x, world_y) {
            match draw_manager.pick(x, y) {
                Some(pick) if self.owns(&pick.name,draw_manager) => {}
                _ => return None,
            }
        }

        Some(handle)
    }

    pub fn resize(&mut self, draw_manager: &mut DrawManager, surface_size: SurfaceSize, design: [f32; 2]) {
        if let Some(register_name) = self.register_name.frame.as_ref() {
            if let Some(object) = draw_manager.object_mut(register_name.as_str()) {
                object.camera_for(surface_size, design, self.scale_mode);
            }
        }
        if let Some(register_name) = self.register_name.title_bar.as_ref() {
            if let Some(object) = draw_manager.object_mut(register_name.as_str()) {
                object.camera_for(surface_size, design, self.scale_mode);
            }
        }
        if let Some(register_name) = self.register_name.frame_outline.as_ref() {
            if let Some(object) = draw_manager.object_mut(register_name.as_str()) {
                object.camera_for(surface_size, design, self.scale_mode);
            }
        }
        if let Some(register_name) = self.register_name.title_bar_slice_line.as_ref() {
            if let Some(object) = draw_manager.object_mut(register_name.as_str()) {
                object.camera_for(surface_size, design, self.scale_mode);
            }
        }
        if let Some(register_name) = self.register_name.text_title.as_ref() {
            draw_manager.with_text_renderer(register_name, |text_title, draw_manager| {
                text_title.camera_for(surface_size, design, self.scale_mode);

                for name in text_title.shape_ids() {
                    if let Some(object) = draw_manager.object_mut(name) {
                        object.camera_for(surface_size, design, self.scale_mode);
                    }
                }
            });
        }

        // for window_item in &mut self.window_items {
        //     for name in window_item.object_names() {
        //         if let Some(object) = draw_manager.object_mut(&name) {
        //             object.camera_for(surface_size, design, self.scale_mode);
        //         }
        //     }
        //
        //     window_item.resize(draw_manager, surface_size, design);
        // }
    }

    fn owns(&self, name: &str, draw_manager: &mut DrawManager) -> bool {
        self.object_names(draw_manager).iter().any(|owned| owned == name)
    }

    fn object_names(&self, draw_manager: &mut DrawManager) -> Vec<String> {
        let mut names = Vec::new();

        for name in [
            self.register_name.frame.as_ref(),
            self.register_name.title_bar.as_ref(),
            self.register_name.frame_outline.as_ref(),
            self.register_name.title_bar_slice_line.as_ref(),
        ]
            .into_iter()
            .flatten()
        {
            names.push(name.clone());
        }

        if let Some(text_title) = &self.register_name.text_title {
            names.extend(draw_manager.group_names(text_title).map(String::from));
        }

        // for window_item in &self.window_items {
        //     names.extend(window_item.object_names());
        // }

        names
    }

    fn resize_moved(&mut self, draw_manager: &mut DrawManager, x: f32, y: f32) -> bool {
        let Some(world) = self.to_world(draw_manager, x, y) else {
            return false;
        };
        let Some(resize) = self.resize.as_ref() else {
            return false;
        };

        let delta = [world[0] - resize.start[0], world[1] - resize.start[1]];
        let quad = self.resized_quad(resize, delta);

        let moved = quad.x != self.frame_quad.x || quad.y != self.frame_quad.y;
        let sized = quad.width != self.frame_quad.width || quad.height != self.frame_quad.height;

        self.frame_quad = quad;

        if sized {
            self.pending.rebuild = true;
        } else if moved {
            self.pending.offset = true;
        }

        true
    }

    fn resized_quad(&self, resize: &Resize, delta: [f32; 2]) -> Quad {
        let start = &resize.start_quad;
        let handle = resize.handle;
        let min_width = self.min_width;
        let min_height = self.min_height();
        let mut quad = *start;

        if handle.moves_left() {
            let right = start.x + start.width;
            quad.x = (start.x + delta[0]).min(right - min_width);
            quad.width = right - quad.x;
        }
        if handle.moves_right() {
            quad.width = (start.width + delta[0]).max(min_width);
        }
        if handle.moves_top() {
            let bottom = start.y + start.height;
            quad.y = (start.y + delta[1]).min(bottom - min_height);
            quad.height = bottom - quad.y;
        }
        if handle.moves_bottom() {
            quad.height = (start.height + delta[1]).max(min_height);
        }

        quad
    }

    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }

    pub fn is_resizing(&self) -> bool {
        self.resize.is_some()
    }

    pub fn min_height(&self) -> f32 {
        self.title_bar_height + self.frame_quad.quad_type.radius() * 2.0
    }
}

#[derive(Default)]
struct Pending {
    rebuild: bool,
    offset: bool,
}

struct Resize {
    handle: ResizeHandle,
    start_quad: Quad,
    start: [f32; 2],
}

struct Drag {
    grab: [f32; 2],
}

struct RegisterName {
    pub frame: Option<String>,
    pub title_bar: Option<String>,
    pub frame_outline: Option<String>,
    pub title_bar_slice_line: Option<String>,
    pub text_title: Option<String>,
}

pub struct WindowFont {
    pub base_font: FontHandle,
}

pub struct ThemeColor {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

pub struct WindowTheme {
    pub frame_fill_color: ThemeColor,
    pub frame_outline_color: ThemeColor,
    pub title_bar_fill_color: ThemeColor,
    pub title_bar_slice_line_color: ThemeColor,
    pub text_title_color: ThemeColor,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum ResizeHandle {
    Left,
    Right,
    Top,
    Bottom,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl ResizeHandle {
    fn moves_left(self) -> bool {
        matches!(self, Self::Left | Self::TopLeft | Self::BottomLeft)
    }

    fn moves_right(self) -> bool {
        matches!(self, Self::Right | Self::TopRight | Self::BottomRight)
    }

    fn moves_top(self) -> bool {
        matches!(self, Self::Top | Self::TopLeft | Self::TopRight)
    }

    fn moves_bottom(self) -> bool {
        matches!(self, Self::Bottom | Self::BottomLeft | Self::BottomRight)
    }
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum QuadType {
    Normal,
    Rounded{
        radius: f32
    }
}

impl QuadType {
    /// 角の半径。角の無い四角は `0.0`。
    pub fn radius(self) -> f32 {
        match self {
            Self::Normal => 0.0,
            Self::Rounded { radius } => radius,
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub struct Quad {
    pub quad_type: QuadType,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Quad {
    pub fn hover(&self, x: f32, y: f32) -> bool {
        self.x < x && self.y < y && self.x + self.width > x && self.y + self.height > y
    }
}

pub struct Gap {
    pub x: f32,
    pub y: f32,
}

impl Gap {

}