use typed_arena::Arena;
use vulkano::format::Format;
use std::cell::RefCell;

use std::collections::VecDeque;
use std::iter::FromIterator;

use std::ptr::eq;

pub const BACKBUFFER_NAME: &str = "BACKBUFFER";

pub struct AttachmentDesc<'rb> {
    name: &'static str,
    format: Format,
    samples: usize,
    readers: RefCell<Vec<&'rb PassDesc<'rb>>>,
    writers: RefCell<Vec<&'rb PassDesc<'rb>>>
}

pub struct PassDesc<'rb> {
    name: &'static str,
    color_inputs: RefCell<Vec<&'rb AttachmentDesc<'rb>>>,
    color_outputs: RefCell<Vec<&'rb AttachmentDesc<'rb>>>,
    depth_input: RefCell<Option<&'rb AttachmentDesc<'rb>>>,
    depth_output: RefCell<Option<&'rb AttachmentDesc<'rb>>>,
}

impl<'rb> PassDesc<'rb> {
    pub fn add_color_output(&'rb self, attachment: &'rb AttachmentDesc<'rb>) {
        self.color_outputs.borrow_mut().push(attachment);
        attachment.writers.borrow_mut().push(self);
    }

    pub fn set_depth_output(&'rb self, attachment: &'rb AttachmentDesc<'rb>) {
        self.depth_output.borrow_mut().replace(attachment);
        attachment.writers.borrow_mut().push(self);
    }

    pub fn add_color_input(&'rb self, attachment: &'rb AttachmentDesc<'rb>) {
        self.color_inputs.borrow_mut().push(attachment);
        attachment.readers.borrow_mut().push(self);
    }

    pub fn set_depth_input(&'rb self, attachment: &'rb AttachmentDesc<'rb>) {
        self.depth_input.borrow_mut().replace(attachment);
        attachment.readers.borrow_mut().push(self);
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
            depth_output: RefCell::new(None),
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

        // Toposort passes
        let mut sorted_passes: Vec<(&'rb PassDesc<'rb>, usize)> = Vec::new();
        let mut no_incoming: VecDeque<(&'rb PassDesc<'rb>, usize)> = VecDeque::new();

        for pass in self.passes.borrow().iter() {
            if pass.color_inputs.borrow().is_empty() && pass.depth_input.borrow().is_none() {
                no_incoming.push_back((pass, 0));
            }
        }

        let remove_edge = |attachment: &'rb AttachmentDesc<'rb>, current_pass: &'rb PassDesc<'rb>, current_depth: usize| {
            let mut new_no_incoming: Vec<(&'rb PassDesc<'rb>, usize)> = Vec::new();
            // For each output attachment, remove pass from writers of attachments
            attachment.writers.borrow_mut().retain(|x| !eq(*x, current_pass));
            // If the pass was the last writer to this attachment, remove the attachment from inputs of all of its readers
            if attachment.writers.borrow().is_empty() {
                for reading_pass in attachment.readers.borrow().iter() {
                    // Remove attachment from color inputs
                    reading_pass.color_inputs.borrow_mut().retain(|x| !eq(*x,attachment));
                    // Remove attachment from depth inputs
                    if reading_pass.depth_input.borrow().is_some() && eq(reading_pass.depth_input.borrow().unwrap(), attachment) {
                        reading_pass.depth_input.replace(None);
                    }
                    // If the reading pass has no more inputs, then add to no_incoming queue at new depth
                    if reading_pass.color_inputs.borrow().is_empty() && reading_pass.depth_input.borrow().is_none() {
                        new_no_incoming.push((reading_pass, current_depth + 1));
                    }
                }
            }
            return new_no_incoming;
        };

        while !no_incoming.is_empty() {
            let current_pass = no_incoming.pop_front().unwrap();
            sorted_passes.push(current_pass);

            // Remove edge for color attachments if it exists
            for color_output in current_pass.0.color_outputs.borrow().iter() {
                no_incoming.append(&mut VecDeque::from_iter(remove_edge(color_output, current_pass.0, current_pass.1)));
            }

            // Remove edge for depth attachments if it exists
            match *current_pass.0.depth_output.borrow() {
                Some(x) => {
                    no_incoming.append(&mut VecDeque::from_iter(remove_edge(x, current_pass.0, current_pass.1)));
                },
                None => ()
            }
        }

        for pass in self.passes.borrow().iter() {
            if !pass.color_inputs.borrow().is_empty() || pass.depth_input.borrow().is_some() {
                return Err("Cyclical render graph provided");
            }
        }

        for pass in sorted_passes.iter() {
            println!("Pass name: {}, sort order: {}", pass.0.name, pass.1);
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
macro_rules! render_config {
    {
        name: $render_config_name:ident,
        attachments: {
            $(
                $atch_name:ident: {
                    format: $format:expr,
                    samples: $samples:expr
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

                let backbuffer = builder.get_backbuffer_attachment();
                $(
                    let $atch_name = builder.add_attachment(std::stringify!($atch_name), $format, $samples);
                )*

                $(
                    // Create pass
                    let $gfx_pass_name = builder.add_pass(std::stringify!($gfx_pass_name));

                    // Add outputs
                    $(
                        $gfx_pass_name.add_color_output($color_output_atch);
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
                )*

                return builder.build();
            }
        }
    )
}
