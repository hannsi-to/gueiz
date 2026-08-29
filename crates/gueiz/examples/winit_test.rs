//! winit 0.31 の複数ウィンドウ最小構成。
//!
//! - 起動時に 2 枚のウィンドウを作成する
//! - `N` キーでウィンドウを追加する
//! - ウィンドウを閉じるとそのウィンドウだけ破棄され、全部閉じるとアプリが終了する
//!
//! 実行:
//! ```sh
//! RUST_LOG=info cargo run -p gueiz --example winit_test
//! ```

use std::collections::HashMap;

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowAttributes, WindowId};

/// 起動時に開くウィンドウの枚数。
const INITIAL_WINDOWS: usize = 2;

#[derive(Default)]
struct App {
    /// WindowId をキーにウィンドウを保持する。複数ウィンドウ管理の基本形。
    windows: HashMap<WindowId, Box<dyn Window>>,
    /// タイトルに振る通し番号。
    next_index: usize,
}

impl App {
    fn spawn_window(&mut self, event_loop: &dyn ActiveEventLoop) {
        let index = self.next_index;
        self.next_index += 1;

        let attributes = WindowAttributes::default()
            .with_title(format!("gueiz window #{index}"))
            .with_surface_size(LogicalSize::new(640.0, 480.0));

        match event_loop.create_window(attributes) {
            Ok(window) => {
                log::info!("created window #{index} ({:?})", window.id());
                self.windows.insert(window.id(), window);
            },
            Err(err) => log::error!("failed to create window #{index}: {err}"),
        }
    }
}

impl ApplicationHandler for App {
    /// サーフェスを作れるようになったタイミングで呼ばれる（旧 `resumed` 相当）。
    /// Android などでは再開のたびに呼ばれうるので、ウィンドウ生成はここに置く。
    fn can_create_surfaces(&mut self, event_loop: &dyn ActiveEventLoop) {
        for _ in 0..INITIAL_WINDOWS {
            self.spawn_window(event_loop);
        }
    }

    fn window_event(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                // 閉じられたウィンドウだけを破棄する。Box を drop するとウィンドウが消える。
                self.windows.remove(&window_id);
                log::info!("closed {window_id:?} ({} remaining)", self.windows.len());

                if self.windows.is_empty() {
                    event_loop.exit();
                }
            },

            WindowEvent::KeyboardInput {
                event: KeyEvent { logical_key, state: ElementState::Pressed, repeat: false, .. },
                ..
            } => match logical_key.as_ref() {
                Key::Character("n") => self.spawn_window(event_loop),
                Key::Named(NamedKey::Escape) => {
                    self.windows.clear();
                    event_loop.exit();
                },
                _ => {},
            },

            WindowEvent::SurfaceResized(size) => {
                log::debug!("{window_id:?} resized to {}x{}", size.width, size.height);
            },

            WindowEvent::RedrawRequested => {
                // 描画するものがまだ無いので何もしない。
                // 将来 Vulkan のスワップチェーンに繋ぐのはこの位置。
                if let Some(window) = self.windows.get(&window_id) {
                    window.pre_present_notify();
                }
            },

            _ => {},
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let event_loop = EventLoop::new()?;
    // イベントが来るまでスリープする。ゲームループにするなら Poll に変更する。
    event_loop.set_control_flow(ControlFlow::Wait);

    log::info!("press 'N' to open a new window, 'Esc' to quit");
    event_loop.run_app(App::default())?;

    Ok(())
}
