#![feature(prelude_import)]
#[prelude_import]
use std::prelude::v1::*;
#[macro_use]
extern crate std;
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
mod rendering {
    use typed_arena::Arena;
    use vulkano::format::Format;
    use std::cell::RefCell;
    use std::cell::Cell;
    use std::collections::HashSet;
    use std::collections::HashMap;
    use std::collections::BinaryHeap;
    use std::cmp::Ordering;
    use std::ptr::eq;
    pub const BACKBUFFER_NAME: &str = "BACKBUFFER";
    pub struct AttachmentDesc<'rb> {
        name: &'rb str,
        format: Format,
        samples: usize,
        readers: RefCell<Vec<&'rb PassDesc<'rb>>>,
        writers: RefCell<Vec<&'rb PassDesc<'rb>>>,
    }
    pub struct PassDesc<'rb> {
        name: &'rb str,
        color_inputs: RefCell<Vec<&'rb AttachmentDesc<'rb>>>,
        color_outputs: RefCell<Vec<&'rb AttachmentDesc<'rb>>>,
        depth_input: RefCell<Option<&'rb AttachmentDesc<'rb>>>,
        depth_output: RefCell<Option<&'rb AttachmentDesc<'rb>>>,
    }
    impl<'rb> PassDesc<'rb> {
        #[inline]
        fn add_writer(&'rb self, attachment: &'rb AttachmentDesc<'rb>) {
            attachment.writers.borrow_mut().push(self);
        }
        #[inline]
        fn add_reader(&'rb self, attachment: &'rb AttachmentDesc<'rb>) {
            attachment.readers.borrow_mut().push(self);
        }
        #[inline]
        pub fn add_color_output(&'rb self, attachment: &'rb AttachmentDesc<'rb>) {
            self.color_outputs.borrow_mut().push(attachment);
            self.add_writer(attachment);
        }
        #[inline]
        pub fn set_depth_output(&'rb self, attachment: &'rb AttachmentDesc<'rb>) {
            self.depth_output.borrow_mut().replace(attachment);
            self.add_writer(attachment);
        }
        #[inline]
        pub fn add_color_input(&'rb self, attachment: &'rb AttachmentDesc<'rb>) {
            self.color_inputs.borrow_mut().push(attachment);
            self.add_reader(attachment);
        }
        #[inline]
        pub fn set_depth_input(&'rb self, attachment: &'rb AttachmentDesc<'rb>) {
            self.depth_input.borrow_mut().replace(attachment);
            self.add_reader(attachment);
        }
    }
    struct PassNodeDependency<'a, 'rb> {
        pass_node: &'a PassNode<'a, 'rb>,
        attachment: &'rb AttachmentDesc<'rb>,
        is_edge: Cell<bool>,
    }
    struct PassNode<'a, 'rb> {
        pass: &'rb PassDesc<'rb>,
        dependents: RefCell<Vec<PassNodeDependency<'a, 'rb>>>,
        dependencies: RefCell<Vec<PassNodeDependency<'a, 'rb>>>,
    }
    impl<'a, 'rb> PassNode<'a, 'rb> {
        pub fn is_independent(&self) -> bool {
            return self.dependencies.borrow().iter().all(|x| !x.is_edge.get());
        }
        pub fn depends_on(&self, other: &'a PassNode<'a, 'rb>) -> bool {
            if eq(self, other) {
                return true;
            }
            for dependency in self.dependencies.borrow().iter() {
                if dependency.pass_node.depends_on(other) {
                    return true;
                }
            }
            return false;
        }
        #[inline]
        pub fn display(&'rb self) {
            {
                ::std::io::_print(::core::fmt::Arguments::new_v1(
                    &["Pass: ", ". Dependent on: \n"],
                    &match (&self.pass.name,) {
                        (arg0,) => [::core::fmt::ArgumentV1::new(
                            arg0,
                            ::core::fmt::Display::fmt,
                        )],
                    },
                ));
            };
            for dependency in self.dependencies.borrow().iter() {
                {
                    ::std::io::_print(::core::fmt::Arguments::new_v1(
                        &["- Pass ", ", via attachment ", "\n"],
                        &match (&dependency.pass_node.pass.name, &dependency.attachment.name) {
                            (arg0, arg1) => [
                                ::core::fmt::ArgumentV1::new(arg0, ::core::fmt::Display::fmt),
                                ::core::fmt::ArgumentV1::new(arg1, ::core::fmt::Display::fmt),
                            ],
                        },
                    ));
                };
            }
        }
    }
    struct RootNode<'a, 'rb> {
        node: &'a PassNode<'a, 'rb>,
        overlap_score: usize,
    }
    impl<'a, 'rb> Ord for RootNode<'a, 'rb> {
        fn cmp(&self, other: &Self) -> Ordering {
            self.overlap_score.cmp(&other.overlap_score)
        }
    }
    impl<'a, 'rb> PartialOrd for RootNode<'a, 'rb> {
        fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
            Some(self.cmp(other))
        }
    }
    impl<'a, 'rb> Eq for RootNode<'a, 'rb> {}
    impl<'a, 'rb> PartialEq for RootNode<'a, 'rb> {
        fn eq(&self, other: &Self) -> bool {
            self.overlap_score == other.overlap_score
        }
    }
    pub struct RendererBuilder<'rb> {
        attachment_arena: Arena<AttachmentDesc<'rb>>,
        pass_arena: Arena<PassDesc<'rb>>,
        passes: RefCell<Vec<&'rb PassDesc<'rb>>>,
        backbuffer_attachment: AttachmentDesc<'rb>,
    }
    impl<'rb> RendererBuilder<'rb> {
        pub fn new() -> RendererBuilder<'rb> {
            return RendererBuilder {
                attachment_arena: Arena::new(),
                pass_arena: Arena::new(),
                passes: RefCell::new(Vec::new()),
                backbuffer_attachment: AttachmentDesc {
                    name: BACKBUFFER_NAME,
                    format: Format::R8G8B8A8Unorm,
                    samples: 1,
                    readers: RefCell::new(Vec::new()),
                    writers: RefCell::new(Vec::new()),
                },
            };
        }
        pub fn add_attachment(
            &'rb self,
            name: &'static str,
            format: Format,
            samples: usize,
        ) -> &'rb AttachmentDesc {
            return self.attachment_arena.alloc(AttachmentDesc {
                name,
                format,
                samples,
                readers: RefCell::new(Vec::new()),
                writers: RefCell::new(Vec::new()),
            });
        }
        pub fn add_depth_attachment(
            &'rb self,
            name: &'static str,
            samples: usize,
        ) -> &'rb AttachmentDesc {
            return self.add_attachment(name, Format::D24Unorm_S8Uint, samples);
        }
        pub fn add_pass(&'rb self, name: &'static str) -> &'rb PassDesc<'rb> {
            let pass = self.pass_arena.alloc(PassDesc {
                name,
                color_inputs: RefCell::new(Vec::new()),
                color_outputs: RefCell::new(Vec::new()),
                depth_input: RefCell::new(None),
                depth_output: RefCell::new(None),
            });
            self.passes.borrow_mut().push(pass);
            return pass;
        }
        pub fn get_backbuffer_attachment(&'rb self) -> &'rb AttachmentDesc<'rb> {
            return &self.backbuffer_attachment;
        }
        fn validate_passes<'a, I>(passes: I) -> Result<(), &'static str>
        where
            I: Iterator<Item = &'a &'rb PassDesc<'rb>>,
            'rb: 'a,
        {
            fn is_valid_depth_attachment<'rb>(
                attachment: Option<&'rb AttachmentDesc<'rb>>,
            ) -> bool {
                match attachment {
                    Some(AttachmentDesc {
                        format: Format::D16Unorm,
                        ..
                    })
                    | Some(AttachmentDesc {
                        format: Format::D16Unorm_S8Uint,
                        ..
                    })
                    | Some(AttachmentDesc {
                        format: Format::D24Unorm_S8Uint,
                        ..
                    })
                    | Some(AttachmentDesc {
                        format: Format::D32Sfloat,
                        ..
                    })
                    | Some(AttachmentDesc {
                        format: Format::D32Sfloat_S8Uint,
                        ..
                    })
                    | None => true,
                    _ => false,
                }
            }
            for pass in passes {
                if !is_valid_depth_attachment(*pass.depth_input.borrow()) {
                    return Err("Cannot set non-depth attachment to depth input.");
                }
                if !is_valid_depth_attachment(*pass.depth_output.borrow()) {
                    return Err("Cannot set non-depth attachment to depth output.");
                }
            }
            Ok(())
        }
        fn schedule_passes<'a, I>(
            passes: I,
            arena: &'a Arena<PassNode<'a, 'rb>>,
        ) -> Result<Vec<&'a PassNode<'a, 'rb>>, &'static str>
        where
            I: Iterator<Item = &'a &'rb PassDesc<'rb>>,
            'rb: 'a,
        {
            let mut pass_nodes: HashMap<&str, &'a PassNode<'a, 'rb>> = HashMap::new();
            let mut root_nodes: BinaryHeap<RootNode<'a, 'rb>> = BinaryHeap::new();
            for pass in passes {
                if pass_nodes.contains_key(pass.name) {
                    return Err("Pass name collision");
                }
                pass_nodes.insert(
                    pass.name,
                    arena.alloc(PassNode {
                        pass,
                        dependents: RefCell::new(Vec::new()),
                        dependencies: RefCell::new(Vec::new()),
                    }),
                );
            }
            for pass_node in pass_nodes.values() {
                let pass = pass_node.pass;
                for color_input in pass.color_inputs.borrow().iter() {
                    for writer in color_input.writers.borrow().iter() {
                        pass_node
                            .dependencies
                            .borrow_mut()
                            .push(PassNodeDependency {
                                pass_node: pass_nodes.get(writer.name).unwrap(),
                                attachment: color_input,
                                is_edge: Cell::new(true),
                            });
                    }
                }
                if pass.depth_input.borrow().is_some() {
                    let depth_input = pass.depth_input.borrow().unwrap();
                    for writer in depth_input.writers.borrow().iter() {
                        pass_node
                            .dependencies
                            .borrow_mut()
                            .push(PassNodeDependency {
                                pass_node: pass_nodes.get(writer.name).unwrap(),
                                attachment: depth_input,
                                is_edge: Cell::new(true),
                            })
                    }
                }
                for color_output in pass.color_outputs.borrow().iter() {
                    for reader in color_output.readers.borrow().iter() {
                        pass_node.dependents.borrow_mut().push(PassNodeDependency {
                            pass_node: pass_nodes.get(reader.name).unwrap(),
                            attachment: color_output,
                            is_edge: Cell::new(true),
                        });
                    }
                }
                if pass.depth_output.borrow().is_some() {
                    let depth_output = pass.depth_output.borrow().unwrap();
                    for reader in depth_output.readers.borrow().iter() {
                        pass_node.dependents.borrow_mut().push(PassNodeDependency {
                            pass_node: pass_nodes.get(reader.name).unwrap(),
                            attachment: depth_output,
                            is_edge: Cell::new(true),
                        })
                    }
                }
                if pass_node.is_independent() {
                    root_nodes.push(RootNode {
                        node: pass_node,
                        overlap_score: usize::MAX,
                    });
                }
            }
            let mut sorted_passes: Vec<&'a PassNode<'a, 'rb>> = Vec::new();
            let mut visited: HashSet<&str> = HashSet::new();
            while !root_nodes.is_empty() {
                let current_pass = root_nodes.pop().unwrap().node;
                sorted_passes.push(current_pass);
                for dependent in current_pass.dependents.borrow().iter() {
                    if !dependent.is_edge.get() {
                        continue;
                    }
                    dependent.is_edge.replace(false);
                    for dependency in dependent.pass_node.dependencies.borrow().iter() {
                        if eq(dependency.pass_node, current_pass) {
                            dependency.is_edge.replace(false);
                        }
                    }
                    if dependent.pass_node.is_independent()
                        && !visited.contains(dependent.pass_node.pass.name)
                    {
                        let overlap_score: usize = sorted_passes.iter().fold(0, |s, sorted| {
                            if !dependent.pass_node.depends_on(sorted) {
                                return s + 1;
                            }
                            return s;
                        });
                        root_nodes.push(RootNode {
                            node: dependent.pass_node,
                            overlap_score,
                        });
                        visited.insert(dependent.pass_node.pass.name);
                    }
                }
            }
            for pass_node in pass_nodes.values() {
                if !pass_node.is_independent() {
                    return Err("Cyclical render graph provided");
                }
            }
            Ok(sorted_passes)
        }
        pub fn build(&'rb self) -> Result<Renderer, &'static str> {
            let passes = self.passes.borrow();
            {
                ::std::io::_print(::core::fmt::Arguments::new_v1(
                    &["Validating passes...\n"],
                    &match () {
                        () => [],
                    },
                ));
            };
            RendererBuilder::validate_passes(passes.iter())?;
            {
                ::std::io::_print(::core::fmt::Arguments::new_v1(
                    &["Scheduling passes...\n"],
                    &match () {
                        () => [],
                    },
                ));
            };
            let pass_node_arena: Arena<PassNode<'_, 'rb>> = Arena::new();
            let scheduled_passes =
                RendererBuilder::schedule_passes(passes.iter(), &pass_node_arena)?;
            {
                ::std::io::_print(::core::fmt::Arguments::new_v1(
                    &["Pass scheduling complete. Result:\n\n"],
                    &match () {
                        () => [],
                    },
                ));
            };
            for (i, pass_node) in scheduled_passes.iter().enumerate() {
                ::std::io::_print(::core::fmt::Arguments::new_v1(
                    &["index ", ": "],
                    &match (&i,) {
                        (arg0,) => [::core::fmt::ArgumentV1::new(
                            arg0,
                            ::core::fmt::Display::fmt,
                        )],
                    },
                ));
                pass_node.display();
            }
            {
                ::std::io::_print(::core::fmt::Arguments::new_v1(
                    &["\nCreating vulkan resources\n\n"],
                    &match () {
                        () => [],
                    },
                ));
            };
            return Ok(Renderer {});
        }
    }
    pub struct Renderer {}
}
mod test_renderer {
    use vulkano::format::Format;
    pub fn build() -> Result<crate::rendering::Renderer, &'static str> {
        let builder = crate::rendering::RendererBuilder::new();
        let depth = builder.add_attachment("depth", Format::D24Unorm_S8Uint, 1);
        let albedo = builder.add_attachment("albedo", Format::R8G8B8A8Unorm, 1);
        let normal = builder.add_attachment("normal", Format::R8G8Unorm, 1);
        let color = builder.add_attachment("color", Format::R8G8B8A8Unorm, 1);
        let blur = builder.add_attachment("blur", Format::R8G8B8A8Unorm, 1);
        let blur2 = builder.add_attachment("blur2", Format::R8G8B8A8Unorm, 1);
        let velocity = builder.add_attachment("velocity", Format::R8G8B8A8Unorm, 1);
        let motion_blur = builder.add_attachment("motion_blur", Format::R8G8B8A8Unorm, 1);
        {
            let gbuffer = builder.add_pass("gbuffer");
            gbuffer.add_color_output(albedo);
            gbuffer.add_color_output(normal);
            gbuffer.set_depth_output(depth);
        }
        {
            let lighting = builder.add_pass("lighting");
            lighting.add_color_output(color);
            lighting.add_color_input(albedo);
            lighting.add_color_input(normal);
            lighting.set_depth_input(depth);
        }
        {
            let blur_pass = builder.add_pass("blur_pass");
            blur_pass.add_color_output(blur);
            blur_pass.add_color_input(color);
        }
        {
            let blur_pass2 = builder.add_pass("blur_pass2");
            blur_pass2.add_color_output(blur2);
            blur_pass2.add_color_input(blur);
        }
        {
            let composite_pass = builder.add_pass("composite_pass");
            let backbuffer = builder.get_backbuffer_attachment();
            composite_pass.add_color_output(backbuffer);
            composite_pass.add_color_input(color);
            composite_pass.add_color_input(blur);
            composite_pass.add_color_input(blur2);
            composite_pass.add_color_input(motion_blur);
        }
        {
            let velocity_pass = builder.add_pass("velocity_pass");
            velocity_pass.add_color_output(velocity);
        }
        {
            let motion_blur_pass = builder.add_pass("motion_blur_pass");
            motion_blur_pass.add_color_output(motion_blur);
            motion_blur_pass.add_color_input(velocity);
            motion_blur_pass.add_color_input(color);
        }
        return builder.build();
    }
}
struct Vertex {
    position: [f32; 2],
    color: [f32; 3],
}
#[automatically_derived]
#[allow(unused_qualifications)]
impl ::core::default::Default for Vertex {
    #[inline]
    fn default() -> Vertex {
        Vertex {
            position: ::core::default::Default::default(),
            color: ::core::default::Default::default(),
        }
    }
}
#[automatically_derived]
#[allow(unused_qualifications)]
impl ::core::marker::Copy for Vertex {}
#[automatically_derived]
#[allow(unused_qualifications)]
impl ::core::clone::Clone for Vertex {
    #[inline]
    fn clone(&self) -> Vertex {
        {
            let _: ::core::clone::AssertParamIsClone<[f32; 2]>;
            let _: ::core::clone::AssertParamIsClone<[f32; 3]>;
            *self
        }
    }
}
#[allow(unsafe_code)]
unsafe impl ::vulkano::pipeline::vertex::Vertex for Vertex {
    #[inline(always)]
    fn member(name: &str) -> Option<::vulkano::pipeline::vertex::VertexMemberInfo> {
        use std::ptr;
        #[allow(unused_imports)]
        use ::vulkano::format::Format;
        use ::vulkano::pipeline::vertex::VertexMemberInfo;
        use ::vulkano::pipeline::vertex::VertexMemberTy;
        use ::vulkano::pipeline::vertex::VertexMember;
        if name == "position" {
            let dummy = <Vertex>::default();
            #[inline]
            fn f<T: VertexMember>(_: &T) -> (VertexMemberTy, usize) {
                T::format()
            }
            let (ty, array_size) = f(&dummy.position);
            let dummy_ptr = (&dummy) as *const _;
            let member_ptr = (&dummy.position) as *const _;
            return Some(VertexMemberInfo {
                offset: member_ptr as usize - dummy_ptr as usize,
                ty: ty,
                array_size: array_size,
            });
        }
        if name == "color" {
            let dummy = <Vertex>::default();
            #[inline]
            fn f<T: VertexMember>(_: &T) -> (VertexMemberTy, usize) {
                T::format()
            }
            let (ty, array_size) = f(&dummy.color);
            let dummy_ptr = (&dummy) as *const _;
            let member_ptr = (&dummy.color) as *const _;
            return Some(VertexMemberInfo {
                offset: member_ptr as usize - dummy_ptr as usize,
                ty: ty,
                array_size: array_size,
            });
        }
        None
    }
}
struct InstanceData {
    position_offset: [f32; 2],
    scale: f32,
}
#[automatically_derived]
#[allow(unused_qualifications)]
impl ::core::default::Default for InstanceData {
    #[inline]
    fn default() -> InstanceData {
        InstanceData {
            position_offset: ::core::default::Default::default(),
            scale: ::core::default::Default::default(),
        }
    }
}
#[automatically_derived]
#[allow(unused_qualifications)]
impl ::core::marker::Copy for InstanceData {}
#[automatically_derived]
#[allow(unused_qualifications)]
impl ::core::clone::Clone for InstanceData {
    #[inline]
    fn clone(&self) -> InstanceData {
        {
            let _: ::core::clone::AssertParamIsClone<[f32; 2]>;
            let _: ::core::clone::AssertParamIsClone<f32>;
            *self
        }
    }
}
#[allow(unsafe_code)]
unsafe impl ::vulkano::pipeline::vertex::Vertex for InstanceData {
    #[inline(always)]
    fn member(name: &str) -> Option<::vulkano::pipeline::vertex::VertexMemberInfo> {
        use std::ptr;
        #[allow(unused_imports)]
        use ::vulkano::format::Format;
        use ::vulkano::pipeline::vertex::VertexMemberInfo;
        use ::vulkano::pipeline::vertex::VertexMemberTy;
        use ::vulkano::pipeline::vertex::VertexMember;
        if name == "position_offset" {
            let dummy = <InstanceData>::default();
            #[inline]
            fn f<T: VertexMember>(_: &T) -> (VertexMemberTy, usize) {
                T::format()
            }
            let (ty, array_size) = f(&dummy.position_offset);
            let dummy_ptr = (&dummy) as *const _;
            let member_ptr = (&dummy.position_offset) as *const _;
            return Some(VertexMemberInfo {
                offset: member_ptr as usize - dummy_ptr as usize,
                ty: ty,
                array_size: array_size,
            });
        }
        if name == "scale" {
            let dummy = <InstanceData>::default();
            #[inline]
            fn f<T: VertexMember>(_: &T) -> (VertexMemberTy, usize) {
                T::format()
            }
            let (ty, array_size) = f(&dummy.scale);
            let dummy_ptr = (&dummy) as *const _;
            let member_ptr = (&dummy.scale) as *const _;
            return Some(VertexMemberInfo {
                offset: member_ptr as usize - dummy_ptr as usize,
                ty: ty,
                array_size: array_size,
            });
        }
        None
    }
}
fn device_rank(physical: &PhysicalDevice) -> u64 {
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
    test_renderer::build().unwrap();
    let instance = Instance::new(None, &InstanceExtensions::none(), None)
        .expect("Failed to create vulkan instance");
    let physical = PhysicalDevice::enumerate(&instance)
        .max_by(|x, y| device_rank(&x).cmp(&device_rank(&y)))
        .expect("No physical device available");
    {
        ::std::io::_print(::core::fmt::Arguments::new_v1(
            &["Physical device chosen: ", "\n"],
            &match (&physical.name(),) {
                (arg0,) => [::core::fmt::ArgumentV1::new(
                    arg0,
                    ::core::fmt::Display::fmt,
                )],
            },
        ));
    };
    for family in physical.queue_families() {
        {
            ::std::io::_print(::core::fmt::Arguments::new_v1(
                &["Found a queue family with ", " queue(s)\n"],
                &match (&family.queues_count(),) {
                    (arg0,) => [::core::fmt::ArgumentV1::new(arg0, ::core::fmt::Debug::fmt)],
                },
            ));
        };
    }
    let queue_family = physical
        .queue_families()
        .find(|&q| q.supports_graphics() && q.supports_compute())
        .expect("Couldn't find a valid queue family");
    let (device, mut queues) = {
        Device::new(
            physical,
            &Features::none(),
            &DeviceExtensions {
                khr_storage_buffer_storage_class: true,
                ..DeviceExtensions::none()
            },
            [(queue_family, 0.5)].iter().cloned(),
        )
        .expect("Failed to create device")
    };
    let queue = queues.next().unwrap();
    let vertex_buffer = CpuAccessibleBuffer::from_iter(
        device.clone(),
        BufferUsage::all(),
        false,
        <[_]>::into_vec(box [
            Vertex {
                position: [-0.5, -0.5],
                color: [1.0, 1.0, 0.0],
            },
            Vertex {
                position: [0.0, 0.5],
                color: [1.0, 0.0, 1.0],
            },
            Vertex {
                position: [0.5, -0.25],
                color: [0.0, 1.0, 1.0],
            },
        ])
        .into_iter(),
    )
    .unwrap();
    let instance_buffer = CpuAccessibleBuffer::from_iter(
        device.clone(),
        BufferUsage::all(),
        false,
        <[_]>::into_vec(box [
            InstanceData {
                position_offset: [-0.5, -0.5],
                scale: 1.0,
            },
            InstanceData {
                position_offset: [0.0, 0.5],
                scale: 0.5,
            },
            InstanceData {
                position_offset: [0.5, -0.25],
                scale: 0.75,
            },
        ])
        .into_iter(),
    )
    .unwrap();
    let render_pass = Arc :: new ({ use :: vulkano :: framebuffer :: RenderPassDesc ; mod scope { # ! [allow (non_camel_case_types)] # ! [allow (non_snake_case)] use :: vulkano :: format :: ClearValue ; use :: vulkano :: format :: Format ; use :: vulkano :: framebuffer :: RenderPassDesc ; use :: vulkano :: framebuffer :: RenderPassDescClearValues ; use :: vulkano :: framebuffer :: AttachmentDescription ; use :: vulkano :: framebuffer :: PassDescription ; use :: vulkano :: framebuffer :: PassDependencyDescription ; use :: vulkano :: image :: ImageLayout ; use :: vulkano :: sync :: AccessFlagBits ; use :: vulkano :: sync :: PipelineStages ; pub struct CustomRenderPassDesc { pub color : (Format , u32) , } # [allow (unsafe_code)] unsafe impl RenderPassDesc for CustomRenderPassDesc { # [inline] fn num_attachments (& self) -> usize { num_attachments () } # [inline] fn attachment_desc (& self , id : usize) -> Option < AttachmentDescription > { attachment (self , id) } # [inline] fn num_subpasses (& self) -> usize { num_subpasses () } # [inline] fn subpass_desc (& self , id : usize) -> Option < PassDescription > { subpass (id) } # [inline] fn num_dependencies (& self) -> usize { num_dependencies () } # [inline] fn dependency_desc (& self , id : usize) -> Option < PassDependencyDescription > { dependency (id) } } unsafe impl RenderPassDescClearValues < Vec < ClearValue > > for CustomRenderPassDesc { fn convert_clear_values (& self , values : Vec < ClearValue >) -> Box < dyn Iterator < Item = ClearValue > > { Box :: new (values . into_iter ()) } } # [inline] fn num_attachments () -> usize { # ! [allow (unused_assignments)] # ! [allow (unused_mut)] # ! [allow (unused_variables)] let mut num = 0 ; let color = num ; num += 1 ; num } # [inline] fn attachment (desc : & CustomRenderPassDesc , id : usize) -> Option < AttachmentDescription > { # ! [allow (unused_assignments)] # ! [allow (unused_mut)] let mut num = 0 ; { if id == num { let (initial_layout , final_layout) = attachment_layouts (num) ; return Some (:: vulkano :: framebuffer :: AttachmentDescription { format : desc . color . 0 , samples : desc . color . 1 , load : :: vulkano :: framebuffer :: LoadOp :: Clear , store : :: vulkano :: framebuffer :: StoreOp :: Store , stencil_load : :: vulkano :: framebuffer :: LoadOp :: Clear , stencil_store : :: vulkano :: framebuffer :: StoreOp :: Store , initial_layout : initial_layout , final_layout : final_layout , }) ; } num += 1 ; } None } # [inline] fn num_subpasses () -> usize { # ! [allow (unused_assignments)] # ! [allow (unused_mut)] # ! [allow (unused_variables)] let mut num = 0 ; let color = num ; num += 1 ; num } # [inline] fn subpass (id : usize) -> Option < PassDescription > { # ! [allow (unused_assignments)] # ! [allow (unused_mut)] # ! [allow (unused_variables)] let mut attachment_num = 0 ; let color = attachment_num ; attachment_num += 1 ; let mut cur_pass_num = 0 ; { if id == cur_pass_num { let mut depth = None ; let mut desc = PassDescription { color_attachments : < [_] > :: into_vec (box [(color , ImageLayout :: ColorAttachmentOptimal)]) , depth_stencil : depth , input_attachments : :: alloc :: vec :: Vec :: new () , resolve_attachments : :: alloc :: vec :: Vec :: new () , preserve_attachments : (0 .. attachment_num) . filter (| & a | { if a == color { return false ; } true }) . collect () , } ; if ! (desc . resolve_attachments . is_empty () || desc . resolve_attachments . len () == desc . color_attachments . len ()) { { :: std :: rt :: begin_panic ("assertion failed: desc.resolve_attachments.is_empty() ||\n    desc.resolve_attachments.len() == desc.color_attachments.len()") } } ; return Some (desc) ; } cur_pass_num += 1 ; } None } # [inline] fn num_dependencies () -> usize { num_subpasses () . saturating_sub (1) } # [inline] fn dependency (id : usize) -> Option < PassDependencyDescription > { let num_passes = num_subpasses () ; if id + 1 >= num_passes { return None ; } Some (PassDependencyDescription { source_subpass : id , destination_subpass : id + 1 , source_stages : PipelineStages { all_graphics : true , .. PipelineStages :: none () } , destination_stages : PipelineStages { all_graphics : true , .. PipelineStages :: none () } , source_access : AccessFlagBits :: all () , destination_access : AccessFlagBits :: all () , by_region : true , }) } # [doc = " Returns the initial and final layout of an attachment, given its num."] # [doc = ""] # [doc = " The value always correspond to the first and last usages of an attachment."] fn attachment_layouts (num : usize) -> (ImageLayout , ImageLayout) { # ! [allow (unused_assignments)] # ! [allow (unused_mut)] # ! [allow (unused_variables)] let mut attachment_num = 0 ; let color = attachment_num ; attachment_num += 1 ; let mut initial_layout = None ; let mut final_layout = None ; { if color == num { if initial_layout . is_none () { initial_layout = Some (ImageLayout :: ColorAttachmentOptimal) ; } final_layout = Some (ImageLayout :: ColorAttachmentOptimal) ; } } if color == num { } (initial_layout . expect ({ let res = :: alloc :: fmt :: format (:: core :: fmt :: Arguments :: new_v1 (& ["Attachment " , " is missing initial_layout, this is normally automatically determined but you can manually specify it for an individual attachment in the single_pass_renderpass! macro"] , & match (& attachment_num ,) { (arg0 ,) => [:: core :: fmt :: ArgumentV1 :: new (arg0 , :: core :: fmt :: Display :: fmt)] , })) ; res } . as_ref ()) , final_layout . expect ({ let res = :: alloc :: fmt :: format (:: core :: fmt :: Arguments :: new_v1 (& ["Attachment " , " is missing final_layout, this is normally automatically determined but you can manually specify it for an individual attachment in the single_pass_renderpass! macro"] , & match (& attachment_num ,) { (arg0 ,) => [:: core :: fmt :: ArgumentV1 :: new (arg0 , :: core :: fmt :: Display :: fmt)] , })) ; res } . as_ref ())) } } scope :: CustomRenderPassDesc { color : (Format :: R8G8B8A8Unorm , 1) , } . build_render_pass (device . clone ()) } . unwrap ()) ;
    let image = StorageImage::new(
        device.clone(),
        Dimensions::Dim2d {
            width: 1024,
            height: 1024,
        },
        Format::R8G8B8A8Unorm,
        Some(queue.family()),
    )
    .unwrap();
    let image_buf = CpuAccessibleBuffer::from_iter(
        device.clone(),
        BufferUsage::all(),
        false,
        (0..1024 * 1024 * 4).map(|_| 0u8),
    )
    .expect("Failed to create image buf");
    let framebuffer = Arc::new(
        Framebuffer::start(render_pass.clone())
            .add(image.clone())
            .unwrap()
            .build()
            .unwrap(),
    );
    mod vs {
        #[allow(unused_imports)]
        use std::sync::Arc;
        #[allow(unused_imports)]
        use std::vec::IntoIter as VecIntoIter;
        #[allow(unused_imports)]
        use vulkano::device::Device;
        #[allow(unused_imports)]
        use vulkano::descriptor::descriptor::DescriptorDesc;
        #[allow(unused_imports)]
        use vulkano::descriptor::descriptor::DescriptorDescTy;
        #[allow(unused_imports)]
        use vulkano::descriptor::descriptor::DescriptorBufferDesc;
        #[allow(unused_imports)]
        use vulkano::descriptor::descriptor::DescriptorImageDesc;
        #[allow(unused_imports)]
        use vulkano::descriptor::descriptor::DescriptorImageDescDimensions;
        #[allow(unused_imports)]
        use vulkano::descriptor::descriptor::DescriptorImageDescArray;
        #[allow(unused_imports)]
        use vulkano::descriptor::descriptor::ShaderStages;
        #[allow(unused_imports)]
        use vulkano::descriptor::descriptor_set::DescriptorSet;
        #[allow(unused_imports)]
        use vulkano::descriptor::descriptor_set::UnsafeDescriptorSet;
        #[allow(unused_imports)]
        use vulkano::descriptor::descriptor_set::UnsafeDescriptorSetLayout;
        #[allow(unused_imports)]
        use vulkano::descriptor::pipeline_layout::PipelineLayout;
        #[allow(unused_imports)]
        use vulkano::descriptor::pipeline_layout::PipelineLayoutDesc;
        #[allow(unused_imports)]
        use vulkano::descriptor::pipeline_layout::PipelineLayoutDescPcRange;
        #[allow(unused_imports)]
        use vulkano::pipeline::shader::SpecializationConstants as SpecConstsTrait;
        #[allow(unused_imports)]
        use vulkano::pipeline::shader::SpecializationMapEntry;
        pub struct Shader {
            shader: ::std::sync::Arc<::vulkano::pipeline::shader::ShaderModule>,
        }
        impl Shader {
            /// Loads the shader in Vulkan as a `ShaderModule`.
            #[inline]
            #[allow(unsafe_code)]
            pub fn load(
                device: ::std::sync::Arc<::vulkano::device::Device>,
            ) -> Result<Shader, ::vulkano::OomError> {
                let words = [
                    119734787u32,
                    66304u32,
                    851976u32,
                    40u32,
                    0u32,
                    131089u32,
                    1u32,
                    393227u32,
                    1u32,
                    1280527431u32,
                    1685353262u32,
                    808793134u32,
                    0u32,
                    196622u32,
                    0u32,
                    1u32,
                    720911u32,
                    0u32,
                    4u32,
                    1852399981u32,
                    0u32,
                    13u32,
                    18u32,
                    21u32,
                    24u32,
                    36u32,
                    38u32,
                    196611u32,
                    2u32,
                    450u32,
                    655364u32,
                    1197427783u32,
                    1279741775u32,
                    1885560645u32,
                    1953718128u32,
                    1600482425u32,
                    1701734764u32,
                    1919509599u32,
                    1769235301u32,
                    25974u32,
                    524292u32,
                    1197427783u32,
                    1279741775u32,
                    1852399429u32,
                    1685417059u32,
                    1768185701u32,
                    1952671090u32,
                    6649449u32,
                    262149u32,
                    4u32,
                    1852399981u32,
                    0u32,
                    393221u32,
                    11u32,
                    1348430951u32,
                    1700164197u32,
                    2019914866u32,
                    0u32,
                    393222u32,
                    11u32,
                    0u32,
                    1348430951u32,
                    1953067887u32,
                    7237481u32,
                    458758u32,
                    11u32,
                    1u32,
                    1348430951u32,
                    1953393007u32,
                    1702521171u32,
                    0u32,
                    458758u32,
                    11u32,
                    2u32,
                    1130327143u32,
                    1148217708u32,
                    1635021673u32,
                    6644590u32,
                    458758u32,
                    11u32,
                    3u32,
                    1130327143u32,
                    1147956341u32,
                    1635021673u32,
                    6644590u32,
                    196613u32,
                    13u32,
                    0u32,
                    327685u32,
                    18u32,
                    1769172848u32,
                    1852795252u32,
                    0u32,
                    262149u32,
                    21u32,
                    1818321779u32,
                    101u32,
                    393221u32,
                    24u32,
                    1769172848u32,
                    1852795252u32,
                    1717989215u32,
                    7628147u32,
                    327685u32,
                    36u32,
                    1601467759u32,
                    1869377379u32,
                    114u32,
                    262149u32,
                    38u32,
                    1869377379u32,
                    114u32,
                    327752u32,
                    11u32,
                    0u32,
                    11u32,
                    0u32,
                    327752u32,
                    11u32,
                    1u32,
                    11u32,
                    1u32,
                    327752u32,
                    11u32,
                    2u32,
                    11u32,
                    3u32,
                    327752u32,
                    11u32,
                    3u32,
                    11u32,
                    4u32,
                    196679u32,
                    11u32,
                    2u32,
                    262215u32,
                    18u32,
                    30u32,
                    0u32,
                    262215u32,
                    21u32,
                    30u32,
                    3u32,
                    262215u32,
                    24u32,
                    30u32,
                    2u32,
                    262215u32,
                    36u32,
                    30u32,
                    0u32,
                    262215u32,
                    38u32,
                    30u32,
                    1u32,
                    131091u32,
                    2u32,
                    196641u32,
                    3u32,
                    2u32,
                    196630u32,
                    6u32,
                    32u32,
                    262167u32,
                    7u32,
                    6u32,
                    4u32,
                    262165u32,
                    8u32,
                    32u32,
                    0u32,
                    262187u32,
                    8u32,
                    9u32,
                    1u32,
                    262172u32,
                    10u32,
                    6u32,
                    9u32,
                    393246u32,
                    11u32,
                    7u32,
                    6u32,
                    10u32,
                    10u32,
                    262176u32,
                    12u32,
                    3u32,
                    11u32,
                    262203u32,
                    12u32,
                    13u32,
                    3u32,
                    262165u32,
                    14u32,
                    32u32,
                    1u32,
                    262187u32,
                    14u32,
                    15u32,
                    0u32,
                    262167u32,
                    16u32,
                    6u32,
                    2u32,
                    262176u32,
                    17u32,
                    1u32,
                    16u32,
                    262203u32,
                    17u32,
                    18u32,
                    1u32,
                    262176u32,
                    20u32,
                    1u32,
                    6u32,
                    262203u32,
                    20u32,
                    21u32,
                    1u32,
                    262203u32,
                    17u32,
                    24u32,
                    1u32,
                    262187u32,
                    6u32,
                    27u32,
                    0u32,
                    262187u32,
                    6u32,
                    28u32,
                    1065353216u32,
                    262176u32,
                    32u32,
                    3u32,
                    7u32,
                    262167u32,
                    34u32,
                    6u32,
                    3u32,
                    262176u32,
                    35u32,
                    3u32,
                    34u32,
                    262203u32,
                    35u32,
                    36u32,
                    3u32,
                    262176u32,
                    37u32,
                    1u32,
                    34u32,
                    262203u32,
                    37u32,
                    38u32,
                    1u32,
                    327734u32,
                    2u32,
                    4u32,
                    0u32,
                    3u32,
                    131320u32,
                    5u32,
                    262205u32,
                    16u32,
                    19u32,
                    18u32,
                    262205u32,
                    6u32,
                    22u32,
                    21u32,
                    327822u32,
                    16u32,
                    23u32,
                    19u32,
                    22u32,
                    262205u32,
                    16u32,
                    25u32,
                    24u32,
                    327809u32,
                    16u32,
                    26u32,
                    23u32,
                    25u32,
                    327761u32,
                    6u32,
                    29u32,
                    26u32,
                    0u32,
                    327761u32,
                    6u32,
                    30u32,
                    26u32,
                    1u32,
                    458832u32,
                    7u32,
                    31u32,
                    29u32,
                    30u32,
                    27u32,
                    28u32,
                    327745u32,
                    32u32,
                    33u32,
                    13u32,
                    15u32,
                    196670u32,
                    33u32,
                    31u32,
                    262205u32,
                    34u32,
                    39u32,
                    38u32,
                    196670u32,
                    36u32,
                    39u32,
                    65789u32,
                    65592u32,
                ];
                unsafe {
                    Ok(Shader {
                        shader: ::vulkano::pipeline::shader::ShaderModule::from_words(
                            device, &words,
                        )?,
                    })
                }
            }
            /// Returns the module that was created.
            #[allow(dead_code)]
            #[inline]
            pub fn module(&self) -> &::std::sync::Arc<::vulkano::pipeline::shader::ShaderModule> {
                &self.shader
            }
            /// Returns a logical struct describing the entry point named `{ep_name}`.
            #[inline]
            #[allow(unsafe_code)]
            pub fn main_entry_point(
                &self,
            ) -> ::vulkano::pipeline::shader::GraphicsEntryPoint<(), MainInput, MainOutput, Layout>
            {
                unsafe {
                    #[allow(dead_code)]
                    static NAME: [u8; 5usize] = [109u8, 97u8, 105u8, 110u8, 0];
                    self.shader.graphics_entry_point(
                        ::std::ffi::CStr::from_ptr(NAME.as_ptr() as *const _),
                        MainInput,
                        MainOutput,
                        Layout(ShaderStages {
                            vertex: true,
                            ..ShaderStages::none()
                        }),
                        ::vulkano::pipeline::shader::GraphicsShaderType::Vertex,
                    )
                }
            }
        }
        pub struct MainInput;
        #[automatically_derived]
        #[allow(unused_qualifications)]
        impl ::core::fmt::Debug for MainInput {
            fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
                match *self {
                    MainInput => {
                        let mut debug_trait_builder = f.debug_tuple("MainInput");
                        debug_trait_builder.finish()
                    }
                }
            }
        }
        #[automatically_derived]
        #[allow(unused_qualifications)]
        impl ::core::marker::Copy for MainInput {}
        #[automatically_derived]
        #[allow(unused_qualifications)]
        impl ::core::clone::Clone for MainInput {
            #[inline]
            fn clone(&self) -> MainInput {
                {
                    *self
                }
            }
        }
        impl ::core::marker::StructuralPartialEq for MainInput {}
        #[automatically_derived]
        #[allow(unused_qualifications)]
        impl ::core::cmp::PartialEq for MainInput {
            #[inline]
            fn eq(&self, other: &MainInput) -> bool {
                match *other {
                    MainInput => match *self {
                        MainInput => true,
                    },
                }
            }
        }
        impl ::core::marker::StructuralEq for MainInput {}
        #[automatically_derived]
        #[allow(unused_qualifications)]
        impl ::core::cmp::Eq for MainInput {
            #[inline]
            #[doc(hidden)]
            fn assert_receiver_is_total_eq(&self) -> () {
                {}
            }
        }
        #[automatically_derived]
        #[allow(unused_qualifications)]
        impl ::core::hash::Hash for MainInput {
            fn hash<__H: ::core::hash::Hasher>(&self, state: &mut __H) -> () {
                match *self {
                    MainInput => {}
                }
            }
        }
        #[allow(unsafe_code)]
        unsafe impl ::vulkano::pipeline::shader::ShaderInterfaceDef for MainInput {
            type Iter = MainInputIter;
            fn elements(&self) -> MainInputIter {
                MainInputIter { num: 0 }
            }
        }
        pub struct MainInputIter {
            num: u16,
        }
        #[automatically_derived]
        #[allow(unused_qualifications)]
        impl ::core::fmt::Debug for MainInputIter {
            fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
                match *self {
                    MainInputIter {
                        num: ref __self_0_0,
                    } => {
                        let mut debug_trait_builder = f.debug_struct("MainInputIter");
                        let _ = debug_trait_builder.field("num", &&(*__self_0_0));
                        debug_trait_builder.finish()
                    }
                }
            }
        }
        #[automatically_derived]
        #[allow(unused_qualifications)]
        impl ::core::marker::Copy for MainInputIter {}
        #[automatically_derived]
        #[allow(unused_qualifications)]
        impl ::core::clone::Clone for MainInputIter {
            #[inline]
            fn clone(&self) -> MainInputIter {
                {
                    let _: ::core::clone::AssertParamIsClone<u16>;
                    *self
                }
            }
        }
        impl Iterator for MainInputIter {
            type Item = ::vulkano::pipeline::shader::ShaderInterfaceDefEntry;
            #[inline]
            fn next(&mut self) -> Option<Self::Item> {
                if self.num == 0u16 {
                    self.num += 1;
                    return Some(::vulkano::pipeline::shader::ShaderInterfaceDefEntry {
                        location: 0u32..1u32,
                        format: ::vulkano::format::Format::R32G32Sfloat,
                        name: Some(::std::borrow::Cow::Borrowed("position")),
                    });
                }
                if self.num == 1u16 {
                    self.num += 1;
                    return Some(::vulkano::pipeline::shader::ShaderInterfaceDefEntry {
                        location: 3u32..4u32,
                        format: ::vulkano::format::Format::R32Sfloat,
                        name: Some(::std::borrow::Cow::Borrowed("scale")),
                    });
                }
                if self.num == 2u16 {
                    self.num += 1;
                    return Some(::vulkano::pipeline::shader::ShaderInterfaceDefEntry {
                        location: 2u32..3u32,
                        format: ::vulkano::format::Format::R32G32Sfloat,
                        name: Some(::std::borrow::Cow::Borrowed("position_offset")),
                    });
                }
                if self.num == 3u16 {
                    self.num += 1;
                    return Some(::vulkano::pipeline::shader::ShaderInterfaceDefEntry {
                        location: 1u32..2u32,
                        format: ::vulkano::format::Format::R32G32B32Sfloat,
                        name: Some(::std::borrow::Cow::Borrowed("color")),
                    });
                }
                None
            }
            #[inline]
            fn size_hint(&self) -> (usize, Option<usize>) {
                let len = 4usize - self.num as usize;
                (len, Some(len))
            }
        }
        impl ExactSizeIterator for MainInputIter {}
        pub struct MainOutput;
        #[automatically_derived]
        #[allow(unused_qualifications)]
        impl ::core::fmt::Debug for MainOutput {
            fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
                match *self {
                    MainOutput => {
                        let mut debug_trait_builder = f.debug_tuple("MainOutput");
                        debug_trait_builder.finish()
                    }
                }
            }
        }
        #[automatically_derived]
        #[allow(unused_qualifications)]
        impl ::core::marker::Copy for MainOutput {}
        #[automatically_derived]
        #[allow(unused_qualifications)]
        impl ::core::clone::Clone for MainOutput {
            #[inline]
            fn clone(&self) -> MainOutput {
                {
                    *self
                }
            }
        }
        impl ::core::marker::StructuralPartialEq for MainOutput {}
        #[automatically_derived]
        #[allow(unused_qualifications)]
        impl ::core::cmp::PartialEq for MainOutput {
            #[inline]
            fn eq(&self, other: &MainOutput) -> bool {
                match *other {
                    MainOutput => match *self {
                        MainOutput => true,
                    },
                }
            }
        }
        impl ::core::marker::StructuralEq for MainOutput {}
        #[automatically_derived]
        #[allow(unused_qualifications)]
        impl ::core::cmp::Eq for MainOutput {
            #[inline]
            #[doc(hidden)]
            fn assert_receiver_is_total_eq(&self) -> () {
                {}
            }
        }
        #[automatically_derived]
        #[allow(unused_qualifications)]
        impl ::core::hash::Hash for MainOutput {
            fn hash<__H: ::core::hash::Hasher>(&self, state: &mut __H) -> () {
                match *self {
                    MainOutput => {}
                }
            }
        }
        #[allow(unsafe_code)]
        unsafe impl ::vulkano::pipeline::shader::ShaderInterfaceDef for MainOutput {
            type Iter = MainOutputIter;
            fn elements(&self) -> MainOutputIter {
                MainOutputIter { num: 0 }
            }
        }
        pub struct MainOutputIter {
            num: u16,
        }
        #[automatically_derived]
        #[allow(unused_qualifications)]
        impl ::core::fmt::Debug for MainOutputIter {
            fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
                match *self {
                    MainOutputIter {
                        num: ref __self_0_0,
                    } => {
                        let mut debug_trait_builder = f.debug_struct("MainOutputIter");
                        let _ = debug_trait_builder.field("num", &&(*__self_0_0));
                        debug_trait_builder.finish()
                    }
                }
            }
        }
        #[automatically_derived]
        #[allow(unused_qualifications)]
        impl ::core::marker::Copy for MainOutputIter {}
        #[automatically_derived]
        #[allow(unused_qualifications)]
        impl ::core::clone::Clone for MainOutputIter {
            #[inline]
            fn clone(&self) -> MainOutputIter {
                {
                    let _: ::core::clone::AssertParamIsClone<u16>;
                    *self
                }
            }
        }
        impl Iterator for MainOutputIter {
            type Item = ::vulkano::pipeline::shader::ShaderInterfaceDefEntry;
            #[inline]
            fn next(&mut self) -> Option<Self::Item> {
                if self.num == 0u16 {
                    self.num += 1;
                    return Some(::vulkano::pipeline::shader::ShaderInterfaceDefEntry {
                        location: 0u32..1u32,
                        format: ::vulkano::format::Format::R32G32B32Sfloat,
                        name: Some(::std::borrow::Cow::Borrowed("out_color")),
                    });
                }
                None
            }
            #[inline]
            fn size_hint(&self) -> (usize, Option<usize>) {
                let len = 1usize - self.num as usize;
                (len, Some(len))
            }
        }
        impl ExactSizeIterator for MainOutputIter {}
        pub mod ty {}
        pub struct Layout(pub ShaderStages);
        #[automatically_derived]
        #[allow(unused_qualifications)]
        impl ::core::fmt::Debug for Layout {
            fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
                match *self {
                    Layout(ref __self_0_0) => {
                        let mut debug_trait_builder = f.debug_tuple("Layout");
                        let _ = debug_trait_builder.field(&&(*__self_0_0));
                        debug_trait_builder.finish()
                    }
                }
            }
        }
        #[automatically_derived]
        #[allow(unused_qualifications)]
        impl ::core::clone::Clone for Layout {
            #[inline]
            fn clone(&self) -> Layout {
                match *self {
                    Layout(ref __self_0_0) => Layout(::core::clone::Clone::clone(&(*__self_0_0))),
                }
            }
        }
        #[allow(unsafe_code)]
        unsafe impl PipelineLayoutDesc for Layout {
            fn num_sets(&self) -> usize {
                0usize
            }
            fn num_bindings_in_set(&self, set: usize) -> Option<usize> {
                match set {
                    _ => None,
                }
            }
            fn descriptor(&self, set: usize, binding: usize) -> Option<DescriptorDesc> {
                match (set, binding) {
                    _ => None,
                }
            }
            fn num_push_constants_ranges(&self) -> usize {
                0usize
            }
            fn push_constants_range(&self, num: usize) -> Option<PipelineLayoutDescPcRange> {
                if num != 0 || 0usize == 0 {
                    None
                } else {
                    Some(PipelineLayoutDescPcRange {
                        offset: 0,
                        size: 0usize,
                        stages: ShaderStages::all(),
                    })
                }
            }
        }
        #[allow(non_snake_case)]
        #[repr(C)]
        pub struct SpecializationConstants {}
        #[automatically_derived]
        #[allow(unused_qualifications)]
        #[allow(non_snake_case)]
        impl ::core::fmt::Debug for SpecializationConstants {
            fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
                match *self {
                    SpecializationConstants {} => {
                        let mut debug_trait_builder = f.debug_struct("SpecializationConstants");
                        debug_trait_builder.finish()
                    }
                }
            }
        }
        #[automatically_derived]
        #[allow(unused_qualifications)]
        #[allow(non_snake_case)]
        impl ::core::marker::Copy for SpecializationConstants {}
        #[automatically_derived]
        #[allow(unused_qualifications)]
        #[allow(non_snake_case)]
        impl ::core::clone::Clone for SpecializationConstants {
            #[inline]
            fn clone(&self) -> SpecializationConstants {
                {
                    *self
                }
            }
        }
        impl Default for SpecializationConstants {
            fn default() -> SpecializationConstants {
                SpecializationConstants {}
            }
        }
        unsafe impl SpecConstsTrait for SpecializationConstants {
            fn descriptors() -> &'static [SpecializationMapEntry] {
                static DESCRIPTORS: [SpecializationMapEntry; 0usize] = [];
                &DESCRIPTORS
            }
        }
    }
    mod fs {
        #[allow(unused_imports)]
        use std::sync::Arc;
        #[allow(unused_imports)]
        use std::vec::IntoIter as VecIntoIter;
        #[allow(unused_imports)]
        use vulkano::device::Device;
        #[allow(unused_imports)]
        use vulkano::descriptor::descriptor::DescriptorDesc;
        #[allow(unused_imports)]
        use vulkano::descriptor::descriptor::DescriptorDescTy;
        #[allow(unused_imports)]
        use vulkano::descriptor::descriptor::DescriptorBufferDesc;
        #[allow(unused_imports)]
        use vulkano::descriptor::descriptor::DescriptorImageDesc;
        #[allow(unused_imports)]
        use vulkano::descriptor::descriptor::DescriptorImageDescDimensions;
        #[allow(unused_imports)]
        use vulkano::descriptor::descriptor::DescriptorImageDescArray;
        #[allow(unused_imports)]
        use vulkano::descriptor::descriptor::ShaderStages;
        #[allow(unused_imports)]
        use vulkano::descriptor::descriptor_set::DescriptorSet;
        #[allow(unused_imports)]
        use vulkano::descriptor::descriptor_set::UnsafeDescriptorSet;
        #[allow(unused_imports)]
        use vulkano::descriptor::descriptor_set::UnsafeDescriptorSetLayout;
        #[allow(unused_imports)]
        use vulkano::descriptor::pipeline_layout::PipelineLayout;
        #[allow(unused_imports)]
        use vulkano::descriptor::pipeline_layout::PipelineLayoutDesc;
        #[allow(unused_imports)]
        use vulkano::descriptor::pipeline_layout::PipelineLayoutDescPcRange;
        #[allow(unused_imports)]
        use vulkano::pipeline::shader::SpecializationConstants as SpecConstsTrait;
        #[allow(unused_imports)]
        use vulkano::pipeline::shader::SpecializationMapEntry;
        pub struct Shader {
            shader: ::std::sync::Arc<::vulkano::pipeline::shader::ShaderModule>,
        }
        impl Shader {
            /// Loads the shader in Vulkan as a `ShaderModule`.
            #[inline]
            #[allow(unsafe_code)]
            pub fn load(
                device: ::std::sync::Arc<::vulkano::device::Device>,
            ) -> Result<Shader, ::vulkano::OomError> {
                let words = [
                    119734787u32,
                    66304u32,
                    851976u32,
                    19u32,
                    0u32,
                    131089u32,
                    1u32,
                    393227u32,
                    1u32,
                    1280527431u32,
                    1685353262u32,
                    808793134u32,
                    0u32,
                    196622u32,
                    0u32,
                    1u32,
                    458767u32,
                    4u32,
                    4u32,
                    1852399981u32,
                    0u32,
                    9u32,
                    12u32,
                    196624u32,
                    4u32,
                    7u32,
                    196611u32,
                    2u32,
                    450u32,
                    655364u32,
                    1197427783u32,
                    1279741775u32,
                    1885560645u32,
                    1953718128u32,
                    1600482425u32,
                    1701734764u32,
                    1919509599u32,
                    1769235301u32,
                    25974u32,
                    524292u32,
                    1197427783u32,
                    1279741775u32,
                    1852399429u32,
                    1685417059u32,
                    1768185701u32,
                    1952671090u32,
                    6649449u32,
                    262149u32,
                    4u32,
                    1852399981u32,
                    0u32,
                    262149u32,
                    9u32,
                    1868783462u32,
                    7499628u32,
                    327685u32,
                    12u32,
                    1667198569u32,
                    1919904879u32,
                    0u32,
                    262215u32,
                    9u32,
                    30u32,
                    0u32,
                    262215u32,
                    12u32,
                    30u32,
                    0u32,
                    131091u32,
                    2u32,
                    196641u32,
                    3u32,
                    2u32,
                    196630u32,
                    6u32,
                    32u32,
                    262167u32,
                    7u32,
                    6u32,
                    4u32,
                    262176u32,
                    8u32,
                    3u32,
                    7u32,
                    262203u32,
                    8u32,
                    9u32,
                    3u32,
                    262167u32,
                    10u32,
                    6u32,
                    3u32,
                    262176u32,
                    11u32,
                    1u32,
                    10u32,
                    262203u32,
                    11u32,
                    12u32,
                    1u32,
                    262187u32,
                    6u32,
                    14u32,
                    1065353216u32,
                    327734u32,
                    2u32,
                    4u32,
                    0u32,
                    3u32,
                    131320u32,
                    5u32,
                    262205u32,
                    10u32,
                    13u32,
                    12u32,
                    327761u32,
                    6u32,
                    15u32,
                    13u32,
                    0u32,
                    327761u32,
                    6u32,
                    16u32,
                    13u32,
                    1u32,
                    327761u32,
                    6u32,
                    17u32,
                    13u32,
                    2u32,
                    458832u32,
                    7u32,
                    18u32,
                    15u32,
                    16u32,
                    17u32,
                    14u32,
                    196670u32,
                    9u32,
                    18u32,
                    65789u32,
                    65592u32,
                ];
                unsafe {
                    Ok(Shader {
                        shader: ::vulkano::pipeline::shader::ShaderModule::from_words(
                            device, &words,
                        )?,
                    })
                }
            }
            /// Returns the module that was created.
            #[allow(dead_code)]
            #[inline]
            pub fn module(&self) -> &::std::sync::Arc<::vulkano::pipeline::shader::ShaderModule> {
                &self.shader
            }
            /// Returns a logical struct describing the entry point named `{ep_name}`.
            #[inline]
            #[allow(unsafe_code)]
            pub fn main_entry_point(
                &self,
            ) -> ::vulkano::pipeline::shader::GraphicsEntryPoint<(), MainInput, MainOutput, Layout>
            {
                unsafe {
                    #[allow(dead_code)]
                    static NAME: [u8; 5usize] = [109u8, 97u8, 105u8, 110u8, 0];
                    self.shader.graphics_entry_point(
                        ::std::ffi::CStr::from_ptr(NAME.as_ptr() as *const _),
                        MainInput,
                        MainOutput,
                        Layout(ShaderStages {
                            fragment: true,
                            ..ShaderStages::none()
                        }),
                        ::vulkano::pipeline::shader::GraphicsShaderType::Fragment,
                    )
                }
            }
        }
        pub struct MainInput;
        #[automatically_derived]
        #[allow(unused_qualifications)]
        impl ::core::fmt::Debug for MainInput {
            fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
                match *self {
                    MainInput => {
                        let mut debug_trait_builder = f.debug_tuple("MainInput");
                        debug_trait_builder.finish()
                    }
                }
            }
        }
        #[automatically_derived]
        #[allow(unused_qualifications)]
        impl ::core::marker::Copy for MainInput {}
        #[automatically_derived]
        #[allow(unused_qualifications)]
        impl ::core::clone::Clone for MainInput {
            #[inline]
            fn clone(&self) -> MainInput {
                {
                    *self
                }
            }
        }
        impl ::core::marker::StructuralPartialEq for MainInput {}
        #[automatically_derived]
        #[allow(unused_qualifications)]
        impl ::core::cmp::PartialEq for MainInput {
            #[inline]
            fn eq(&self, other: &MainInput) -> bool {
                match *other {
                    MainInput => match *self {
                        MainInput => true,
                    },
                }
            }
        }
        impl ::core::marker::StructuralEq for MainInput {}
        #[automatically_derived]
        #[allow(unused_qualifications)]
        impl ::core::cmp::Eq for MainInput {
            #[inline]
            #[doc(hidden)]
            fn assert_receiver_is_total_eq(&self) -> () {
                {}
            }
        }
        #[automatically_derived]
        #[allow(unused_qualifications)]
        impl ::core::hash::Hash for MainInput {
            fn hash<__H: ::core::hash::Hasher>(&self, state: &mut __H) -> () {
                match *self {
                    MainInput => {}
                }
            }
        }
        #[allow(unsafe_code)]
        unsafe impl ::vulkano::pipeline::shader::ShaderInterfaceDef for MainInput {
            type Iter = MainInputIter;
            fn elements(&self) -> MainInputIter {
                MainInputIter { num: 0 }
            }
        }
        pub struct MainInputIter {
            num: u16,
        }
        #[automatically_derived]
        #[allow(unused_qualifications)]
        impl ::core::fmt::Debug for MainInputIter {
            fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
                match *self {
                    MainInputIter {
                        num: ref __self_0_0,
                    } => {
                        let mut debug_trait_builder = f.debug_struct("MainInputIter");
                        let _ = debug_trait_builder.field("num", &&(*__self_0_0));
                        debug_trait_builder.finish()
                    }
                }
            }
        }
        #[automatically_derived]
        #[allow(unused_qualifications)]
        impl ::core::marker::Copy for MainInputIter {}
        #[automatically_derived]
        #[allow(unused_qualifications)]
        impl ::core::clone::Clone for MainInputIter {
            #[inline]
            fn clone(&self) -> MainInputIter {
                {
                    let _: ::core::clone::AssertParamIsClone<u16>;
                    *self
                }
            }
        }
        impl Iterator for MainInputIter {
            type Item = ::vulkano::pipeline::shader::ShaderInterfaceDefEntry;
            #[inline]
            fn next(&mut self) -> Option<Self::Item> {
                if self.num == 0u16 {
                    self.num += 1;
                    return Some(::vulkano::pipeline::shader::ShaderInterfaceDefEntry {
                        location: 0u32..1u32,
                        format: ::vulkano::format::Format::R32G32B32Sfloat,
                        name: Some(::std::borrow::Cow::Borrowed("in_color")),
                    });
                }
                None
            }
            #[inline]
            fn size_hint(&self) -> (usize, Option<usize>) {
                let len = 1usize - self.num as usize;
                (len, Some(len))
            }
        }
        impl ExactSizeIterator for MainInputIter {}
        pub struct MainOutput;
        #[automatically_derived]
        #[allow(unused_qualifications)]
        impl ::core::fmt::Debug for MainOutput {
            fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
                match *self {
                    MainOutput => {
                        let mut debug_trait_builder = f.debug_tuple("MainOutput");
                        debug_trait_builder.finish()
                    }
                }
            }
        }
        #[automatically_derived]
        #[allow(unused_qualifications)]
        impl ::core::marker::Copy for MainOutput {}
        #[automatically_derived]
        #[allow(unused_qualifications)]
        impl ::core::clone::Clone for MainOutput {
            #[inline]
            fn clone(&self) -> MainOutput {
                {
                    *self
                }
            }
        }
        impl ::core::marker::StructuralPartialEq for MainOutput {}
        #[automatically_derived]
        #[allow(unused_qualifications)]
        impl ::core::cmp::PartialEq for MainOutput {
            #[inline]
            fn eq(&self, other: &MainOutput) -> bool {
                match *other {
                    MainOutput => match *self {
                        MainOutput => true,
                    },
                }
            }
        }
        impl ::core::marker::StructuralEq for MainOutput {}
        #[automatically_derived]
        #[allow(unused_qualifications)]
        impl ::core::cmp::Eq for MainOutput {
            #[inline]
            #[doc(hidden)]
            fn assert_receiver_is_total_eq(&self) -> () {
                {}
            }
        }
        #[automatically_derived]
        #[allow(unused_qualifications)]
        impl ::core::hash::Hash for MainOutput {
            fn hash<__H: ::core::hash::Hasher>(&self, state: &mut __H) -> () {
                match *self {
                    MainOutput => {}
                }
            }
        }
        #[allow(unsafe_code)]
        unsafe impl ::vulkano::pipeline::shader::ShaderInterfaceDef for MainOutput {
            type Iter = MainOutputIter;
            fn elements(&self) -> MainOutputIter {
                MainOutputIter { num: 0 }
            }
        }
        pub struct MainOutputIter {
            num: u16,
        }
        #[automatically_derived]
        #[allow(unused_qualifications)]
        impl ::core::fmt::Debug for MainOutputIter {
            fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
                match *self {
                    MainOutputIter {
                        num: ref __self_0_0,
                    } => {
                        let mut debug_trait_builder = f.debug_struct("MainOutputIter");
                        let _ = debug_trait_builder.field("num", &&(*__self_0_0));
                        debug_trait_builder.finish()
                    }
                }
            }
        }
        #[automatically_derived]
        #[allow(unused_qualifications)]
        impl ::core::marker::Copy for MainOutputIter {}
        #[automatically_derived]
        #[allow(unused_qualifications)]
        impl ::core::clone::Clone for MainOutputIter {
            #[inline]
            fn clone(&self) -> MainOutputIter {
                {
                    let _: ::core::clone::AssertParamIsClone<u16>;
                    *self
                }
            }
        }
        impl Iterator for MainOutputIter {
            type Item = ::vulkano::pipeline::shader::ShaderInterfaceDefEntry;
            #[inline]
            fn next(&mut self) -> Option<Self::Item> {
                if self.num == 0u16 {
                    self.num += 1;
                    return Some(::vulkano::pipeline::shader::ShaderInterfaceDefEntry {
                        location: 0u32..1u32,
                        format: ::vulkano::format::Format::R32G32B32A32Sfloat,
                        name: Some(::std::borrow::Cow::Borrowed("f_color")),
                    });
                }
                None
            }
            #[inline]
            fn size_hint(&self) -> (usize, Option<usize>) {
                let len = 1usize - self.num as usize;
                (len, Some(len))
            }
        }
        impl ExactSizeIterator for MainOutputIter {}
        pub mod ty {}
        pub struct Layout(pub ShaderStages);
        #[automatically_derived]
        #[allow(unused_qualifications)]
        impl ::core::fmt::Debug for Layout {
            fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
                match *self {
                    Layout(ref __self_0_0) => {
                        let mut debug_trait_builder = f.debug_tuple("Layout");
                        let _ = debug_trait_builder.field(&&(*__self_0_0));
                        debug_trait_builder.finish()
                    }
                }
            }
        }
        #[automatically_derived]
        #[allow(unused_qualifications)]
        impl ::core::clone::Clone for Layout {
            #[inline]
            fn clone(&self) -> Layout {
                match *self {
                    Layout(ref __self_0_0) => Layout(::core::clone::Clone::clone(&(*__self_0_0))),
                }
            }
        }
        #[allow(unsafe_code)]
        unsafe impl PipelineLayoutDesc for Layout {
            fn num_sets(&self) -> usize {
                0usize
            }
            fn num_bindings_in_set(&self, set: usize) -> Option<usize> {
                match set {
                    _ => None,
                }
            }
            fn descriptor(&self, set: usize, binding: usize) -> Option<DescriptorDesc> {
                match (set, binding) {
                    _ => None,
                }
            }
            fn num_push_constants_ranges(&self) -> usize {
                0usize
            }
            fn push_constants_range(&self, num: usize) -> Option<PipelineLayoutDescPcRange> {
                if num != 0 || 0usize == 0 {
                    None
                } else {
                    Some(PipelineLayoutDescPcRange {
                        offset: 0,
                        size: 0usize,
                        stages: ShaderStages::all(),
                    })
                }
            }
        }
        #[allow(non_snake_case)]
        #[repr(C)]
        pub struct SpecializationConstants {}
        #[automatically_derived]
        #[allow(unused_qualifications)]
        #[allow(non_snake_case)]
        impl ::core::fmt::Debug for SpecializationConstants {
            fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
                match *self {
                    SpecializationConstants {} => {
                        let mut debug_trait_builder = f.debug_struct("SpecializationConstants");
                        debug_trait_builder.finish()
                    }
                }
            }
        }
        #[automatically_derived]
        #[allow(unused_qualifications)]
        #[allow(non_snake_case)]
        impl ::core::marker::Copy for SpecializationConstants {}
        #[automatically_derived]
        #[allow(unused_qualifications)]
        #[allow(non_snake_case)]
        impl ::core::clone::Clone for SpecializationConstants {
            #[inline]
            fn clone(&self) -> SpecializationConstants {
                {
                    *self
                }
            }
        }
        impl Default for SpecializationConstants {
            fn default() -> SpecializationConstants {
                SpecializationConstants {}
            }
        }
        unsafe impl SpecConstsTrait for SpecializationConstants {
            fn descriptors() -> &'static [SpecializationMapEntry] {
                static DESCRIPTORS: [SpecializationMapEntry; 0usize] = [];
                &DESCRIPTORS
            }
        }
    }
    let vs = vs::Shader::load(device.clone()).expect("Failed to create VS");
    let fs = fs::Shader::load(device.clone()).expect("Failed to create FS");
    let pipeline = Arc::new(
        GraphicsPipeline::start()
            .vertex_input(OneVertexOneInstanceDefinition::<Vertex, InstanceData>::new())
            .vertex_shader(vs.main_entry_point(), ())
            .viewports_dynamic_scissors_irrelevant(1)
            .fragment_shader(fs.main_entry_point(), ())
            .render_pass(Subpass::from(render_pass.clone(), 0).unwrap())
            .build(device.clone())
            .unwrap(),
    );
    let dynamic_state = DynamicState {
        viewports: Some(<[_]>::into_vec(box [Viewport {
            origin: [0.0, 0.0],
            dimensions: [1024.0, 1024.0],
            depth_range: 0.0..1.0,
        }])),
        ..DynamicState::none()
    };
    let mut cmd_buf_builder =
        AutoCommandBufferBuilder::primary_one_time_submit(device.clone(), queue_family).unwrap();
    cmd_buf_builder
        .begin_render_pass(
            framebuffer.clone(),
            false,
            <[_]>::into_vec(box [[0.0, 0.0, 1.0, 1.0].into()]),
        )
        .unwrap()
        .draw(
            pipeline.clone(),
            &dynamic_state,
            (vertex_buffer.clone(), instance_buffer.clone()),
            (),
            (),
        )
        .unwrap()
        .end_render_pass()
        .unwrap()
        .copy_image_to_buffer(image.clone(), image_buf.clone())
        .unwrap();
    let cmd_buf = cmd_buf_builder.build().unwrap();
    let future = sync::now(device.clone())
        .then_execute(queue.clone(), cmd_buf)
        .unwrap()
        .then_signal_fence_and_flush()
        .unwrap();
    future.wait(None).unwrap();
    {
        ::std::io::_print(::core::fmt::Arguments::new_v1(
            &["Draw done\n"],
            &match () {
                () => [],
            },
        ));
    };
    let image_buf_content = image_buf.read().unwrap();
    let image = ImageBuffer::<Rgba<u8>, _>::from_raw(1024, 1024, &image_buf_content[..]).unwrap();
    image.save("triangle.png").unwrap();
}
