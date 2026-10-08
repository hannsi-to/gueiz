use gueiz_gpu::renderer::SurfaceSize;
use crate::draw_manager::DrawManager;

pub trait WindowItem {
    fn create_object(&mut self, item_x: f32, item_y: f32, surface_size: SurfaceSize, design: [f32; 2]);

    fn register_draw_manager(&mut self, draw_manager: &mut DrawManager);
    
    fn resize(&mut self, draw_manager: &mut DrawManager, surface_size: SurfaceSize, design: [f32; 2]) {
        
    }
    
    fn mouse_left_pressed(&mut self, draw_manager: &DrawManager, x: f32, y: f32) -> bool {
        false
    }

    fn mouse_right_pressed(&mut self, draw_manager: &DrawManager, x: f32, y: f32) -> bool {
        false
    }

    fn mouse_left_released(&mut self, draw_manager: &DrawManager, x: f32, y: f32) -> bool {
        false
    }

    fn mouse_right_released(&mut self, draw_manager: &DrawManager, x: f32, y: f32) -> bool {
        false
    }

    fn mouse_moved(&mut self, draw_manager: &mut DrawManager, x: f32, y: f32) -> bool {
        false
    }
}