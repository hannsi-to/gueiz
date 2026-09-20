use std::cell::RefCell;
use std::fmt::{Display, Formatter};
use std::sync::Arc;
use std::time::Instant;

use fxhash::FxHashMap;
use winit::dpi::{LogicalSize, PhysicalInsets, PhysicalPosition, PhysicalSize};
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::monitor::Fullscreen;
#[cfg(any(
    target_os = "linux",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd"
))]
use winit::platform::startup_notify::WindowExtStartupNotify;
use winit::raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle,
};

use crate::error::GueizWindowError;
use crate::error::GueizWindowError::{
    ApplicationError, UnknownWindowIdError, WindowCreationError, WindowRequestError,
};

pub trait ApplicationHandler {
    /// 描き先を作れるようになった。ここで窓を開く。
    fn can_create_surfaces(&mut self, application: &Application);

    /// 窓を描き直す番が来た。何もしないのが既定。
    fn redraw_requested(&mut self, application: &Application, window_id: WindowId) {
        let _ = (application, window_id);
    }

    /// 窓の大きさが変わった。描き先も張り直すこと。何もしないのが既定。
    fn surface_resized(
        &mut self,
        application: &Application,
        window_id: WindowId,
        surface_size: WindowSize,
    ) {
        let _ = (application, window_id, surface_size);
    }

    /// 窓が閉じられた。呼ばれた時点で、その窓はもう一覧にいない。
    ///
    /// 何もしないのが既定。最後の 1 枚が閉じれば、このあと自動で終わる。
    fn window_closed(&mut self, application: &Application, window_id: WindowId) {
        let _ = (application, window_id);
    }

    /// 窓の上で指し手が動いた。何もしないのが既定。
    ///
    /// 位置は**窓の左上から数えた画素**。画面の左上からではない。
    fn cursor_moved(
        &mut self,
        application: &Application,
        window_id: WindowId,
        cursor_position: CursorPosition,
    ) {
        let _ = (application, window_id, cursor_position);
    }

    /// 指し手が窓から出た。何もしないのが既定。
    ///
    /// 押したまま窓の外へ出たときは呼ばれない。掴んでいる間は窓が
    /// 指し手を握り続けるので、[`ApplicationHandler::cursor_moved`] が
    /// 窓の外の位置で届く。
    fn cursor_left(&mut self, application: &Application, window_id: WindowId) {
        let _ = (application, window_id);
    }

    /// 窓の上で釦が押された、または離された。何もしないのが既定。
    ///
    /// 触りや筆で触れたときも、当たる釦に読み替えて届く。
    fn mouse_input(
        &mut self,
        application: &Application,
        window_id: WindowId,
        button_state: ButtonState,
        mouse_button: MouseButton,
        cursor_position: CursorPosition,
    ) {
        let _ = (application, window_id, button_state, mouse_button, cursor_position);
    }
}

pub struct ApplicationRunner {
    application_context: ApplicationContext,
    event_loop: EventLoop,
}

impl ApplicationRunner {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_application_loop_type(&mut self, application_loop_type: ApplicationLoopType) {
        self.application_context.application_loop_type = application_loop_type;
    }

    pub fn run_application<A: ApplicationHandler + 'static>(
        self,
        application_handler: A,
    ) -> Result<(), GueizWindowError> {
        let winit_handler = WinitApplicationHandler {
            application_handler,
            application_state: ApplicationState {
                application_context: RefCell::new(self.application_context),
                window_manager: RefCell::new(WindowManager::new()),
            },
        };

        self.event_loop.run_app(winit_handler).map_err(ApplicationError)
    }
}

impl Default for ApplicationRunner {
    fn default() -> Self {
        Self {
            application_context: Default::default(),
            event_loop: EventLoop::new().unwrap(),
        }
    }
}

struct WinitApplicationHandler<A: ApplicationHandler> {
    application_handler: A,
    application_state: ApplicationState,
}

impl<A: ApplicationHandler> winit::application::ApplicationHandler for WinitApplicationHandler<A> {
    fn can_create_surfaces(&mut self, event_loop: &dyn ActiveEventLoop) {
        let Self { application_handler, application_state } = self;

        let application = Application { active_event_loop: event_loop, application_state };
        application_handler.can_create_surfaces(&application);
    }

    fn window_event(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        let Self { application_handler, application_state } = self;

        let window_id = WindowId(window_id);
        let application = Application { active_event_loop: event_loop, application_state };

        match event {
            WindowEvent::CloseRequested => {
                // 呼ぶ側に知らせる前に一覧から外す。知らせている最中に
                // `with_window` を呼ばれても、閉じた窓は見えない。
                application_state
                    .window_manager
                    .borrow_mut()
                    .remove_window(window_id);

                application_handler.window_closed(&application, window_id);

                if application_state.window_manager.borrow().is_empty() {
                    event_loop.exit();
                }
            }

            WindowEvent::SurfaceResized(surface_size) => {
                application_handler.surface_resized(&application, window_id, surface_size.into());
            }

            WindowEvent::RedrawRequested => {
                application_handler.redraw_requested(&application, window_id);
            }

            WindowEvent::PointerMoved { position, .. } => {
                application_handler.cursor_moved(&application, window_id, position.into());
            }

            WindowEvent::PointerLeft { .. } => {
                application_handler.cursor_left(&application, window_id);
            }

            WindowEvent::PointerButton { state, position, button, .. } => {
                // 触りや筆は当たる釦に読み替える。読み替えられないものは捨てる。
                if let Some(mouse_button) = button.mouse_button() {
                    application_handler.mouse_input(
                        &application,
                        window_id,
                        state.into(),
                        mouse_button.into(),
                        position.into(),
                    );
                }
            }

            _ => {}
        }
    }
}

struct ApplicationState {
    application_context: RefCell<ApplicationContext>,
    window_manager: RefCell<WindowManager>,
}

pub struct Application<'a> {
    active_event_loop: &'a dyn ActiveEventLoop,
    application_state: &'a ApplicationState,
}

impl<'a> Application<'a> {
    pub fn create_window(
        &self,
        window_descriptor: WindowDescriptor,
    ) -> Result<WindowId, GueizWindowError> {
        self.application_state
            .window_manager
            .borrow_mut()
            .create_window(self.active_event_loop, window_descriptor)
    }

    pub fn set_main_window_id(&self, main_window_id: WindowId) -> Result<(), GueizWindowError> {
        self.application_state.window_manager.borrow_mut().set_main_window_id(main_window_id)
    }

    pub fn main_window_id(&self) -> Option<WindowId> {
        self.application_state.window_manager.borrow().main_window_id
    }

    pub fn set_application_loop_type(&self, application_loop_type: ApplicationLoopType) {
        self.application_state
            .application_context
            .borrow_mut()
            .application_loop_type = application_loop_type;
    }

    pub fn with_window<R>(
        &self,
        window_id: WindowId,
        f: impl FnOnce(&Window) -> R,
    ) -> Option<R> {
        self.application_state.window_manager.borrow().window(window_id).map(f)
    }

    pub fn with_main_window<R>(&self, f: impl FnOnce(&Window) -> R) -> Option<R> {
        let window_manager = self.application_state.window_manager.borrow();
        window_manager.main_window_id.and_then(|window_id| window_manager.window(window_id)).map(f)
    }

    pub fn exit(&self) {
        self.active_event_loop.exit();
    }
}

struct ApplicationContext {
    application_loop_type: ApplicationLoopType,
}

impl Default for ApplicationContext {
    fn default() -> Self {
        Self {
            application_loop_type: ApplicationLoopType::Poll,
        }
    }
}

#[derive(PartialEq)]
pub enum ApplicationLoopType {
    Poll,
    Wait,
    WaitUntil(Instant),
}

impl From<&ApplicationLoopType> for winit::event_loop::ControlFlow {
    fn from(value: &ApplicationLoopType) -> Self {
        match value {
            ApplicationLoopType::Poll => Self::Poll,
            ApplicationLoopType::Wait => Self::Wait,
            ApplicationLoopType::WaitUntil(time) => Self::WaitUntil(*time),
        }
    }
}

/// 窓の作り方。[`Application::create_window`] に渡す。
///
/// 既定は「普通のアプリの窓」。ウィジェットのように飾りの無い窓を出すなら、
/// `decorations` を `false`、`transparent` を `true` にする。
///
/// # 生成時にしか効かないもの
///
/// `transparent` と `skip_taskbar` は **窓を作るときにしか決められません。**
/// Windows の透明化は `DwmEnableBlurBehindWindow` で行いますが、winit が
/// これを呼ぶのは窓を作る瞬間だけです。あとから [`Window::set_transparent`]
/// を呼んでも見た目は変わりません。
///
/// # 例
///
/// ```no_run
/// use gueiz_window::window::{WindowDescriptor, WindowLevel};
///
/// let widget = WindowDescriptor {
///     title: String::from("clock"),
///     width: 240,
///     height: 120,
///     transparent: true,
///     decorations: false,
///     skip_taskbar: true,
///     window_level: WindowLevel::AlwaysOnBottom,
///     active: false,
///     ..Default::default()
/// };
/// ```
#[derive(Clone)]
#[derive(Debug)]
pub struct WindowDescriptor {
    pub title: String,
    pub width: u32,
    pub height: u32,
    /// 置く場所。`None` なら OS に任せる。
    pub position: Option<WindowPosition>,
    /// 背景を透かす。**生成時にしか効かない。**
    ///
    /// 描く側でも揃える必要がある。GPU 側で合成方法を
    /// `SurfaceAlphaMode::PreMultiplied` にし、消す色のアルファを 0 にすること。
    /// どれか一つでも欠けると透けない。
    pub transparent: bool,
    /// 枠と題名の帯を出す。
    pub decorations: bool,
    /// 窓の前後。ウィジェットなら [`WindowLevel::AlwaysOnBottom`]。
    pub window_level: WindowLevel,
    /// 作った直後から見せる。
    pub visible: bool,
    /// 縁を掴んで大きさを変えられる。
    pub resizable: bool,
    /// 作った直後に前へ出して入力を奪う。ウィジェットなら `false`。
    pub active: bool,
    /// タスクバーと Alt+Tab から隠す。**生成時にしか効かない。**
    ///
    /// Windows だけの指定。ほかの OS では黙って無視される。
    pub skip_taskbar: bool,
}

impl Default for WindowDescriptor {
    fn default() -> Self {
        Self {
            title: String::from("gueiz"),
            width: 1280,
            height: 720,
            position: None,
            transparent: false,
            decorations: true,
            window_level: WindowLevel::Normal,
            visible: true,
            resizable: true,
            active: true,
            skip_taskbar: false,
        }
    }
}

impl From<WindowDescriptor> for winit::window::WindowAttributes {
    fn from(value: WindowDescriptor) -> Self {
        let mut window_attributes = Self::default()
            .with_title(value.title)
            .with_surface_size(LogicalSize::new(value.width, value.height))
            .with_transparent(value.transparent)
            .with_decorations(value.decorations)
            .with_window_level(value.window_level.into())
            .with_visible(value.visible)
            .with_resizable(value.resizable)
            .with_active(value.active);

        if let Some(position) = value.position {
            window_attributes = window_attributes.with_position(PhysicalPosition::from(position));
        }

        // タスクバーから隠すのは Windows 固有の作法。winit では窓を作るときの
        // プラットフォーム別の指定として渡す。
        #[cfg(target_os = "windows")]
        {
            use winit::platform::windows::WindowAttributesWindows;

            window_attributes = window_attributes.with_platform_attributes(Box::new(
                WindowAttributesWindows::default().with_skip_taskbar(value.skip_taskbar),
            ));
        }

        window_attributes
    }
}

struct WindowManager {
    main_window_id: Option<WindowId>,
    windows: FxHashMap<WindowId, Window>,
}

impl WindowManager {
    pub(crate) fn new() -> Self {
        Self {
            main_window_id: None,
            windows: FxHashMap::default(),
        }
    }

    pub fn create_window(
        &mut self,
        active_event_loop: &dyn ActiveEventLoop,
        window_descriptor: WindowDescriptor,
    ) -> Result<WindowId, GueizWindowError> {
        let window = active_event_loop
            .create_window(window_descriptor.into())
            .map_err(WindowCreationError)?;
        let window_id = WindowId(window.id());

        self.windows.insert(window_id, Window::new(window));

        if self.main_window_id.is_none() {
            self.main_window_id = Some(window_id);
        }

        Ok(window_id)
    }

    pub fn remove_window(&mut self, window_id: WindowId) {
        self.windows.remove(&window_id);

        if self.main_window_id == Some(window_id) {
            self.main_window_id = self.windows.keys().copied().next();
        }
    }

    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }

    pub fn window(&self, window_id: WindowId) -> Option<&Window> {
        self.windows.get(&window_id)
    }

    pub fn set_main_window_id(&mut self, main_window_id: WindowId) -> Result<(), GueizWindowError> {
        if !self.windows.contains_key(&main_window_id) {
            Err(UnknownWindowIdError(main_window_id))
        } else {
            self.main_window_id = Some(main_window_id);
            Ok(())
        }
    }
}

#[derive(Eq, Hash, PartialEq)]
#[derive(Clone, Copy)]
#[derive(Debug)]
pub struct WindowId(winit::window::WindowId);

impl Display for WindowId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

pub struct Window {
    window: Arc<dyn winit::window::Window>,
}

impl Window {
    fn new(window: Box<dyn winit::window::Window>) -> Self {
        Self {
            window: Arc::from(window),
        }
    }

    /// 窓より長生きできる持ち手を取る。GPU の描き先を作るのに使う。
    ///
    /// 詳しくは [`WindowSurfaceHandle`]。
    pub fn surface_handle(&self) -> WindowSurfaceHandle {
        WindowSurfaceHandle {
            window: Arc::clone(&self.window),
        }
    }

    pub fn id(&self) -> WindowId {
        WindowId(self.window.id())
    }

    pub fn scale_factor(&self) -> f64 {
        self.window.scale_factor()
    }

    pub fn request_redraw(&self) {
        self.window.request_redraw();
    }

    pub fn pre_present_notify(&self) {
        self.window.pre_present_notify();
    }

    pub fn reset_dead_keys(&self) {
        self.window.reset_dead_keys();
    }

    pub fn surface_size(&self) -> WindowSize {
        self.window.surface_size().into()
    }

    pub fn request_surface_size(&self, surface_size: WindowSize) -> Option<WindowSize> {
        self.window
            .request_surface_size(PhysicalSize::from(surface_size).into())
            .map(Into::into)
    }

    pub fn outer_size(&self) -> WindowSize {
        self.window.outer_size().into()
    }

    pub fn set_min_surface_size(&self, min_surface_size: Option<WindowSize>) {
        self.window
            .set_min_surface_size(min_surface_size.map(|size| PhysicalSize::from(size).into()));
    }

    pub fn set_max_surface_size(&self, max_surface_size: Option<WindowSize>) {
        self.window
            .set_max_surface_size(max_surface_size.map(|size| PhysicalSize::from(size).into()));
    }

    pub fn surface_resize_increments(&self) -> Option<WindowSize> {
        self.window.surface_resize_increments().map(Into::into)
    }

    pub fn set_surface_resize_increments(&self, surface_resize_increments: Option<WindowSize>) {
        self.window.set_surface_resize_increments(
            surface_resize_increments.map(|size| PhysicalSize::from(size).into()),
        );
    }

    pub fn safe_area(&self) -> WindowInsets {
        self.window.safe_area().into()
    }

    pub fn surface_position(&self) -> WindowPosition {
        self.window.surface_position().into()
    }

    pub fn outer_position(&self) -> Result<WindowPosition, GueizWindowError> {
        self.window
            .outer_position()
            .map(Into::into)
            .map_err(WindowRequestError)
    }

    pub fn set_outer_position(&self, outer_position: WindowPosition) {
        self.window
            .set_outer_position(PhysicalPosition::from(outer_position).into());
    }

    pub fn title(&self) -> String {
        self.window.title()
    }

    pub fn set_title(&self, title: &str) {
        self.window.set_title(title);
    }

    pub fn set_transparent(&self, transparent: bool) {
        self.window.set_transparent(transparent);
    }

    pub fn set_blur(&self, blur: bool) {
        self.window.set_blur(blur);
    }

    pub fn set_decorations(&self, decorations: bool) {
        self.window.set_decorations(decorations);
    }

    pub fn is_decorated(&self) -> bool {
        self.window.is_decorated()
    }

    pub fn set_window_level(&self, window_level: WindowLevel) {
        self.window.set_window_level(window_level.into());
    }

    pub fn set_content_protected(&self, content_protected: bool) {
        self.window.set_content_protected(content_protected);
    }

    pub fn theme(&self) -> Option<WindowTheme> {
        self.window.theme().map(Into::into)
    }

    pub fn set_theme(&self, window_theme: Option<WindowTheme>) {
        self.window.set_theme(window_theme.map(Into::into));
    }

    pub fn set_visible(&self, visible: bool) {
        self.window.set_visible(visible);
    }

    pub fn is_visible(&self) -> Option<bool> {
        self.window.is_visible()
    }

    pub fn set_resizable(&self, resizable: bool) {
        self.window.set_resizable(resizable);
    }

    pub fn is_resizable(&self) -> bool {
        self.window.is_resizable()
    }

    pub fn set_minimized(&self, minimized: bool) {
        self.window.set_minimized(minimized);
    }

    pub fn is_minimized(&self) -> Option<bool> {
        self.window.is_minimized()
    }

    pub fn set_maximized(&self, maximized: bool) {
        self.window.set_maximized(maximized);
    }

    pub fn is_maximized(&self) -> bool {
        self.window.is_maximized()
    }

    pub fn set_fullscreen(&self, fullscreen: bool) {
        self.window
            .set_fullscreen(fullscreen.then_some(Fullscreen::Borderless(None)));
    }

    pub fn is_fullscreen(&self) -> bool {
        self.window.fullscreen().is_some()
    }

    pub fn focus_window(&self) {
        self.window.focus_window();
    }

    pub fn has_focus(&self) -> bool {
        self.window.has_focus()
    }

    pub fn request_user_attention(&self, user_attention_type: Option<UserAttentionType>) {
        self.window
            .request_user_attention(user_attention_type.map(Into::into));
    }

    pub fn set_cursor_visible(&self, visible: bool) {
        self.window.set_cursor_visible(visible);
    }

    pub fn set_cursor_position(&self, position: WindowPosition) -> Result<(), GueizWindowError> {
        self.window
            .set_cursor_position(PhysicalPosition::from(position).into())
            .map_err(WindowRequestError)
    }

    pub fn set_cursor_grab_mode(&self, cursor_grab_mode: CursorGrabMode) -> Result<(), GueizWindowError> {
        self.window
            .set_cursor_grab(cursor_grab_mode.into())
            .map_err(WindowRequestError)
    }

    pub fn set_cursor_hittest(&self, cursor_hittest: bool) -> Result<(), GueizWindowError> {
        self.window
            .set_cursor_hittest(cursor_hittest)
            .map_err(WindowRequestError)
    }

    pub fn drag_window(&self) -> Result<(), GueizWindowError> {
        self.window.drag_window().map_err(WindowRequestError)
    }

    pub fn drag_resize_window(&self, resize_direction: ResizeDirection) -> Result<(), GueizWindowError> {
        self.window
            .drag_resize_window(resize_direction.into())
            .map_err(WindowRequestError)
    }

    pub fn request_activation_token(&self) -> Result<(), GueizWindowError> {
        #[cfg(any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        ))]
        {
            self.window
                .request_activation_token()
                .map(|_serial| ())
                .map_err(WindowRequestError)
        }

        #[cfg(not(any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        )))]
        {
            Err(GueizWindowError::NotSupportedError(
                gueiz_core::error::GueizCoreError::NotSupportedError(String::from(
                    "request_activation_token",
                )),
            ))
        }
    }
}

impl HasDisplayHandle for Window {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        self.window.rwh_06_display_handle().display_handle()
    }
}

impl HasWindowHandle for Window {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        self.window.rwh_06_window_handle().window_handle()
    }
}

/// 窓の持ち手。[`Window::surface_handle`] で取る。
///
/// GPU の描き先（`wgpu` のサーフェスなど）は `'static` な持ち手を欲しがりますが、
/// 窓を持っているのは [`Application`] で、呼ぶ側は借りることしかできません。
/// これはその橋渡しです。生のハンドルを返すだけなので、`wgpu` の
/// `create_surface` にそのまま渡せます。
///
/// 窓を閉じても、この持ち手が残っているかぎり窓は解放されません。
/// **描き先を捨てるときに、これも一緒に捨てること。**
#[derive(Clone)]
pub struct WindowSurfaceHandle {
    window: Arc<dyn winit::window::Window>,
}

impl HasDisplayHandle for WindowSurfaceHandle {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        self.window.rwh_06_display_handle().display_handle()
    }
}

impl HasWindowHandle for WindowSurfaceHandle {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        self.window.rwh_06_window_handle().window_handle()
    }
}

#[derive(Clone, Copy)]
#[derive(Eq, PartialEq)]
#[derive(Debug)]
pub struct WindowSize {
    pub width: u32,
    pub height: u32,
}

impl WindowSize {
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }
}

impl From<PhysicalSize<u32>> for WindowSize {
    fn from(value: PhysicalSize<u32>) -> Self {
        Self::new(value.width, value.height)
    }
}

impl From<WindowSize> for PhysicalSize<u32> {
    fn from(value: WindowSize) -> Self {
        Self::new(value.width, value.height)
    }
}

#[derive(Clone, Copy)]
#[derive(Eq, PartialEq)]
#[derive(Debug)]
pub struct WindowPosition {
    pub x: i32,
    pub y: i32,
}

impl WindowPosition {
    pub fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

impl From<PhysicalPosition<i32>> for WindowPosition {
    fn from(value: PhysicalPosition<i32>) -> Self {
        Self::new(value.x, value.y)
    }
}

impl From<WindowPosition> for PhysicalPosition<i32> {
    fn from(value: WindowPosition) -> Self {
        Self::new(value.x, value.y)
    }
}

#[derive(Clone, Copy)]
#[derive(Eq, PartialEq)]
#[derive(Debug)]
pub struct WindowInsets {
    pub top: u32,
    pub left: u32,
    pub bottom: u32,
    pub right: u32,
}

impl From<PhysicalInsets<u32>> for WindowInsets {
    fn from(value: PhysicalInsets<u32>) -> Self {
        Self {
            top: value.top,
            left: value.left,
            bottom: value.bottom,
            right: value.right,
        }
    }
}

#[derive(Clone, Copy)]
#[derive(Eq, PartialEq)]
#[derive(Debug)]
pub enum WindowLevel {
    AlwaysOnBottom,
    Normal,
    AlwaysOnTop,
}

impl From<WindowLevel> for winit::window::WindowLevel {
    fn from(value: WindowLevel) -> Self {
        match value {
            WindowLevel::AlwaysOnBottom => Self::AlwaysOnBottom,
            WindowLevel::Normal => Self::Normal,
            WindowLevel::AlwaysOnTop => Self::AlwaysOnTop,
        }
    }
}

#[derive(Clone, Copy)]
#[derive(Eq, PartialEq)]
#[derive(Debug)]
pub enum WindowTheme {
    Light,
    Dark,
}

impl From<winit::window::Theme> for WindowTheme {
    fn from(value: winit::window::Theme) -> Self {
        match value {
            winit::window::Theme::Light => Self::Light,
            winit::window::Theme::Dark => Self::Dark,
        }
    }
}

impl From<WindowTheme> for winit::window::Theme {
    fn from(value: WindowTheme) -> Self {
        match value {
            WindowTheme::Light => Self::Light,
            WindowTheme::Dark => Self::Dark,
        }
    }
}

#[derive(Clone, Copy)]
#[derive(Eq, PartialEq)]
#[derive(Debug)]
pub enum CursorGrabMode {
    None,
    Confined,
    Locked,
}

impl From<CursorGrabMode> for winit::window::CursorGrabMode {
    fn from(value: CursorGrabMode) -> Self {
        match value {
            CursorGrabMode::None => Self::None,
            CursorGrabMode::Confined => Self::Confined,
            CursorGrabMode::Locked => Self::Locked,
        }
    }
}

#[derive(Clone, Copy)]
#[derive(Eq, PartialEq)]
#[derive(Debug)]
pub enum UserAttentionType {
    Critical,
    Informational,
}

impl From<UserAttentionType> for winit::window::UserAttentionType {
    fn from(value: UserAttentionType) -> Self {
        match value {
            UserAttentionType::Critical => Self::Critical,
            UserAttentionType::Informational => Self::Informational,
        }
    }
}

#[derive(Clone, Copy)]
#[derive(Eq, PartialEq)]
#[derive(Debug)]
pub enum ResizeDirection {
    East,
    North,
    NorthEast,
    NorthWest,
    South,
    SouthEast,
    SouthWest,
    West,
}

impl From<ResizeDirection> for winit::window::ResizeDirection {
    fn from(value: ResizeDirection) -> Self {
        match value {
            ResizeDirection::East => Self::East,
            ResizeDirection::North => Self::North,
            ResizeDirection::NorthEast => Self::NorthEast,
            ResizeDirection::NorthWest => Self::NorthWest,
            ResizeDirection::South => Self::South,
            ResizeDirection::SouthEast => Self::SouthEast,
            ResizeDirection::SouthWest => Self::SouthWest,
            ResizeDirection::West => Self::West,
        }
    }
}

/// 指し手の位置。**窓の左上から数えた画素。**
///
/// 画素の間を指せるので整数ではない。窓を掴んで動かすときは、掴んだ場所と
/// 今の場所の差だけ窓を動かせばよい。
#[derive(Clone, Copy)]
#[derive(PartialEq)]
#[derive(Debug, Default)]
pub struct CursorPosition {
    pub x: f64,
    pub y: f64,
}

impl CursorPosition {
    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

impl From<PhysicalPosition<f64>> for CursorPosition {
    fn from(value: PhysicalPosition<f64>) -> Self {
        Self::new(value.x, value.y)
    }
}

/// 釦が押されたか、離されたか。
#[derive(Clone, Copy)]
#[derive(Eq, PartialEq)]
#[derive(Debug)]
pub enum ButtonState {
    Pressed,
    Released,
}

impl From<winit::event::ElementState> for ButtonState {
    fn from(value: winit::event::ElementState) -> Self {
        match value {
            winit::event::ElementState::Pressed => Self::Pressed,
            winit::event::ElementState::Released => Self::Released,
        }
    }
}

/// 鼠の釦。
#[derive(Clone, Copy)]
#[derive(Eq, PartialEq)]
#[derive(Debug)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    /// 横の釦。戻るに割り当てられていることが多い。
    Back,
    /// 横の釦。進むに割り当てられていることが多い。
    Forward,
    /// 6 個目から先。数は winit の並びのまま。
    Other(u8),
}

impl From<winit::event::MouseButton> for MouseButton {
    fn from(value: winit::event::MouseButton) -> Self {
        match value {
            winit::event::MouseButton::Left => Self::Left,
            winit::event::MouseButton::Right => Self::Right,
            winit::event::MouseButton::Middle => Self::Middle,
            winit::event::MouseButton::Back => Self::Back,
            winit::event::MouseButton::Forward => Self::Forward,
            other => Self::Other(other as u8),
        }
    }
}
