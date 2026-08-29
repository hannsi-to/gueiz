use std::cell::RefCell;
use std::fmt::{Display, Formatter};
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
    fn can_create_surfaces(&mut self, application: &Application);
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
        if let WindowEvent::CloseRequested = event {
            let mut window_manager = self.application_state.window_manager.borrow_mut();
            window_manager.remove_window(WindowId(window_id));

            if window_manager.is_empty() {
                event_loop.exit();
            }
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

pub struct WindowDescriptor {
    pub title: String,
    pub width: u32,
    pub height: u32,
}

impl Default for WindowDescriptor {
    fn default() -> Self {
        Self {
            title: String::from("gueiz"),
            width: 1280,
            height: 720,
        }
    }
}

impl From<WindowDescriptor> for winit::window::WindowAttributes {
    fn from(value: WindowDescriptor) -> Self {
        Self::default()
            .with_title(value.title)
            .with_surface_size(LogicalSize::new(value.width, value.height))
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
    window: Box<dyn winit::window::Window>,
}

impl Window {
    fn new(window: Box<dyn winit::window::Window>) -> Self {
        Self {
            window,
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
