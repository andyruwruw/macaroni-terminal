//! The glass theme's shaders (`backdrop.wgsl`): the window background, the same background
//! blurred under each frosted sheet (computed exactly, see there), and the sheets' rims. iced
//! can't read back what's already drawn, so this blurs only the backdrop, not other UI behind a
//! sheet.
//!
//! Three constructors make the shader widgets: [`window`] (the root `stack`'s bottom layer, in
//! `view.rs`), [`behind`] (under each frosted sheet) and [`rim`] (over each sheet), the last two
//! used by [`crate::glass`]. Each is an iced `shader` widget filling its bounds, whose
//! [`Primitive`] draws one full-screen triangle that iced clips to those bounds.
//!
//! Two uniform buffers, one bind group:
//! - binding 0, the per-primitive `Sheet` (bounds, color, mode, blur...): each primitive takes a
//!   slot in one buffer during `prepare` (all of a frame's prepares run before any draw) and
//!   binds it by dynamic offset; `trim` frees the slots after the frame.
//! - binding 1, the shared [`Scene`] (`Chrome.backdrop`): written once per frame, by the first
//!   backdrop primitive's `prepare`, since every primitive in a frame carries the same colors.
//!
//! Trap: a shader primitive draws after every quad and mesh in its layer, so a backdrop must sit
//! in a layer of its own under what it backs (the `stack` base, or a sheet's `push_under`).

use std::sync::atomic::{AtomicU32, Ordering};

use iced::wgpu;
use iced::widget::shader::{self, Viewport};
use iced::{Color, Element, Fill, Rectangle, mouse};

/// More sheets than this in one frame draw without their backdrop or rim.
const SLOTS: u32 = 1024;
/// The uniform's floats, padded to WGSL's 16-byte struct alignment.
const FLOATS: usize = 20;
/// The backdrop's colors: its gradient's ends, then its glows.
pub type Scene = [Color; 7];
/// The [`Scene`]'s floats: seven RGBA colors, as WGSL's `array<vec4<f32>, 7>`.
const SCENE_FLOATS: usize = 7 * 4;
/// Where the rim's highlights peak: this many degrees counterclockwise of 12 o'clock (top
/// edge) and 6 o'clock (bottom), as if the sheet were square, so it shows on wide tabs too.
const HIGHLIGHT_ANGLE: f32 = 35.0;

/// What a primitive draws.
#[derive(Debug, Clone, Copy)]
enum Kind {
    /// The backdrop: the window's own (`blur` 0, with grain) or the part behind a sheet.
    Backdrop {
        /// The blur's standard deviation, logical pixels.
        blur: f32,
        /// 0 to 1: how much the sheet acts as a lens at its edges.
        refract: f32,
        /// The backdrop's colors.
        scene: Scene,
    },
    /// A sheet's rim.
    Rim {
        /// The faint edge all round (straight alpha).
        color: Color,
        /// The highlights' alpha at their peak.
        shine: f32,
    },
}

/// The whole background, for the bottom of the window.
pub fn window<'a, M: 'a>(scene: Scene) -> Element<'a, M> {
    layer(
        0.0,
        Kind::Backdrop {
            blur: 0.0,
            refract: 0.0,
            scene,
        },
    )
}

/// The background behind a sheet with corner `radius`, blurred by `blur` (a standard deviation,
/// in logical pixels). `refract`, 0 to 1, makes it a thicker lens: the backdrop bends and splits
/// toward the edges, which catch light at the top left and shade at the bottom right.
pub fn behind<'a, M: 'a>(radius: f32, blur: f32, refract: f32, scene: Scene) -> Element<'a, M> {
    layer(
        radius,
        Kind::Backdrop {
            blur,
            refract,
            scene,
        },
    )
}

/// A sheet's rim: `color` all round, plus a highlight along the top edge and a dimmer one along
/// the bottom, each fading out toward the corners and adding up to `shine` alpha.
pub fn rim<'a, M: 'a>(radius: f32, color: Color, shine: f32) -> Element<'a, M> {
    layer(radius, Kind::Rim { color, shine })
}

/// A shader widget filling its parent, drawing `kind` in a rounded rectangle of `radius`.
fn layer<'a, M: 'a>(radius: f32, kind: Kind) -> Element<'a, M> {
    iced::widget::shader(Program { radius, kind })
        .width(Fill)
        .height(Fill)
        .into()
}

/// The shader widget's program: what to draw, handed to each frame's [`Primitive`].
struct Program {
    /// Corner radius, logical pixels.
    radius: f32,
    /// What to draw.
    kind: Kind,
}

impl<M> shader::Program<M> for Program {
    type State = ();
    type Primitive = Primitive;

    fn draw(&self, _: &(), _: mouse::Cursor, _: Rectangle) -> Primitive {
        Primitive {
            radius: self.radius,
            kind: self.kind,
            slot: AtomicU32::new(SLOTS),
        }
    }
}

/// One frame's draw of one shader widget.
#[derive(Debug)]
pub struct Primitive {
    /// Corner radius, logical pixels.
    radius: f32,
    /// What to draw.
    kind: Kind,
    /// The uniform slot `prepare` took, for `draw` (which only gets `&self`); [`SLOTS`] or more
    /// means none, and the primitive draws nothing.
    slot: AtomicU32,
}

impl shader::Primitive for Primitive {
    type Pipeline = Pipeline;

    fn prepare(
        &self,
        pipeline: &mut Pipeline,
        _: &wgpu::Device,
        queue: &wgpu::Queue,
        bounds: &Rectangle,
        viewport: &Viewport,
    ) {
        let slot = pipeline.next;
        pipeline.next += 1;
        self.slot.store(slot, Ordering::Relaxed);
        if slot >= SLOTS {
            return;
        }
        let window = viewport.logical_size();
        // Positions stay in logical pixels; the shader divides `frag.xy` by the scale factor.
        let (mode, blur, refract, color, shine) = match self.kind {
            Kind::Backdrop {
                blur,
                refract,
                scene,
            } => {
                if !pipeline.scene_written {
                    pipeline.scene_written = true;
                    let floats: Vec<f32> =
                        scene.iter().flat_map(|c| [c.r, c.g, c.b, c.a]).collect();
                    let bytes: Vec<u8> = floats.iter().flat_map(|f| f.to_ne_bytes()).collect();
                    queue.write_buffer(&pipeline.scene, 0, &bytes);
                }
                (0.0, blur, refract, Color::TRANSPARENT, 0.0)
            }
            Kind::Rim { color, shine } => (1.0, 0.0, 0.0, color, shine),
        };
        // The WGSL `Sheet` struct, field by field (two trailing floats pad it to 16 bytes).
        let floats: [f32; FLOATS] = [
            bounds.x,
            bounds.y,
            bounds.width,
            bounds.height,
            color.r,
            color.g,
            color.b,
            color.a,
            window.width,
            window.height,
            viewport.scale_factor(),
            self.radius,
            blur,
            if pipeline.srgb { 1.0 } else { 0.0 },
            mode,
            // The highlight's offset from the edge's middle, as a fraction of a square's side.
            0.5 * HIGHLIGHT_ANGLE.to_radians().tan(),
            shine,
            refract,
            0.0,
            0.0,
        ];
        let bytes: Vec<u8> = floats.iter().flat_map(|f| f.to_ne_bytes()).collect();
        queue.write_buffer(&pipeline.uniforms, slot as u64 * pipeline.stride, &bytes);
    }

    fn draw(&self, pipeline: &Pipeline, pass: &mut wgpu::RenderPass<'_>) -> bool {
        let slot = self.slot.load(Ordering::Relaxed);
        // No vertex buffer: the vertex stage makes a full-screen triangle from the indices, and
        // the render pass is already clipped to the widget's bounds.
        if slot < SLOTS {
            pass.set_pipeline(&pipeline.pipeline);
            pass.set_bind_group(0, &pipeline.bind_group, &[slot * pipeline.stride as u32]);
            pass.draw(0..3, 0..1);
        }
        true
    }
}

/// The GPU side, created once per renderer by iced and shared by every backdrop primitive.
pub struct Pipeline {
    /// The shader, alpha blended over what's drawn.
    pipeline: wgpu::RenderPipeline,
    /// [`SLOTS`] per-primitive `Sheet` uniforms, `stride` bytes apart (binding 0).
    uniforms: wgpu::Buffer,
    /// The shared [`Scene`] (binding 1).
    scene: wgpu::Buffer,
    /// This frame's backdrop colors are in `scene`.
    scene_written: bool,
    /// Both buffers; binding 0 at a dynamic offset.
    bind_group: wgpu::BindGroup,
    /// One slot's size, rounded up to the device's uniform offset alignment.
    stride: u64,
    /// The next free slot this frame.
    next: u32,
    /// Whether the target is sRGB, so the shader must output linear colors.
    srgb: bool,
}

impl shader::Pipeline for Pipeline {
    fn new(device: &wgpu::Device, _: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let size = (FLOATS * 4) as u64;
        let align = u64::from(device.limits().min_uniform_buffer_offset_alignment);
        let stride = size.div_ceil(align) * align;
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("backdrop uniforms"),
            size: stride * u64::from(SLOTS),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let scene_size = (SCENE_FLOATS * 4) as u64;
        let scene = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("backdrop scene"),
            size: scene_size,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("backdrop"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(size),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(scene_size),
                    },
                    count: None,
                },
            ],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("backdrop"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &uniforms,
                        offset: 0,
                        size: wgpu::BufferSize::new(size),
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: scene.as_entire_binding(),
                },
            ],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("backdrop"),
            source: wgpu::ShaderSource::Wgsl(include_str!("backdrop.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("backdrop"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("backdrop"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        Self {
            pipeline,
            uniforms,
            scene,
            scene_written: false,
            bind_group,
            stride,
            next: 0,
            srgb: format.is_srgb(),
        }
    }

    // Called once a frame has been drawn: every slot is free again, and the next frame's first
    // backdrop rewrites the scene (it changes on config reload).
    fn trim(&mut self) {
        self.next = 0;
        self.scene_written = false;
    }
}
