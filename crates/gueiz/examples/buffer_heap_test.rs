//! `BufferHeap` を実際の GPU に対して動かす確認用の例。ウィンドウは開かない。
//!
//! ヒープ側に寿命の長いデータを置き、フレーム側を数フレームぶん回して、
//! 検証エラーが出ないことと、スライスが輪番で使われることを見る。
//!
//! ```sh
//! RUST_LOG=info cargo run -p gueiz --example buffer_heap_test
//! ```

use std::error::Error;

use gueiz_2d::buffer::{BufferHeap, BufferHeapDescriptor};
use gueiz_2d::wgpu;

const FRAMES_IN_FLIGHT: u32 = 3;

fn main() -> Result<(), Box<dyn Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    // サーフェスを作らない構成。ウィンドウ無しで GPU だけ掴む。
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    }))?;

    let adapter_info = adapter.get_info();
    log::info!("backend : {:?}", adapter_info.backend);
    log::info!("adapter : {}", adapter_info.name);

    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("gueiz device"),
        ..Default::default()
    }))?;

    // 前方 3 KiB がヒープ、後方 1 KiB が 3 スライスのフレーム領域。
    let mut buffer_heap = BufferHeap::new(&device, &BufferHeapDescriptor {
        label: Some("gueiz vertex heap"),
        size: 4096,
        frame_size: 1024,
        frames_in_flight: FRAMES_IN_FLIGHT,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        alignment: 4,
    });

    log::info!(
        "layout  : heap {} bytes, frame {} bytes ({} slices x {} bytes)",
        buffer_heap.heap().size(),
        buffer_heap.frame_region().size(),
        buffer_heap.frame_region().slice_count(),
        buffer_heap.frame_region().slice_size(),
    );

    // 寿命の長いデータ。フレームをまたいで残る。
    let persistent = buffer_heap.allocate(256)?;
    buffer_heap.write(&queue, &persistent, &[0xAB; 256])?;
    log::info!("persistent allocation at {}", persistent.offset());

    // フレーム領域を 1 周半まわす。2 周目でスライスが再利用され、
    // そこで初めて begin_frame の待ちが効く。
    for frame_index in 0..(FRAMES_IN_FLIGHT * 2) {
        buffer_heap.begin_frame(&device)?;

        let transient = buffer_heap.allocate_frame(64)?;
        buffer_heap.write(&queue, &transient, &[frame_index as u8; 64])?;

        log::info!(
            "frame {frame_index}: slice {}, offset {}, {} bytes left",
            buffer_heap.frame_region().current_slice(),
            transient.offset(),
            buffer_heap.frame_region().available(),
        );

        // 実際の描画の代わりに、空のコマンドバッファを投げて submission を得る。
        let encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("gueiz frame"),
        });
        let submission_index = queue.submit(Some(encoder.finish()));
        buffer_heap.end_frame(submission_index);
    }

    // ヒープ側はフレームを回しても影響を受けない。
    log::info!(
        "heap    : {} / {} bytes used, {} blocks",
        buffer_heap.heap().used(),
        buffer_heap.heap().size(),
        buffer_heap.heap().block_count(),
    );

    buffer_heap.deallocate(persistent)?;
    log::info!("after deallocate: {} bytes used", buffer_heap.heap().used());

    Ok(())
}
