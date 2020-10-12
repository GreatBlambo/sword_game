use typed_arena::Arena;
use vulkano::format::Format;
use std::cell::RefCell;
use std::cell::Cell;

use std::collections::VecDeque;
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
    writers: RefCell<Vec<&'rb PassDesc<'rb>>>
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
    pub fn display(&'rb self) {
        println!("Pass name: {}", self.name);
    }

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

pub struct RendererBuilder<'rb> {
    attachment_arena: Arena<AttachmentDesc<'rb>>,
    pass_arena: Arena<PassDesc<'rb>>,
    passes: RefCell<Vec<&'rb PassDesc<'rb>>>,
    backbuffer_attachment: AttachmentDesc<'rb>
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
                writers: RefCell::new(Vec::new())
            }
        };
    }

    pub fn add_attachment(&'rb self, name: &'static str, format: Format, samples: usize) -> &'rb AttachmentDesc {
        return self.attachment_arena.alloc(AttachmentDesc {
            name,
            format,
            samples,
            readers: RefCell::new(Vec::new()),
            writers: RefCell::new(Vec::new())
        });
    }

    pub fn add_depth_attachment(&'rb self, name: &'static str, samples: usize) -> &'rb AttachmentDesc {
        return self.add_attachment(name, Format::D24Unorm_S8Uint, samples);
    }

    pub fn add_pass(&'rb self, name: &'static str) -> &'rb PassDesc<'rb> {
        let pass = self.pass_arena.alloc(PassDesc {
            name,
            color_inputs: RefCell::new(Vec::new()),
            color_outputs: RefCell::new(Vec::new()),
            depth_input: RefCell::new(None),
            depth_output: RefCell::new(None)
        });

        self.passes.borrow_mut().push(pass);

        return pass;
    }

    pub fn get_backbuffer_attachment(&'rb self) -> &'rb AttachmentDesc<'rb> {
        return &self.backbuffer_attachment;
    }

    pub fn build(&'rb self) -> Result<Renderer, &'static str> {
        // Validate

        // Validate depth formats
        for pass in self.passes.borrow().iter() {
            fn is_valid_depth_attachment<'rb>(attachment: Option<&'rb AttachmentDesc<'rb>>) -> bool {
                match attachment {
                    Some(AttachmentDesc {format: Format::D16Unorm, ..}) |
                    Some(AttachmentDesc {format: Format::D16Unorm_S8Uint, ..}) |
                    Some(AttachmentDesc {format: Format::D24Unorm_S8Uint, ..}) |
                    Some(AttachmentDesc {format: Format::D32Sfloat, ..}) |
                    Some(AttachmentDesc {format: Format::D32Sfloat_S8Uint, ..}) |
                    None
                        => return true,
                    _ => return false 
                }
            }

            if !is_valid_depth_attachment(*pass.depth_input.borrow()) {
                return Err("Cannot set non-depth attachment to depth input.");
            }

            if !is_valid_depth_attachment(*pass.depth_output.borrow()) {
                return Err("Cannot set non-depth attachment to depth output.");
            }
        }

        struct PassNodeDependency<'a, 'rb> {
            pass_node: &'a PassNode<'a, 'rb>,
            attachment: &'rb AttachmentDesc<'rb>,
            is_edge: Cell<bool> // JUST used for toposort
        }

        struct PassNode<'a, 'rb> {
            pass: &'rb PassDesc<'rb>,
            dependents: RefCell<Vec<PassNodeDependency<'a, 'rb>>>,
            dependencies: RefCell<Vec<PassNodeDependency<'a, 'rb>>>
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
        }

        let mut pass_nodes: HashMap<&str, PassNode<'_, 'rb>> = HashMap::new();

        struct RootNode<'a, 'rb> {
            node: &'a PassNode<'a, 'rb>,
            overlap_score: usize
        }

        impl<'a, 'rb> Ord for RootNode<'a, 'rb> {
            fn cmp(&self, other: &Self) -> Ordering {
                self.overlap_score.cmp(&other.overlap_score)
            }
        }
        impl<'a, 'rb> PartialOrd for RootNode<'a, 'rb>{
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

        let mut root_nodes: BinaryHeap<RootNode<'_, 'rb>> = BinaryHeap::new();

        // Create new node objects
        for pass in self.passes.borrow().iter() {
            if pass_nodes.contains_key(pass.name) {
                return Err("Pass name collision");
            }
            pass_nodes.insert(pass.name, PassNode {
                pass,
                dependents: RefCell::new(Vec::new()),
                dependencies: RefCell::new(Vec::new()),
            });
        }

        // Fill in inter-node dependencies
        for pass in self.passes.borrow().iter() {
            let pass_node: &PassNode<'_, 'rb> = pass_nodes.get(pass.name).unwrap();

            for color_input in pass.color_inputs.borrow().iter() {
                for writer in color_input.writers.borrow().iter() {
                    pass_node.dependencies.borrow_mut().push(
                        PassNodeDependency {
                            pass_node: pass_nodes.get(writer.name).unwrap(),
                            attachment: color_input,
                            is_edge: Cell::new(true)
                        }
                    );
                }
            }

            if pass.depth_input.borrow().is_some() {
                let depth_input = pass.depth_input.borrow().unwrap();
                for writer in depth_input.writers.borrow().iter() {
                    pass_node.dependencies.borrow_mut().push(
                        PassNodeDependency {
                            pass_node: pass_nodes.get(writer.name).unwrap(),
                            attachment: depth_input,
                            is_edge: Cell::new(true)
                        }
                    )
                }
            }

            for color_output in pass.color_outputs.borrow().iter() {
                for reader in color_output.readers.borrow().iter() {
                    pass_node.dependents.borrow_mut().push(
                        PassNodeDependency {
                            pass_node: pass_nodes.get(reader.name).unwrap(),
                            attachment: color_output,
                            is_edge: Cell::new(true)
                        }
                    );
                }
            }

            if pass.depth_output.borrow().is_some() {
                let depth_output = pass.depth_output.borrow().unwrap();
                for reader in depth_output.readers.borrow().iter() {
                    pass_node.dependents.borrow_mut().push(
                        PassNodeDependency {
                            pass_node: pass_nodes.get(reader.name).unwrap(),
                            attachment: depth_output,
                            is_edge: Cell::new(true)
                        }
                    )
                }
            }

            if pass_node.is_independent() {
                root_nodes.push(RootNode {
                    node: pass_node,
                    overlap_score: usize::MAX
                });
            }
        }

        let mut sorted_passes: Vec<&PassNode<'_, 'rb>> = Vec::new();

        let mut visited: HashSet<&str> = HashSet::new();
        while !root_nodes.is_empty() {
            // Schedule
            let current_pass = root_nodes.pop().unwrap().node;
            sorted_passes.push(current_pass);

            // Remove edge for color attachments if it exists
            for dependent in current_pass.dependents.borrow().iter() {
                if !dependent.is_edge.get() {
                    continue;
                }

                dependent.is_edge.replace(false);

                // Remove current pass from dependent's dependencies
                for dependency in dependent.pass_node.dependencies.borrow().iter() {
                    if eq(dependency.pass_node, current_pass) {
                        dependency.is_edge.replace(false);
                    }
                }

                // If dependent no longer has dependencies, add to root nodes queue
                if dependent.pass_node.is_independent() && !visited.contains(dependent.pass_node.pass.name) {
                    // Calculate overlap score
                    let overlap_score: usize = sorted_passes.iter().fold(0,
                        |s, sorted| {
                            if dependent.pass_node.depends_on(sorted) {
                                return s + 1;
                            }
                            return s;
                        }
                    );
                    // Insert into queue
                    root_nodes.push(RootNode {
                            node: dependent.pass_node,
                            overlap_score
                        }
                    );
                    visited.insert(dependent.pass_node.pass.name);
                }
            }
        }

        // Check for cycles
        for pass_node in pass_nodes.values() {
            if !pass_node.is_independent() {
                return Err("Cyclical render graph provided");
            }
        }

        // Reorder for optimal pipelining

        println!("\nPass sorting complete. Result:\n");
        for pass_node in sorted_passes.iter() {
            pass_node.pass.display();
        }

        // Create vulkan resources

        println!("\nCreating vulkan resources\n");

        // Map logical attachments to physical attachments

        // Two logical attachments (A, B) can be merged into one physical attachment if:
        // 1. The last write of A completes before the first write of B
        // 2. The last read of A completes before the first write of B



        return Ok(
            Renderer {

            }
        );
    }
}

pub struct Renderer {

}

#[macro_export]
macro_rules! color_output {
    (backbuffer, $gfx_pass_name:ident, $builder:ident) => (
        let backbuffer = $builder.get_backbuffer_attachment();
        $gfx_pass_name.add_color_output(backbuffer);
    );
    ($color_output_atch:ident, $gfx_pass_name:ident, $builder:ident) => (
        $gfx_pass_name.add_color_output($color_output_atch);
    );
}

#[macro_export]
macro_rules! attachment {
    ($atch_name:ident, $builder:ident, $format:expr, $samples:literal) => (
        let $atch_name = $builder.add_attachment(std::stringify!($atch_name), $format, $samples);
    );
    ($atch_name:ident, $builder:ident, $format:expr) => (
        let $atch_name = $builder.add_attachment(std::stringify!($atch_name), $format, 1);
    );
}

#[macro_export(local_inner_macros)]
macro_rules! render_config {
    {
        name: $render_config_name:ident,
        attachments: {
            $(
                $atch_name:ident: {
                    format: $format:expr
                    $(,samples: $samples:literal)?
                }
            ),*
        },
        default_vertex_bindings: [
            $(
                {
                    vertex_type_name: $vertex_type_name:ty,
                    input_rate: $input_rate:literal,
                    attributes: {
                        $(
                            $attribute_name:ident: $attribute_type:ty$(,)?
                        )+
                    }
                }$(,)?
            )+
        ],
        graphics_passes: {
            $(
                $gfx_pass_name:ident: {
                    color_outputs: [$($color_output_atch:ident),*],
                    depth_stencil_output: {$($depth_output_atch:ident)?},
                    color_inputs: [$($color_input_atch:ident),*]$(,)*
                    depth_stencil_input: {$($depth_input_atch:ident)?},
                    pipeline: {
                        shader_paths: {
                            vertex: $vertex_path:literal$(,)?
                            $(geometry: $geometry_path:literal,)?
                            $(tess_ctrl: $tess_ctrl_path:literal,)?
                            $(tess_eval: $tess_eval_path:literal,)?
                            fragment: $fragment_path:literal
                        }
                    }
                }
            ),*
        }
    } => (
        mod $render_config_name {
            use vulkano::format::Format;
            pub fn build() -> Result<crate::rendering::Renderer, &'static str> {
                let builder = crate::rendering::RendererBuilder::new();
                $(
                    attachment!($atch_name, builder, $format$(, $samples)?);
                )*

                $(
                    {
                        // Create pass
                        let $gfx_pass_name = builder.add_pass(std::stringify!($gfx_pass_name));

                        // Add outputs
                        $(
                            color_output!($color_output_atch, $gfx_pass_name, builder);
                        )*
                        $(
                            $gfx_pass_name.set_depth_output($depth_output_atch);
                        )?

                        // Add inputs
                        $(
                            $gfx_pass_name.add_color_input($color_input_atch);
                        )*
                        $(
                            $gfx_pass_name.set_depth_input($depth_input_atch);
                        )?
                    }
                )*

                return builder.build();
            }
        }
    )
}
