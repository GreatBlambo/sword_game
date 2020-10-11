use vulkano::instance::Instance;
use vulkano::instance::InstanceExtensions;
use vulkano::instance::PhysicalDevice;
use vulkano::instance::PhysicalDeviceType;

use vulkano::device::Device;
use vulkano::device::DeviceExtensions;
use vulkano::device::Features;

use vulkano::buffer::CpuAccessibleBuffer;
use vulkano::buffer::BufferUsage;

use std::sync::Arc;
use vulkano::format::Format;

use vulkano::framebuffer::Framebuffer;
use vulkano::image::Dimensions;
use vulkano::image::StorageImage;

use vulkano::command_buffer::AutoCommandBufferBuilder;

use vulkano::pipeline::GraphicsPipeline;
use vulkano::framebuffer::Subpass;

use vulkano::command_buffer::DynamicState;
use vulkano::pipeline::viewport::Viewport;

use vulkano::sync;
use vulkano::sync::GpuFuture;

use image::ImageBuffer;
use image::Rgba;

use vulkano::pipeline::vertex::OneVertexOneInstanceDefinition;

mod rendering;

render_config!(
    name: test_render_config,
    attachments: {
        depth: {
            format: Format::D24Unorm_S8Uint
        },
        albedo: {
            format: Format::R8G8B8A8Unorm
        },
        normal: {
            format: Format::R8G8Unorm
        },
        color: {
            format: Format::R8G8B8A8Unorm
        },
        blur: {
            format: Format::R8G8B8A8Unorm
        },
        blur2: {
            format: Format::R8G8B8A8Unorm
        },
        id: {
            format: Format::R32Uint
        }
    },
    default_vertex_bindings: [
        {
            vertex_type_name: Vertex,
            input_rate: 0,
            attributes: {
                position: [f32; 2],
                color: [f32; 2]
            }
        },
        {
            vertex_type_name: InstanceData,
            input_rate: 1,
            attributes: {
                position_offset: [f32; 2],
                scale: f32
            }
        }
    ],
    graphics_passes: {
        gbuffer: {
            color_outputs: [albedo, normal],
            depth_stencil_output: {depth},
            color_inputs: [],
            depth_stencil_input: {},
            pipeline: {
                shader_paths: {
                    vertex: "src/shaders/passthrough_2d.vert",
                    fragment: "src/shaders/passthrough.frag"
                }
            }
        },
        lighting: {
            color_outputs: [color],
            depth_stencil_output: {},
            color_inputs: [albedo, normal],
            depth_stencil_input: {depth},
            pipeline: {
                shader_paths: {
                    vertex: "src/shaders/passthrough_2d.vert",
                    fragment: "src/shaders/passthrough.frag"
                }
            }
        },
        blur_pass: {
            color_outputs: [blur],
            depth_stencil_output: {},
            color_inputs: [color],
            depth_stencil_input: {},
            pipeline: {
                shader_paths: {
                    vertex: "src/shaders/passthrough_2d.vert",
                    fragment: "src/shaders/passthrough.frag"
                }
            }
        },
        blur_pass2: {
            color_outputs: [blur2],
            depth_stencil_output: {},
            color_inputs: [color],
            depth_stencil_input: {},
            pipeline: {
                shader_paths: {
                    vertex: "src/shaders/passthrough_2d.vert",
                    fragment: "src/shaders/passthrough.frag"
                }
            }
        },
        composite_pass: {
            color_outputs: [backbuffer],
            depth_stencil_output: {},
            color_inputs: [color, blur, blur2],
            depth_stencil_input: {},
            pipeline: {
                shader_paths: {
                    vertex: "src/shaders/passthrough_2d.vert",
                    fragment: "src/shaders/passthrough.frag"
                }
            }
        },
        id_pass: {
            color_outputs: [id],
            depth_stencil_output: {},
            color_inputs: [color],
            depth_stencil_input: {depth},
            pipeline: {
                shader_paths: {
                    vertex: "src/shaders/passthrough_2d.vert",
                    fragment: "src/shaders/passthrough.frag"
                }
            }
        }
    }
);

#[derive(Default, Copy, Clone)]
struct Vertex {
    position: [f32; 2],
    color: [f32; 3]
}

vulkano::impl_vertex!(Vertex, position, color);

#[derive(Default, Copy, Clone)]
struct InstanceData {
    position_offset: [f32; 2],
    scale: f32
}

vulkano::impl_vertex!(InstanceData, position_offset, scale);

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
    test_render_config::build().unwrap();

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

    let vertex_buffer = CpuAccessibleBuffer::from_iter(
        device.clone(),
        BufferUsage::all(),
        false,
        vec![
            Vertex {
                position: [-0.5, -0.5],
                color: [1.0, 1.0, 0.0]
            },
            Vertex {
                position: [0.0, 0.5],
                color: [1.0, 0.0, 1.0]
            },
            Vertex {
                position: [0.5, -0.25],
                color: [0.0, 1.0, 1.0]
            }
        ].into_iter()
    ).unwrap();

    let instance_buffer= CpuAccessibleBuffer::from_iter(
        device.clone(),
        BufferUsage::all(),
        false,
        vec![
            InstanceData {
                position_offset: [-0.5, -0.5],
                scale: 1.0 
            },
            InstanceData {
                position_offset: [0.0, 0.5],
                scale: 0.5
            },
            InstanceData {
                position_offset: [0.5, -0.25],
                scale: 0.75
            }
        ].into_iter()
    ).unwrap();

    // Render pass and framebuffers

    let render_pass = Arc::new(
        vulkano::single_pass_renderpass!(
            device.clone(),
            attachments: {
                color: {
                    load: Clear,
                    store: Store,
                    format: Format::R8G8B8A8Unorm,
                    samples: 1,
                }
            },
            pass: {
                color: [color],
                depth_stencil: {}
            }
        ).unwrap()
    );

    let image = StorageImage::new(
        device.clone(),
        Dimensions::Dim2d {
            width: 1024,
            height: 1024
        },
        Format::R8G8B8A8Unorm,
        Some(queue.family())
    ).unwrap();

    let image_buf = CpuAccessibleBuffer::from_iter(
        device.clone(),
        BufferUsage::all(),
        false,
        (0 .. 1024 * 1024 * 4).map(|_| 0u8)
    ).expect("Failed to create image buf");

    let framebuffer = Arc::new(
        Framebuffer::start(
            render_pass.clone()
        )
        .add(image.clone()).unwrap()
        .build().unwrap()
    );

    // Pipeline
    mod vs {
        vulkano_shaders::shader!{
            ty: "vertex",
            path: "src/shaders/passthrough_2d.vert"
        }
    }

    mod fs {
        vulkano_shaders::shader!{
            ty: "fragment",
            path: "src/shaders/passthrough.frag"
        }
    }

    let vs = vs::Shader::load(device.clone()).expect("Failed to create VS");
    let fs = fs::Shader::load(device.clone()).expect("Failed to create FS");

    let pipeline = Arc::new(
        GraphicsPipeline::start()
            .vertex_input(
                OneVertexOneInstanceDefinition::<Vertex, InstanceData>::new()
            )//.vertex_input_single_buffer::<Vertex>()
            .vertex_shader(
                vs.main_entry_point(), 
                ()
            )
            .viewports_dynamic_scissors_irrelevant(1)
            .fragment_shader(
                fs.main_entry_point(), 
                ()
            )
            .render_pass(
                Subpass::from(
                    render_pass.clone(), 
                    0
                ).unwrap()
            )
            .build(device.clone())
            .unwrap()
    );

    // Draw commands

    let dynamic_state = DynamicState {
        viewports: Some(
            vec![
                Viewport {
                    origin: [0.0, 0.0],
                    dimensions: [1024.0, 1024.0],
                    depth_range: 0.0 .. 1.0,
                }
            ]
        ),
        .. DynamicState::none()
    };

    let mut cmd_buf_builder = AutoCommandBufferBuilder::primary_one_time_submit(
        device.clone(), 
        queue_family
    ).unwrap();

    cmd_buf_builder
        .begin_render_pass(
            framebuffer.clone(), 
            false, 
            vec![[0.0, 0.0, 1.0, 1.0].into()]
        ).unwrap()

        .draw(
            pipeline.clone(),
            &dynamic_state,
            (vertex_buffer.clone(), instance_buffer.clone()),
            (),
            ()
        ).unwrap()

        .end_render_pass()
        .unwrap()
        
        .copy_image_to_buffer(
            image.clone(), 
            image_buf.clone()
        )
        .unwrap();
    
    let cmd_buf = cmd_buf_builder.build().unwrap();

    // Execute
    let future = sync::now(device.clone())
        .then_execute(
            queue.clone(), 
            cmd_buf
        )
        .unwrap()
        .then_signal_fence_and_flush()
        .unwrap();

    future.wait(None).unwrap();

    println!("Draw done");

    let image_buf_content = image_buf.read().unwrap();
    let image = ImageBuffer::<Rgba<u8>, _>::from_raw(
        1024,
        1024,
        &image_buf_content[..]
    ).unwrap();

    image.save("triangle.png").unwrap();
}