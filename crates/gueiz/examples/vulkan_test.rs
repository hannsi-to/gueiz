//! Vulkan の最小構成。ウィンドウを 1 枚開き、画面をクリアするだけ。
//!
//! instance → surface → physical device → device → swapchain → render pass →
//! framebuffer → command buffer → 同期オブジェクト、という Vulkan 初期化の
//! 最短経路をひと通り通す。パイプラインも頂点バッファもまだ無い。
//!
//! ```sh
//! RUST_LOG=info cargo run -p gueiz --example vulkan_test
//! ```
//!
//! macOS では MoltenVK が必要。詳細は [`VulkanRenderer::create_instance`] のコメント。

use std::error::Error;
use std::ffi::c_char;

use ash::vk;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowAttributes, WindowId};

/// GPU の処理を待たずに CPU が先行できるフレーム数。
const MAX_FRAMES_IN_FLIGHT: usize = 2;

fn main() -> Result<(), Box<dyn Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let event_loop = EventLoop::new()?;
    // クリア色をアニメーションさせるので毎フレーム回す。
    event_loop.set_control_flow(ControlFlow::Poll);
    event_loop.run_app(VulkanApplication::default())?;

    Ok(())
}

#[derive(Default)]
struct VulkanApplication {
    // 宣言順 = drop 順。サーフェスを持つレンダラをウィンドウより先に落とす。
    renderer: Option<VulkanRenderer>,
    window: Option<Box<dyn Window>>,
    frame_index: u64,
}

impl ApplicationHandler for VulkanApplication {
    fn can_create_surfaces(&mut self, event_loop: &dyn ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let window_attributes = WindowAttributes::default()
            .with_title("gueiz vulkan")
            .with_surface_size(LogicalSize::new(1280.0, 720.0));

        let window = match event_loop.create_window(window_attributes) {
            Ok(window) => window,
            Err(error) => {
                log::error!("failed to create window: {error}");
                event_loop.exit();
                return;
            }
        };

        match VulkanRenderer::new(window.as_ref()) {
            Ok(renderer) => self.renderer = Some(renderer),
            Err(error) => {
                log::error!("failed to initialize vulkan: {error}");
                event_loop.exit();
                return;
            }
        }

        self.window = Some(window);
    }

    fn window_event(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        let (Some(window), Some(renderer)) = (self.window.as_ref(), self.renderer.as_mut()) else {
            return;
        };

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),

            WindowEvent::SurfaceResized(_) => {
                // 実サイズは swapchain 再生成時に surface capabilities から読む。
                renderer.invalidate_swapchain();
                window.request_redraw();
            }

            WindowEvent::RedrawRequested => {
                // 最小化中はサイズ 0 になり swapchain を作れないので何もしない。
                let surface_size = window.surface_size();
                if surface_size.width == 0 || surface_size.height == 0 {
                    return;
                }

                // 時間とともに色相が回るクリア色。描画できている証拠になる。
                let phase = self.frame_index as f32 * 0.01;
                let clear_color = [
                    phase.sin().mul_add(0.5, 0.5),
                    (phase + 2.094).sin().mul_add(0.5, 0.5),
                    (phase + 4.189).sin().mul_add(0.5, 0.5),
                    1.0,
                ];
                self.frame_index += 1;

                window.pre_present_notify();
                if let Err(error) = renderer.draw_frame(clear_color) {
                    log::error!("failed to draw frame: {error}");
                    event_loop.exit();
                }
            }

            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &dyn ActiveEventLoop) {
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }

    /// サスペンド時などにサーフェスが失われる。winit 0.31 では `exiting` ではなく
    /// これが「サーフェスを捨てろ」の合図。次の `can_create_surfaces` で作り直す。
    fn destroy_surfaces(&mut self, _event_loop: &dyn ActiveEventLoop) {
        self.renderer = None;
        self.window = None;
    }
}

/// Vulkan オブジェクト一式。`Drop` で生成と逆順に破棄する。
struct VulkanRenderer {
    // 破棄順を明示するため、フィールド順 = 破棄順にしてある（Rust は宣言順に drop する）。
    // ただし Vulkan ハンドルは Drop を持たないので、実際の破棄は `Drop for VulkanRenderer`。
    _entry: ash::Entry,
    instance: ash::Instance,

    surface_instance: ash::khr::surface::Instance,
    surface: vk::SurfaceKHR,

    physical_device: vk::PhysicalDevice,
    graphics_queue_family_index: u32,
    present_queue_family_index: u32,

    device: ash::Device,
    graphics_queue: vk::Queue,
    present_queue: vk::Queue,

    swapchain_device: ash::khr::swapchain::Device,
    swapchain: Swapchain,
    /// リサイズや OUT_OF_DATE を受けたら次のフレームで作り直す。
    swapchain_invalidated: bool,

    render_pass: vk::RenderPass,

    command_pool: vk::CommandPool,
    command_buffers: Vec<vk::CommandBuffer>,

    /// CPU が GPU を追い越さないためのフレーム単位の同期。
    image_available_semaphores: Vec<vk::Semaphore>,
    in_flight_fences: Vec<vk::Fence>,
    /// present を待たせる用。スワップチェーン「イメージ」単位で持つ。
    /// フレーム単位にすると、まだ present 中のセマフォを再利用してしまう。
    render_finished_semaphores: Vec<vk::Semaphore>,

    current_frame: usize,
}

/// スワップチェーンとその派生物。リサイズのたびにまとめて作り直す。
struct Swapchain {
    handle: vk::SwapchainKHR,
    format: vk::Format,
    extent: vk::Extent2D,
    image_views: Vec<vk::ImageView>,
    framebuffers: Vec<vk::Framebuffer>,
}

impl VulkanRenderer {
    fn new(window: &dyn Window) -> Result<Self, Box<dyn Error>> {
        // ローダを dlopen する。ここで失敗するなら Vulkan 実装が入っていない。
        let entry = unsafe { ash::Entry::load() }?;

        let display_handle = window.display_handle()?.as_raw();
        let window_handle = window.window_handle()?.as_raw();

        let instance = Self::create_instance(&entry, display_handle)?;

        let surface_instance = ash::khr::surface::Instance::new(&entry, &instance);
        let surface = unsafe {
            ash_window::create_surface(&entry, &instance, display_handle, window_handle, None)
        }?;

        let (physical_device, graphics_queue_family_index, present_queue_family_index) =
            Self::select_physical_device(&instance, &surface_instance, surface)?;

        let (device, graphics_queue, present_queue) = Self::create_device(
            &instance,
            physical_device,
            graphics_queue_family_index,
            present_queue_family_index,
        )?;

        let swapchain_device = ash::khr::swapchain::Device::new(&instance, &device);

        // フォーマットは swapchain 生成前に決めないと render pass を作れない。
        let surface_format = Self::select_surface_format(
            &surface_instance,
            physical_device,
            surface,
        )?;
        let render_pass = Self::create_render_pass(&device, surface_format.format)?;

        let swapchain = Swapchain::new(
            &surface_instance,
            &swapchain_device,
            &device,
            physical_device,
            surface,
            surface_format,
            graphics_queue_family_index,
            present_queue_family_index,
            render_pass,
            vk::SwapchainKHR::null(),
        )?;

        let command_pool = unsafe {
            device.create_command_pool(
                &vk::CommandPoolCreateInfo::default()
                    // フレームごとにコマンドバッファを録り直す。
                    .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER)
                    .queue_family_index(graphics_queue_family_index),
                None,
            )
        }?;

        let command_buffers = unsafe {
            device.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(command_pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(MAX_FRAMES_IN_FLIGHT as u32),
            )
        }?;

        let mut image_available_semaphores = Vec::with_capacity(MAX_FRAMES_IN_FLIGHT);
        let mut in_flight_fences = Vec::with_capacity(MAX_FRAMES_IN_FLIGHT);
        for _ in 0..MAX_FRAMES_IN_FLIGHT {
            image_available_semaphores
                .push(unsafe { device.create_semaphore(&vk::SemaphoreCreateInfo::default(), None) }?);
            // 1 フレーム目で待てるようシグナル済みで作る。
            in_flight_fences.push(unsafe {
                device.create_fence(
                    &vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED),
                    None,
                )
            }?);
        }

        let mut render_finished_semaphores = Vec::with_capacity(swapchain.image_views.len());
        for _ in 0..swapchain.image_views.len() {
            render_finished_semaphores
                .push(unsafe { device.create_semaphore(&vk::SemaphoreCreateInfo::default(), None) }?);
        }

        log::info!(
            "vulkan ready: {}x{} / {:?} / {} images",
            swapchain.extent.width,
            swapchain.extent.height,
            swapchain.format,
            swapchain.image_views.len()
        );

        Ok(Self {
            _entry: entry,
            instance,
            surface_instance,
            surface,
            physical_device,
            graphics_queue_family_index,
            present_queue_family_index,
            device,
            graphics_queue,
            present_queue,
            swapchain_device,
            swapchain,
            swapchain_invalidated: false,
            render_pass,
            command_pool,
            command_buffers,
            image_available_semaphores,
            in_flight_fences,
            render_finished_semaphores,
            current_frame: 0,
        })
    }

    /// # macOS
    ///
    /// MoltenVK は Vulkan の完全な実装ではなく「portability subset」なので、
    /// `VK_KHR_portability_enumeration` と `ENUMERATE_PORTABILITY_KHR` フラグを
    /// 付けないと物理デバイスが 1 つも列挙されない。
    fn create_instance(
        entry: &ash::Entry,
        display_handle: raw_window_handle::RawDisplayHandle,
    ) -> Result<ash::Instance, Box<dyn Error>> {
        let application_info = vk::ApplicationInfo::default()
            .application_name(c"gueiz")
            .application_version(vk::make_api_version(0, 0, 1, 0))
            .engine_name(c"gueiz")
            .engine_version(vk::make_api_version(0, 0, 1, 0))
            .api_version(vk::API_VERSION_1_2);

        let mut extension_names =
            ash_window::enumerate_required_extensions(display_handle)?.to_vec();

        let mut instance_create_flags = vk::InstanceCreateFlags::empty();
        if cfg!(target_os = "macos") {
            extension_names.push(ash::khr::portability_enumeration::NAME.as_ptr());
            extension_names.push(ash::khr::get_physical_device_properties2::NAME.as_ptr());
            instance_create_flags |= vk::InstanceCreateFlags::ENUMERATE_PORTABILITY_KHR;
        }

        // 検証レイヤは SDK が入っていないと存在しない。あれば有効化する。
        let available_layers = unsafe { entry.enumerate_instance_layer_properties() }?;
        let validation_layer_name = c"VK_LAYER_KHRONOS_validation";
        let has_validation_layer = available_layers.iter().any(|layer| {
            layer.layer_name_as_c_str().is_ok_and(|name| name == validation_layer_name)
        });

        let mut layer_names: Vec<*const c_char> = Vec::new();
        if has_validation_layer {
            layer_names.push(validation_layer_name.as_ptr());
            log::info!("validation layer enabled");
        } else {
            log::warn!("VK_LAYER_KHRONOS_validation not found; running without validation");
        }

        let instance_create_info = vk::InstanceCreateInfo::default()
            .application_info(&application_info)
            .flags(instance_create_flags)
            .enabled_extension_names(&extension_names)
            .enabled_layer_names(&layer_names);

        Ok(unsafe { entry.create_instance(&instance_create_info, None) }?)
    }

    /// グラフィックスキューと present キューの両方を持つ物理デバイスを選ぶ。
    /// 同一ファミリでなくてもよい。
    fn select_physical_device(
        instance: &ash::Instance,
        surface_instance: &ash::khr::surface::Instance,
        surface: vk::SurfaceKHR,
    ) -> Result<(vk::PhysicalDevice, u32, u32), Box<dyn Error>> {
        let physical_devices = unsafe { instance.enumerate_physical_devices() }?;

        let mut best: Option<(u32, vk::PhysicalDevice, u32, u32)> = None;

        for physical_device in physical_devices {
            let properties = unsafe { instance.get_physical_device_properties(physical_device) };
            let device_name = properties
                .device_name_as_c_str()
                .unwrap_or(c"<unknown>")
                .to_string_lossy()
                .into_owned();

            // swapchain を作れないデバイスは論外。
            let extensions =
                unsafe { instance.enumerate_device_extension_properties(physical_device) }?;
            let supports_swapchain = extensions.iter().any(|extension| {
                extension
                    .extension_name_as_c_str()
                    .is_ok_and(|name| name == ash::khr::swapchain::NAME)
            });
            if !supports_swapchain {
                log::debug!("skip {device_name}: no VK_KHR_swapchain");
                continue;
            }

            let queue_families =
                unsafe { instance.get_physical_device_queue_family_properties(physical_device) };

            let mut graphics_queue_family_index = None;
            let mut present_queue_family_index = None;

            for (index, queue_family) in queue_families.iter().enumerate() {
                let index = index as u32;

                if graphics_queue_family_index.is_none()
                    && queue_family.queue_flags.contains(vk::QueueFlags::GRAPHICS)
                {
                    graphics_queue_family_index = Some(index);
                }

                if present_queue_family_index.is_none()
                    && unsafe {
                        surface_instance.get_physical_device_surface_support(
                            physical_device,
                            index,
                            surface,
                        )
                    }?
                {
                    present_queue_family_index = Some(index);
                }
            }

            let (Some(graphics), Some(present)) =
                (graphics_queue_family_index, present_queue_family_index)
            else {
                log::debug!("skip {device_name}: no graphics/present queue");
                continue;
            };

            // 単体 GPU > 統合 GPU > その他、の順で選ぶ。
            let score = match properties.device_type {
                vk::PhysicalDeviceType::DISCRETE_GPU => 3,
                vk::PhysicalDeviceType::INTEGRATED_GPU => 2,
                _ => 1,
            };
            log::debug!("candidate {device_name} ({:?}, score {score})", properties.device_type);

            if best.is_none_or(|(best_score, ..)| score > best_score) {
                best = Some((score, physical_device, graphics, present));
            }
        }

        let (_, physical_device, graphics, present) =
            best.ok_or("no suitable vulkan physical device found")?;

        let properties = unsafe { instance.get_physical_device_properties(physical_device) };
        log::info!(
            "using gpu: {} (graphics queue {graphics}, present queue {present})",
            properties.device_name_as_c_str()?.to_string_lossy()
        );

        Ok((physical_device, graphics, present))
    }

    fn create_device(
        instance: &ash::Instance,
        physical_device: vk::PhysicalDevice,
        graphics_queue_family_index: u32,
        present_queue_family_index: u32,
    ) -> Result<(ash::Device, vk::Queue, vk::Queue), Box<dyn Error>> {
        let queue_priorities = [1.0_f32];

        // 同じファミリなら DeviceQueueCreateInfo を重複させてはいけない。
        let mut queue_family_indices = vec![graphics_queue_family_index];
        if present_queue_family_index != graphics_queue_family_index {
            queue_family_indices.push(present_queue_family_index);
        }

        let queue_create_infos: Vec<_> = queue_family_indices
            .iter()
            .map(|&queue_family_index| {
                vk::DeviceQueueCreateInfo::default()
                    .queue_family_index(queue_family_index)
                    .queue_priorities(&queue_priorities)
            })
            .collect();

        let mut extension_names = vec![ash::khr::swapchain::NAME.as_ptr()];

        // MoltenVK は VK_KHR_portability_subset を報告する。
        // 報告されたら「必ず」有効化しなければならない（仕様上の要求）。
        let available_extensions =
            unsafe { instance.enumerate_device_extension_properties(physical_device) }?;
        let needs_portability_subset = available_extensions.iter().any(|extension| {
            extension
                .extension_name_as_c_str()
                .is_ok_and(|name| name == ash::khr::portability_subset::NAME)
        });
        if needs_portability_subset {
            extension_names.push(ash::khr::portability_subset::NAME.as_ptr());
        }

        let device_create_info = vk::DeviceCreateInfo::default()
            .queue_create_infos(&queue_create_infos)
            .enabled_extension_names(&extension_names);

        let device = unsafe { instance.create_device(physical_device, &device_create_info, None) }?;

        let graphics_queue = unsafe { device.get_device_queue(graphics_queue_family_index, 0) };
        let present_queue = unsafe { device.get_device_queue(present_queue_family_index, 0) };

        Ok((device, graphics_queue, present_queue))
    }

    fn select_surface_format(
        surface_instance: &ash::khr::surface::Instance,
        physical_device: vk::PhysicalDevice,
        surface: vk::SurfaceKHR,
    ) -> Result<vk::SurfaceFormatKHR, Box<dyn Error>> {
        let surface_formats = unsafe {
            surface_instance.get_physical_device_surface_formats(physical_device, surface)
        }?;

        let preferred = surface_formats.iter().find(|surface_format| {
            surface_format.format == vk::Format::B8G8R8A8_SRGB
                && surface_format.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR
        });

        Ok(*preferred
            .or(surface_formats.first())
            .ok_or("surface reports no formats")?)
    }

    /// 添付 1 枚（カラー）だけの最小レンダーパス。
    fn create_render_pass(
        device: &ash::Device,
        format: vk::Format,
    ) -> Result<vk::RenderPass, Box<dyn Error>> {
        let color_attachment = vk::AttachmentDescription::default()
            .format(format)
            .samples(vk::SampleCountFlags::TYPE_1)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::STORE)
            .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
            .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .final_layout(vk::ImageLayout::PRESENT_SRC_KHR);

        let color_attachment_references = [vk::AttachmentReference::default()
            .attachment(0)
            .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)];

        let subpasses = [vk::SubpassDescription::default()
            .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
            .color_attachments(&color_attachment_references)];

        // 画像が利用可能になるまでカラー書き込みを待たせる依存。
        let dependencies = [vk::SubpassDependency::default()
            .src_subpass(vk::SUBPASS_EXTERNAL)
            .dst_subpass(0)
            .src_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
            .src_access_mask(vk::AccessFlags::empty())
            .dst_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
            .dst_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)];

        let attachments = [color_attachment];
        let render_pass_create_info = vk::RenderPassCreateInfo::default()
            .attachments(&attachments)
            .subpasses(&subpasses)
            .dependencies(&dependencies);

        Ok(unsafe { device.create_render_pass(&render_pass_create_info, None) }?)
    }

    fn invalidate_swapchain(&mut self) {
        self.swapchain_invalidated = true;
    }

    fn recreate_swapchain(&mut self) -> Result<(), Box<dyn Error>> {
        // 使用中のリソースを壊さないよう GPU の完了を待つ。
        // 毎フレームやると遅いが、リサイズ時だけなので許容。
        unsafe { self.device.device_wait_idle() }?;

        let surface_format = Self::select_surface_format(
            &self.surface_instance,
            self.physical_device,
            self.surface,
        )?;

        let old_swapchain_handle = self.swapchain.handle;
        let old_swapchain = std::mem::replace(
            &mut self.swapchain,
            Swapchain::new(
                &self.surface_instance,
                &self.swapchain_device,
                &self.device,
                self.physical_device,
                self.surface,
                surface_format,
                self.graphics_queue_family_index,
                self.present_queue_family_index,
                self.render_pass,
                // 旧 swapchain を渡すとドライバがリソースを再利用できる。
                old_swapchain_handle,
            )?,
        );
        unsafe { old_swapchain.destroy(&self.device, &self.swapchain_device) };

        // イメージ枚数が変わることがあるのでセマフォも作り直す。
        for &semaphore in &self.render_finished_semaphores {
            unsafe { self.device.destroy_semaphore(semaphore, None) };
        }
        self.render_finished_semaphores.clear();
        for _ in 0..self.swapchain.image_views.len() {
            self.render_finished_semaphores.push(unsafe {
                self.device.create_semaphore(&vk::SemaphoreCreateInfo::default(), None)
            }?);
        }

        self.swapchain_invalidated = false;
        log::info!(
            "swapchain recreated: {}x{}",
            self.swapchain.extent.width,
            self.swapchain.extent.height
        );

        Ok(())
    }

    fn draw_frame(&mut self, clear_color: [f32; 4]) -> Result<(), Box<dyn Error>> {
        if self.swapchain_invalidated {
            self.recreate_swapchain()?;
        }

        let in_flight_fence = self.in_flight_fences[self.current_frame];
        let image_available_semaphore = self.image_available_semaphores[self.current_frame];
        let command_buffer = self.command_buffers[self.current_frame];

        // このフレームスロットの前回の実行が終わるまで待つ。
        unsafe { self.device.wait_for_fences(&[in_flight_fence], true, u64::MAX) }?;

        let image_index = match unsafe {
            self.swapchain_device.acquire_next_image(
                self.swapchain.handle,
                u64::MAX,
                image_available_semaphore,
                vk::Fence::null(),
            )
        } {
            Ok((image_index, _suboptimal)) => image_index,
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                // acquire が失敗したときはセマフォがシグナルされていないので、
                // fence をリセットせずに戻って次フレームで作り直す。
                self.swapchain_invalidated = true;
                return Ok(());
            }
            Err(error) => return Err(error.into()),
        };

        // ここまで来たら必ず submit する。この時点で reset してよい。
        unsafe { self.device.reset_fences(&[in_flight_fence]) }?;

        self.record_command_buffer(command_buffer, image_index as usize, clear_color)?;

        let render_finished_semaphore = self.render_finished_semaphores[image_index as usize];
        let wait_semaphores = [image_available_semaphore];
        let wait_stages = [vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT];
        let signal_semaphores = [render_finished_semaphore];
        let command_buffers = [command_buffer];

        let submit_info = vk::SubmitInfo::default()
            .wait_semaphores(&wait_semaphores)
            .wait_dst_stage_mask(&wait_stages)
            .command_buffers(&command_buffers)
            .signal_semaphores(&signal_semaphores);

        unsafe {
            self.device.queue_submit(self.graphics_queue, &[submit_info], in_flight_fence)
        }?;

        let swapchains = [self.swapchain.handle];
        let image_indices = [image_index];
        let present_info = vk::PresentInfoKHR::default()
            .wait_semaphores(&signal_semaphores)
            .swapchains(&swapchains)
            .image_indices(&image_indices);

        match unsafe { self.swapchain_device.queue_present(self.present_queue, &present_info) } {
            Ok(false) => {}
            // suboptimal / out of date はどちらも作り直しの合図。
            Ok(true) | Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => self.swapchain_invalidated = true,
            Err(error) => return Err(error.into()),
        }

        self.current_frame = (self.current_frame + 1) % MAX_FRAMES_IN_FLIGHT;

        Ok(())
    }

    /// レンダーパスを開いて閉じるだけ。`load_op = CLEAR` なので
    /// これだけで画面が clear_color に塗られる。
    fn record_command_buffer(
        &self,
        command_buffer: vk::CommandBuffer,
        image_index: usize,
        clear_color: [f32; 4],
    ) -> Result<(), Box<dyn Error>> {
        unsafe {
            self.device
                .reset_command_buffer(command_buffer, vk::CommandBufferResetFlags::empty())?;

            self.device.begin_command_buffer(
                command_buffer,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )?;

            let clear_values = [vk::ClearValue {
                color: vk::ClearColorValue { float32: clear_color },
            }];

            let render_pass_begin_info = vk::RenderPassBeginInfo::default()
                .render_pass(self.render_pass)
                .framebuffer(self.swapchain.framebuffers[image_index])
                .render_area(vk::Rect2D::default().extent(self.swapchain.extent))
                .clear_values(&clear_values);

            self.device.cmd_begin_render_pass(
                command_buffer,
                &render_pass_begin_info,
                vk::SubpassContents::INLINE,
            );
            // 描画コマンドはまだ無い。パイプラインを作ったらここに入る。
            self.device.cmd_end_render_pass(command_buffer);

            self.device.end_command_buffer(command_buffer)?;
        }

        Ok(())
    }
}

impl Drop for VulkanRenderer {
    fn drop(&mut self) {
        unsafe {
            // GPU が全部使い終わってから壊す。これを忘れると即クラッシュする。
            if let Err(error) = self.device.device_wait_idle() {
                log::error!("device_wait_idle failed on shutdown: {error}");
            }

            for &semaphore in &self.render_finished_semaphores {
                self.device.destroy_semaphore(semaphore, None);
            }
            for &semaphore in &self.image_available_semaphores {
                self.device.destroy_semaphore(semaphore, None);
            }
            for &fence in &self.in_flight_fences {
                self.device.destroy_fence(fence, None);
            }

            // コマンドバッファはプールごと破棄される。
            self.device.destroy_command_pool(self.command_pool, None);

            self.swapchain.destroy(&self.device, &self.swapchain_device);
            self.device.destroy_render_pass(self.render_pass, None);

            self.device.destroy_device(None);
            self.surface_instance.destroy_surface(self.surface, None);
            self.instance.destroy_instance(None);
        }
    }
}

impl Swapchain {
    #[allow(clippy::too_many_arguments)]
    fn new(
        surface_instance: &ash::khr::surface::Instance,
        swapchain_device: &ash::khr::swapchain::Device,
        device: &ash::Device,
        physical_device: vk::PhysicalDevice,
        surface: vk::SurfaceKHR,
        surface_format: vk::SurfaceFormatKHR,
        graphics_queue_family_index: u32,
        present_queue_family_index: u32,
        render_pass: vk::RenderPass,
        old_swapchain: vk::SwapchainKHR,
    ) -> Result<Self, Box<dyn Error>> {
        let capabilities = unsafe {
            surface_instance.get_physical_device_surface_capabilities(physical_device, surface)
        }?;

        // current_extent が u32::MAX のときだけ自分でサイズを決められる。
        // それ以外はウィンドウシステムの言い値に従う。
        let extent = if capabilities.current_extent.width == u32::MAX {
            vk::Extent2D {
                width: capabilities.min_image_extent.width.max(1),
                height: capabilities.min_image_extent.height.max(1),
            }
        } else {
            capabilities.current_extent
        };

        // 最小 + 1 枚あるとドライバの待ちが減る。0 は「上限なし」。
        let mut image_count = capabilities.min_image_count + 1;
        if capabilities.max_image_count > 0 && image_count > capabilities.max_image_count {
            image_count = capabilities.max_image_count;
        }

        let present_modes = unsafe {
            surface_instance.get_physical_device_surface_present_modes(physical_device, surface)
        }?;
        // FIFO は必ず存在する。MAILBOX があればティアリング無しで低レイテンシ。
        let present_mode = if present_modes.contains(&vk::PresentModeKHR::MAILBOX) {
            vk::PresentModeKHR::MAILBOX
        } else {
            vk::PresentModeKHR::FIFO
        };

        let queue_family_indices = [graphics_queue_family_index, present_queue_family_index];
        let (sharing_mode, queue_family_indices): (_, &[u32]) =
            if graphics_queue_family_index == present_queue_family_index {
                (vk::SharingMode::EXCLUSIVE, &[])
            } else {
                (vk::SharingMode::CONCURRENT, &queue_family_indices)
            };

        let swapchain_create_info = vk::SwapchainCreateInfoKHR::default()
            .surface(surface)
            .min_image_count(image_count)
            .image_format(surface_format.format)
            .image_color_space(surface_format.color_space)
            .image_extent(extent)
            .image_array_layers(1)
            .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
            .image_sharing_mode(sharing_mode)
            .queue_family_indices(queue_family_indices)
            .pre_transform(capabilities.current_transform)
            .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
            .present_mode(present_mode)
            .clipped(true)
            .old_swapchain(old_swapchain);

        let handle = unsafe { swapchain_device.create_swapchain(&swapchain_create_info, None) }?;
        let images = unsafe { swapchain_device.get_swapchain_images(handle) }?;

        let mut image_views = Vec::with_capacity(images.len());
        for &image in &images {
            let image_view_create_info = vk::ImageViewCreateInfo::default()
                .image(image)
                .view_type(vk::ImageViewType::TYPE_2D)
                .format(surface_format.format)
                .subresource_range(
                    vk::ImageSubresourceRange::default()
                        .aspect_mask(vk::ImageAspectFlags::COLOR)
                        .level_count(1)
                        .layer_count(1),
                );
            image_views.push(unsafe { device.create_image_view(&image_view_create_info, None) }?);
        }

        let mut framebuffers = Vec::with_capacity(image_views.len());
        for &image_view in &image_views {
            let attachments = [image_view];
            let framebuffer_create_info = vk::FramebufferCreateInfo::default()
                .render_pass(render_pass)
                .attachments(&attachments)
                .width(extent.width)
                .height(extent.height)
                .layers(1);
            framebuffers
                .push(unsafe { device.create_framebuffer(&framebuffer_create_info, None) }?);
        }

        Ok(Self {
            handle,
            format: surface_format.format,
            extent,
            image_views,
            framebuffers,
        })
    }

    /// # Safety
    ///
    /// GPU がこのスワップチェーンを使い終わっていること。
    unsafe fn destroy(
        &self,
        device: &ash::Device,
        swapchain_device: &ash::khr::swapchain::Device,
    ) {
        unsafe {
            for &framebuffer in &self.framebuffers {
                device.destroy_framebuffer(framebuffer, None);
            }
            for &image_view in &self.image_views {
                device.destroy_image_view(image_view, None);
            }
            // スワップチェーンのイメージ本体はドライバの所有物なので破棄しない。
            swapchain_device.destroy_swapchain(self.handle, None);
        }
    }
}
