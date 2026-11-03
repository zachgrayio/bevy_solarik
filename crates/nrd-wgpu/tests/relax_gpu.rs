//! Real SPIR-V execution and readback, also runnable on Linux's Vulkan software driver.
use solarik_nrd_wgpu::{Context, Frame, Image};

fn write(queue: &wgpu::Queue, image: &Image, bytes: &[u8], pixel_bytes: u32) {
    queue.write_texture(
        image.texture.as_image_copy(),
        bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(32 * pixel_bytes),
            rows_per_image: None,
        },
        image.texture.size(),
    );
}

#[test]
fn relax_reduces_noise_preserves_energy_and_restarts_history() {
    futures_lite::future::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance
            .request_adapter(&Default::default())
            .await
            .expect("Vulkan adapter");
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                required_features: wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES
                    | wgpu::Features::TEXTURE_FORMAT_16BIT_NORM
                    | wgpu::Features::FLOAT32_FILTERABLE
                    | wgpu::Features::PASSTHROUGH_SHADERS,
                required_limits: adapter.limits(),
                ..Default::default()
            })
            .await
            .expect("NRD-capable Vulkan device");
        let mut context = Context::new(&device, 32, 32).unwrap();
        write(&queue, &context.images[0], &vec![0; 32 * 32 * 4], 4);
        let normal: Vec<u8> = [0.5, 0.5, 1.0, 0.0]
            .into_iter()
            .flat_map(|v| half::f16::from_f32(v).to_le_bytes())
            .collect();
        write(&queue, &context.images[1], &normal.repeat(32 * 32), 8);
        write(
            &queue,
            &context.images[2],
            &1f32.to_le_bytes().repeat(32 * 32),
            4,
        );
        let identity = [
            1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
        ];
        let projection = [
            1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 0., -1., 0., 0., 0.1, 0.,
        ];
        for index in 0..32 {
            let signal: Vec<u8> = (0..1024)
                .flat_map(|p| {
                    let value = if (p + p / 32 + index) % 2 == 0 {
                        0.25
                    } else {
                        1.75
                    };
                    [value, value, value, 1.0]
                        .into_iter()
                        .flat_map(|v| half::f16::from_f32(v).to_le_bytes())
                })
                .collect();
            write(&queue, &context.images[3], &signal, 8);
            write(&queue, &context.images[4], &signal, 8);
            let mut encoder = device.create_command_encoder(&Default::default());
            context
                .denoise(
                    &device,
                    &queue,
                    &mut encoder,
                    &Frame {
                        projection,
                        previous_projection: projection,
                        world_to_view: identity,
                        previous_world_to_view: identity,
                        index,
                        reset: index == 0 || index == 16,
                    },
                )
                .unwrap();
            queue.submit([encoder.finish()]);
        }
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("RELAX readback"),
            size: 16384,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        for (slot, offset) in [(5, 0), (6, 8192)] {
            encoder.copy_texture_to_buffer(
                context.images[slot].texture.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset,
                        bytes_per_row: Some(256),
                        rows_per_image: None,
                    },
                },
                context.images[slot].texture.size(),
            );
        }
        queue.submit([encoder.finish()]);
        let (send, receive) = std::sync::mpsc::channel();
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                send.send(result).unwrap()
            });
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        receive.recv().unwrap().unwrap();
        let bytes = readback.slice(..).get_mapped_range().unwrap();
        for (name, bytes) in ["diffuse", "specular"]
            .into_iter()
            .zip(bytes.as_chunks::<8192>().0)
        {
            let values: Vec<f32> = bytes
                .as_chunks::<8>()
                .0
                .iter()
                .map(|pixel| half::f16::from_le_bytes([pixel[0], pixel[1]]).to_f32())
                .collect();
            let mean = values.iter().sum::<f32>() / values.len() as f32;
            let variance =
                values.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / values.len() as f32;
            eprintln!("RELAX {name} mean={mean:.5}, variance={variance:.6}; input variance=0.5625");
            assert!(
                (mean - 1.0).abs() < 0.15,
                "RELAX must preserve mean illumination: {mean}"
            );
            assert!(
                variance < 0.05625,
                "RELAX must remove at least 90% of input variance: {variance}"
            );
        }
    });
}
