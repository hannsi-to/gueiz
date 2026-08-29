use gueiz_window::raw_window_handle::HasWindowHandle;
use gueiz_window::window::{
    Application, ApplicationHandler, ApplicationLoopType, ApplicationRunner, WindowDescriptor,
    WindowSize,
};

struct TestApplication;

impl ApplicationHandler for TestApplication {
    fn can_create_surfaces(&mut self, application: &Application) {
        for index in 0..2 {
            let window_descriptor = WindowDescriptor {
                title: format!("gueiz window #{index}"),
                width: 640,
                height: 480,
            };

            match application.create_window(window_descriptor) {
                Ok(window_id) => log::info!("created window #{index} ({window_id})"),
                Err(error) => log::error!("failed to create window #{index}: {error}"),
            }
        }


        let Some(main_window_id) = application.main_window_id() else {
            log::error!("no window was created");
            application.exit();
            return;
        };

        application.with_window(main_window_id, |window| {
            window.set_title("gueiz main window");
            window.set_min_surface_size(Some(WindowSize::new(320, 240)));

            log::info!("title           : {}", window.title());
            log::info!("surface size    : {:?}", window.surface_size());
            log::info!("outer size      : {:?}", window.outer_size());
            log::info!("scale factor    : {}", window.scale_factor());
            log::info!("safe area       : {:?}", window.safe_area());
            log::info!("theme           : {:?}", window.theme());
            log::info!("resizable       : {}", window.is_resizable());
            log::info!("decorated       : {}", window.is_decorated());
            log::info!("outer position  : {:?}", window.outer_position());
            log::info!("activation token: {:?}", window.request_activation_token());
            log::info!("window handle   : {:?}", window.window_handle().is_ok());

            window.request_redraw();
        });
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let mut application_runner = ApplicationRunner::new();
    application_runner.set_application_loop_type(ApplicationLoopType::Wait);
    application_runner.run_application(TestApplication)?;

    Ok(())
}
