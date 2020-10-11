use vulkano::instance::Instance;
use vulkano::instance::InstanceExtensions;
use vulkano::instance::PhysicalDevice;
use vulkano::instance::PhysicalDeviceType;

use vulkano::device::Device;
use vulkano::device::DeviceExtensions;
use vulkano::device::Features;

use vulkano::buffer::BufferUsage;
use vulkano::buffer::CpuAccessibleBuffer;

use std::sync::Arc;
use vulkano::pipeline::ComputePipeline;

use vulkano::descriptor::descriptor_set::PersistentDescriptorSet;
use vulkano::descriptor::PipelineLayoutAbstract;

use vulkano::command_buffer::AutoCommandBufferBuilder;
use vulkano::sync;
use vulkano::sync::GpuFuture;

use vulkano::format::Format;
use vulkano::image::Dimensions;
use vulkano::image::StorageImage;

use vulkano::format::ClearValue;

use image::{ImageBuffer, Rgba};

mod cs {
    vulkano_shaders::shader!{
        ty: "compute",
        path: "src/shaders/test.comp"
    }
}

mod mandelbrot {
    vulkano_shaders::shader!{
        ty: "compute",
        path: "src/shaders/mandelbrot.comp"
    }
}

fn device_rank(physical: &PhysicalDevice) -> u64 {
    // Device type ranks highest
    let device_type_rank = match physical.ty() {
        PhysicalDeviceType::DiscreteGpu => 5,
        PhysicalDeviceType::IntegratedGpu => 4,
        PhysicalDeviceType::VirtualGpu => 3,
        PhysicalDeviceType::Cpu => 2,
        PhysicalDeviceType::Other => 1,
    };

    return device_type_rank;
}

fn main() {
    // Create a vulkan instance
    let instance = Instance::new(None, &InstanceExtensions::none(), None).expect("Failed to create vulkan instance");

    // Choose a physical device
    let physical = PhysicalDevice::enumerate(&instance).max_by(|x, y| device_rank(&x).cmp(&device_rank(&y))).expect("No physical device available");

    println!("Physical device chosen: {}", physical.name());

    // List queue families and find one that supports both gfx and compute
    for family in physical.queue_families() {
        println!("Found a queue family with {:?} queue(s)", family.queues_count());
    }

    let queue_family = physical.queue_families()
                               .find(|&q| q.supports_graphics() && q.supports_compute())
                               .expect("Couldn't find a valid queue family");

    // Create the device and queues
    let (device, mut queues) = {
        Device::new(
            physical, 
            &Features::none(), 
            &DeviceExtensions{
                khr_storage_buffer_storage_class: true, 
                ..DeviceExtensions::none()
            },
            [(queue_family, 0.5)]
                                .iter()
                                .cloned()
        )
        .expect("Failed to create device")
    };

    let queue = queues.next().unwrap();

    // Create a buffer to be used by compute shader
    let iter = (0 .. 65536).map(|x| cs::ty::SomeStruct {a: x as f32, b: x as f32});
    let buffer = CpuAccessibleBuffer::from_iter(
        device.clone(), 
        BufferUsage::all(), 
        false, 
        iter
    ).expect("Failed to create buffer");

    // Create a compute shader
    let shader = cs::Shader::load(device.clone()).expect("Failed to create shader module");

    // Create a compute pipeline
    let compute_pipeline = Arc::new(
            ComputePipeline::new(
                device.clone(), 
                &shader.main_entry_point(), 
                &()
            )
        .expect("Failed to create compute pipeline")
    );

    // Create the descriptor sets from layouts in the shader
    let layout = compute_pipeline.layout().descriptor_set_layout(0).unwrap();
    let set = Arc::new(
        PersistentDescriptorSet::start(layout.clone())
            .add_buffer(buffer.clone()).unwrap()
            .build().unwrap()
    );

    // Build compute command buffer
    let mut cmd_buffer_builder = AutoCommandBufferBuilder::new(
        device.clone(),
        queue.family()
    ).unwrap();

    // Record a dispatch command with 1024 compute work groups along one dimension

    // Descriptor set is bound here. You can make reads/writes to the underlying
    // buffer before and after dispatch.
    cmd_buffer_builder.dispatch(
        [1024, 1, 1],
        compute_pipeline.clone(),
        set.clone(),
        ()
    ).unwrap();

    let cmd_buffer = cmd_buffer_builder.build().unwrap();

    // Begin image portion

    let image = StorageImage::new(
        device.clone(),
        Dimensions::Dim2d {
            width: 1024,
            height: 1024
        },
        Format::R8G8B8A8Unorm,
        Some(queue.family())
    ).unwrap();

    let image_buffer= CpuAccessibleBuffer::from_iter(
        device.clone(),
        BufferUsage::all(),
        false,
        (0 .. 1024 * 1024 * 4).map(|_| 0u8)
    ).expect("Failed to create image buffer");

    let mandelbrot_shader = mandelbrot::Shader::load(device.clone()).expect("Failed to load mandelbrot shader");
    let image_compute_pipeline = Arc::new(
        ComputePipeline::new(
            device.clone(),
            &mandelbrot_shader.main_entry_point(),
            &()
        ).expect("Failed to create image compute pipeline")
    );

    let image_layout = image_compute_pipeline.layout().descriptor_set_layout(0).unwrap();
    let image_set = Arc::new(
        PersistentDescriptorSet::start(image_layout.clone())
            .add_image(image.clone()).unwrap()
            .build().unwrap()
    );

    let mut image_cmd_buf_builder = AutoCommandBufferBuilder::new(
        device.clone(),
        queue.family()
    ).unwrap();

    image_cmd_buf_builder
        .clear_color_image(
            image.clone(), 
            ClearValue::Float(
                [0.0, 0.0, 1.0, 1.0]
            )
        ).unwrap()
        .dispatch(
            [1024 / 8, 1024 / 8, 1],
            image_compute_pipeline.clone(),
            image_set.clone(),
            ()
        ).unwrap()
        .copy_image_to_buffer(
            image.clone(),
            image_buffer.clone()
        ).unwrap();

    let image_command_buffer = image_cmd_buf_builder.build().unwrap();

    // Execute
    let future = sync::now(device.clone())
        .then_execute(
            queue.clone(), 
            cmd_buffer
        )
        .unwrap()
        .then_execute(
            queue.clone(),
            image_command_buffer,
        )
        .unwrap()
        .then_signal_fence_and_flush()
        .unwrap();

    future.wait(None).unwrap();

    println!("Compute dispatch done");

    // Assert test shader output is correct
    let data_buffer_content = buffer.read().unwrap();
    for n in 0..65536 {
        let test_struct = data_buffer_content[n as usize];
        assert_eq!(test_struct.a, n as f32 * 12f32);
        assert_eq!(test_struct.b, n as f32 * 21f32);
    }

    let image_buffer_content = image_buffer.read().unwrap();
    let image = ImageBuffer::<Rgba<u8>, _>::from_raw(
        1024, 
        1024,
        &image_buffer_content[..]
    ).unwrap();

    image.save("image.png").unwrap();
}