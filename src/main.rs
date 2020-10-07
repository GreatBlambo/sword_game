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

mod cs {
    vulkano_shaders::shader!{
        ty: "compute",
        path: "src/shaders/test.comp"
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
        Device::new(physical, &Features::none(), &DeviceExtensions{khr_storage_buffer_storage_class: true, ..DeviceExtensions::none()},
                    [(queue_family, 0.5)].iter().cloned()).expect("Familed to create device")
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

}
