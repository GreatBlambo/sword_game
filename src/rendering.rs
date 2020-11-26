use typed_arena::Arena;

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
    format: vulkano::format::Format,
    samples: usize,
    usage: Cell<vulkano::image::ImageUsage>,
    readers: RefCell<Vec<&'rb PassDesc<'rb>>>,
    writers: RefCell<Vec<&'rb PassDesc<'rb>>>
}

pub struct PassDesc<'rb> {
    name: &'rb str,
    input_attachments: RefCell<Vec<&'rb AttachmentDesc<'rb>>>,
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
    // Write only color output
    pub fn add_color_output(&'rb self, attachment: &'rb AttachmentDesc<'rb>) {
        self.color_outputs.borrow_mut().push(attachment);
        self.add_writer(attachment);
        attachment.usage.set(vulkano::image::ImageUsage {
            color_attachment: true,
            ..attachment.usage.get()
        });
    }

    #[inline]
    pub fn set_depth_output(&'rb self, attachment: &'rb AttachmentDesc<'rb>) {
        self.depth_output.borrow_mut().replace(attachment);
        self.add_writer(attachment);
        attachment.usage.set(vulkano::image::ImageUsage {
            depth_stencil_attachment: true,
            ..attachment.usage.get()
        });
    }

    #[inline]
    // Read only input attachment
    pub fn add_input_attachment(&'rb self, attachment: &'rb AttachmentDesc<'rb>) {
        self.input_attachments.borrow_mut().push(attachment);
        self.add_reader(attachment);
        attachment.usage.set(vulkano::image::ImageUsage {
            input_attachment: true,
            ..attachment.usage.get()
        });
    }

    #[inline]
    pub fn set_depth_input(&'rb self, attachment: &'rb AttachmentDesc<'rb>) {
        self.depth_input.borrow_mut().replace(attachment);
        self.add_reader(attachment);
        attachment.usage.set(vulkano::image::ImageUsage {
            depth_stencil_attachment: true,
            ..attachment.usage.get()
        });
    }
}

#[derive(Clone)]
struct PassNodeDependency<'a, 'rb> {
    pass_node: &'a PassNode<'a, 'rb>,
    attachment: &'rb AttachmentDesc<'rb>,
    usage: vk_sys::ImageUsageFlagBits,
    is_edge: Cell<bool> // JUST used for toposort
}

impl<'a, 'rb> PassNodeDependency<'a, 'rb> {
    pub fn requires_external_dep(&self) -> bool {
        return self.usage | vk_sys::IMAGE_USAGE_DEPTH_STENCIL_ATTACHMENT_BIT != 0
                || self.usage | vk_sys::IMAGE_USAGE_INPUT_ATTACHMENT_BIT != 0;
    }
}

#[derive(Clone)]
struct PassNode<'a, 'rb> {
    pass: &'rb PassDesc<'rb>,
    dependents: Vec<PassNodeDependency<'a, 'rb>>,
    dependencies: Vec<PassNodeDependency<'a, 'rb>>
}

impl<'a, 'rb> PassNode<'a, 'rb> {
    pub fn is_independent(&self) -> bool {
        return self.dependencies.iter().all(|x| !x.is_edge.get());
    }

    pub fn depends_on(&self, other: &'a PassNode<'a, 'rb>) -> bool {
        if eq(self, other) {
            return true;
        }
        for dependency in self.dependencies.iter() {
            if dependency.pass_node.depends_on(other) {
                return true;
            }
        }
        return false;
    }

    #[inline]
    pub fn display(&'a self) {
        println!("Pass: {}", self.pass.name);

        if !self.dependencies.is_empty() {
            println!("Dependent on:");
            for dependency in self.dependencies.iter() {
                println!("- Pass {}, via attachment {}", dependency.pass_node.pass.name, dependency.attachment.name);
            }
        }
    }
}

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

struct PhysicalPass<'a, 'rb> {
    subpasses: Vec<&'a PassNode<'a, 'rb>>,
    external_dependencies: Vec<PassNodeDependency<'a, 'rb>>
}

impl<'a, 'rb> PhysicalPass<'a, 'rb> {
    pub fn is_external_dep(&self, pass_node: &'a PassNode<'a, 'rb>) -> bool {
        return self.external_dependencies.iter().find(|x| eq(pass_node, x.pass_node)).is_some();
    }

    pub fn is_internal_dep(&self, pass_node: &'a PassNode<'a, 'rb>) -> bool {
        return self.subpasses.iter().find(|&&x| eq(pass_node, x)).is_some();
    }

    pub fn add_subpass(&mut self, pass_node: &'a PassNode<'a, 'rb>) {
        self.subpasses.push(pass_node);
        for dep in pass_node.dependencies.iter() {
            if dep.requires_external_dep() && !self.is_external_dep(dep.pass_node) {
                self.external_dependencies.push(PassNodeDependency {
                    is_edge: Cell::new(dep.is_edge.get()),
                    ..*dep
                });
            }
        }
    }
}

pub struct RendererBuilder<'rb> {
    attachment_arena: Arena<AttachmentDesc<'rb>>,
    attachments: RefCell<Vec<&'rb AttachmentDesc<'rb>>>,
    backbuffer_attachment: AttachmentDesc<'rb>,

    pass_arena: Arena<PassDesc<'rb>>,
    passes: RefCell<Vec<&'rb PassDesc<'rb>>>,
}

impl<'rb> RendererBuilder<'rb> {
    pub fn new() -> RendererBuilder<'rb> {
        return RendererBuilder {
            attachment_arena: Arena::new(),
            attachments: RefCell::new(Vec::new()),
            backbuffer_attachment: AttachmentDesc {
                name: BACKBUFFER_NAME,
                format: vulkano::format::Format::R8G8B8A8Unorm,
                samples: 1,
                usage: Cell::new(vulkano::image::ImageUsage::none()),
                readers: RefCell::new(Vec::new()),
                writers: RefCell::new(Vec::new())
            },
            pass_arena: Arena::new(),
            passes: RefCell::new(Vec::new())
        };
    }

    pub fn add_attachment(&'rb self, name: &'static str, format: vulkano::format::Format, samples: usize) -> &'rb AttachmentDesc {
        let attachment = self.attachment_arena.alloc(AttachmentDesc {
            name,
            format,
            samples,
            usage: Cell::new(vulkano::image::ImageUsage::none()),
            readers: RefCell::new(Vec::new()),
            writers: RefCell::new(Vec::new())
        });

        self.attachments.borrow_mut().push(attachment);

        return attachment;
    }

    pub fn add_depth_attachment(&'rb self, name: &'static str, samples: usize) -> &'rb AttachmentDesc {
        return self.add_attachment(name, vulkano::format::Format::D24Unorm_S8Uint, samples);
    }

    pub fn add_pass(&'rb self, name: &'static str) -> &'rb PassDesc<'rb> {
        let pass = self.pass_arena.alloc(PassDesc {
            name,
            input_attachments: RefCell::new(Vec::new()),
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

    fn validate_passes<'a, I>(passes: I) -> Result<(), &'static str> 
        where 
            I: Iterator<Item = &'a&'rb PassDesc<'rb>>,
            'rb: 'a 
    {
        fn is_valid_depth_attachment<'rb>(attachment: Option<&'rb AttachmentDesc<'rb>>) -> bool {
            match attachment {
                Some(AttachmentDesc {format: vulkano::format::Format::D16Unorm, ..}) |
                Some(AttachmentDesc {format: vulkano::format::Format::D16Unorm_S8Uint, ..}) |
                Some(AttachmentDesc {format: vulkano::format::Format::D24Unorm_S8Uint, ..}) |
                Some(AttachmentDesc {format: vulkano::format::Format::D32Sfloat, ..}) |
                Some(AttachmentDesc {format: vulkano::format::Format::D32Sfloat_S8Uint, ..}) |
                None
                    => true,
                _ => false 
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

    fn create_pass_nodes<'a, I>(passes: I, arena: &'a Arena<PassNode<'a, 'rb>>) -> Result<Vec<&'a mut PassNode<'a, 'rb>>, &'static str>
        where
            I: Iterator<Item = &'a&'rb PassDesc<'rb>>
    {
        let mut pass_nodes: HashMap<&str, RefCell<&'a mut PassNode<'a, 'rb>>> = HashMap::new();

        // Create new node objects
        for pass in passes {
            if pass_nodes.contains_key(pass.name) {
                return Err("Pass name collision");
            }
            pass_nodes.insert(pass.name, RefCell::new(
                arena.alloc(
                    PassNode {
                        pass,
                        dependents: Vec::new(),
                        dependencies: Vec::new(),
                    }
                )
            ));
        }

        // Fill in inter-node dependencies
        for (_, pass_node_cell) in pass_nodes.iter() {
            let pass_node = pass_node_cell.borrow_mut();
            let pass = pass_node.pass;
            for input_attachment in pass.input_attachments.borrow().iter() {
                for writer in input_attachment.writers.borrow().iter() {
                    pass_node.dependencies.push(
                        PassNodeDependency {
                            pass_node: pass_nodes.get(writer.name).unwrap().get_mut(),
                            attachment: input_attachment,
                            is_edge: Cell::new(true),
                            usage: vk_sys::IMAGE_USAGE_INPUT_ATTACHMENT_BIT
                        }
                    );
                }
            }

            if pass.depth_input.borrow().is_some() {
                let depth_input = pass.depth_input.borrow().unwrap();
                for writer in depth_input.writers.borrow().iter() {
                    pass_node.dependencies.push(
                        PassNodeDependency {
                            pass_node: pass_nodes.get(writer.name).unwrap().get_mut(),
                            attachment: depth_input,
                            is_edge: Cell::new(true),
                            usage: vk_sys::IMAGE_USAGE_DEPTH_STENCIL_ATTACHMENT_BIT
                        }
                    )
                }
            }

            for color_output in pass.color_outputs.borrow().iter() {
                for reader in color_output.readers.borrow().iter() {
                    pass_node.dependents.push(
                        PassNodeDependency {
                            pass_node: pass_nodes.get(reader.name).unwrap().get_mut(),
                            attachment: color_output,
                            is_edge: Cell::new(true),
                            usage: vk_sys::IMAGE_USAGE_COLOR_ATTACHMENT_BIT 
                        }
                    );
                }
            }

            if pass.depth_output.borrow().is_some() {
                let depth_output = pass.depth_output.borrow().unwrap();
                for reader in depth_output.readers.borrow().iter() {
                    pass_node.dependents.push(
                        PassNodeDependency {
                            pass_node: pass_nodes.get(reader.name).unwrap().get_mut(),
                            attachment: depth_output,
                            is_edge: Cell::new(true),
                            usage: vk_sys::IMAGE_USAGE_DEPTH_STENCIL_ATTACHMENT_BIT 
                        }
                    )
                }
            }
        }
        return Ok(pass_nodes.into_iter().map(|(_, v)| v.get_mut()).collect();
    }

    fn schedule_passes<'a, I>(pass_nodes: I) -> Result<Vec<&'a PassNode<'a, 'rb>>, &'static str>
        where
            I: Iterator<Item = &'a&'a mut PassNode<'a, 'rb>>,
    {
        let mut root_nodes: BinaryHeap<RootNode<'a, 'rb>> = BinaryHeap::new();

        for pass_node in pass_nodes {
            if pass_node.is_independent() {
                root_nodes.push(RootNode {
                    node: pass_node,
                    overlap_score: usize::MAX
                });
            }
        }

        let mut sorted_passes: Vec<&'a PassNode<'a, 'rb>> = Vec::new();

        let mut visited: HashSet<&str> = HashSet::new();
        while !root_nodes.is_empty() {
            // Schedule
            let current_pass = root_nodes.pop().unwrap().node;

            sorted_passes.push(current_pass);

            // Remove edge for color attachments if it exists
            for dependent in current_pass.dependents.iter() {
                if !dependent.is_edge.get() {
                    continue;
                }

                dependent.is_edge.replace(false);

                // Remove current pass from dependent's dependencies
                for dependency in dependent.pass_node.dependencies.iter() {
                    if eq(dependency.pass_node, current_pass) {
                        dependency.is_edge.replace(false);
                    }
                }

                // If dependent no longer has dependencies, add to root nodes queue
                if dependent.pass_node.is_independent() && !visited.contains(dependent.pass_node.pass.name) {
                    // Calculate overlap score
                    let overlap_score: usize = sorted_passes.iter().fold(0,
                        |s, sorted| {
                            if !dependent.pass_node.depends_on(sorted) {
                                return s + 1;
                            }
                            return s;
                        }
                    );
                    // NOTE: We DON'T have to include the items already in the priority queue, because it's
                    // guaranteed that all items in this queue are independent.
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
        for pass_node in sorted_passes.iter() {
            if !pass_node.is_independent() {
                return Err("Cyclical render graph provided");
            }
        }

        Ok(sorted_passes)
    }

    pub fn build(&'rb self) -> Result<Renderer, &'static str> {
        // Validate
        // TODO: handle this error properly
        let passes = self.passes.borrow();

        println!("Validating passes...");
        RendererBuilder::validate_passes(passes.iter())?;

        // Schedule passes
        println!("Scheduling passes...");
        let pass_node_arena: Arena<PassNode<'_, 'rb>> = Arena::new();
        let pass_nodes = RendererBuilder::create_pass_nodes(passes.iter(), &pass_node_arena).unwrap();
        let scheduled_passes = RendererBuilder::schedule_passes(pass_nodes.iter())?;

        println!("Pass scheduling complete. Result:\n");
        for (i, pass_node ) in scheduled_passes.iter().enumerate() {
            print!("index {}: ", i);
            pass_node.display();
        }

        // Create vulkan resources

        println!("\nCreating vulkan resources\n");

        // Merge passes based on a set of criteria

        // If depth attachments are different, needs to be a different pass
        // If any input, depth, color, or resolve attachment is the same as the previous pass, merge
        // Else, don't merge.

        let mut physical_passes: Vec<PhysicalPass> = Vec::new();
        for pass in scheduled_passes.iter() {
            fn merge_score<'a, 'rb>(pass: &'a PassNode<'a, 'rb>, physical_pass: &PhysicalPass<'a, 'rb>) -> usize {
                // Calculate the merge score of a given pass and a physical pass
                return pass.dependencies.iter()
                    .fold(0, |score, dep| {
                        if !dep.requires_external_dep() && physical_pass.is_internal_dep(dep.pass_node) {
                            // If this is an internal dependency, increase score if it exists as a subpass in the physical pass
                            return score + 1;
                        }
                        if dep.requires_external_dep() && physical_pass.is_external_dep(dep.pass_node) {
                            // If this is an external dependency, increase score if it exists as an external dep in the physical pass
                            return score + 1;
                        } 
                        return score;
                    });
            }
            // Try to find a physical pass which we can merge into 
            // This can only be done if all of the dependencies are met by the physical pass or its external dependencies
            let mut merge_physical_pass = physical_passes.iter_mut()
                .filter(
                    |physical_pass| {
                        // Find suitable physical passes for merging
                        for dep in pass.dependencies.iter() {
                            let in_subpasses = physical_pass.is_internal_dep(dep.pass_node);
                            if dep.requires_external_dep() {
                                // If this is an external dependency, do not merge if it's satisfied as a subpass in the physical pass
                                if in_subpasses { 
                                    return false;
                                }
                                // If this is an external dependency and it depends on any of the subpasses in the physical pass, do not merge
                                if physical_pass.subpasses.iter().any(|subpass| dep.pass_node.depends_on(subpass)) {
                                    return false;
                                }
                            } 
                            if !dep.requires_external_dep() {
                                // If this is an internal dependency, do not merge if it's not satisfied as a subpass dep
                                if !in_subpasses {
                                    return false;
                                }
                            }
                        }
                        return true;
                    }
                ).max_by(|physical_pass_a, physical_pass_b| {
                    // Find the physical pass with the highest merge score
                    return merge_score(pass, physical_pass_a).cmp(&merge_score(pass, physical_pass_b));
                });
            
            match merge_physical_pass {
                // If a mergeable physical pass exists, merge into it
                Some(x) => {
                    x.add_subpass(pass);
                },
                // If no physical pass can be merged into, create a new one
                None => continue
            }
        }

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
                    color_outputs: [$($color_output_atch:ident),*], // Write only color output
                    depth_stencil_output: {$($depth_output_atch:ident)?},
                    input_attachments: [$($input_attachment_atch:ident),*]$(,)* // Read only color input
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
                            $gfx_pass_name.add_input_attachment($input_attachment_atch);
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
