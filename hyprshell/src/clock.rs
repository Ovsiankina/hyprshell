//! The clock as an iced `shader` widget. The whole face is one full-screen
//! triangle evaluated in `clock.wgsl`; the Rust side only fills a uniform
//! buffer. One pipeline is created per renderer and reused for every frame.

use hyprshell_core::view::{ClockView, HandView};
use iced::wgpu;
use iced::widget::shader::{self, Viewport};
use iced::{Rectangle, mouse};

/// What `view()` hands to the widget: the view-model for one surface.
#[derive(Debug, Clone)]
pub struct ClockProgram {
    view: ClockView,
}

impl ClockProgram {
    pub fn new(view: ClockView) -> Self {
        Self { view }
    }
}

impl<Message> shader::Program<Message> for ClockProgram {
    type State = ();
    type Primitive = ClockPrimitive;

    fn draw(&self, _state: &(), _cursor: mouse::Cursor, _bounds: Rectangle) -> ClockPrimitive {
        ClockPrimitive { view: self.view.clone() }
    }
    // `update` keeps its default (`None`): the widget never asks for a redraw
    // by itself. Every frame comes from a message through the redraw scope.
}

#[derive(Debug)]
pub struct ClockPrimitive {
    view: ClockView,
}

/// Must match `struct Uniforms` in `clock.wgsl`: ten `vec4<f32>`.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    rect: [f32; 4],
    face_color: [f32; 4],
    border_color: [f32; 4],
    hour_color: [f32; 4],
    minute_color: [f32; 4],
    second_color: [f32; 4],
    hour: [f32; 4],
    minute: [f32; 4],
    second: [f32; 4],
    params: [f32; 4],
}

impl Uniforms {
    /// `rect` is the widget's bounds in logical pixels; `scale` converts to the
    /// physical pixels the fragment shader sees.
    fn new(view: &ClockView, rect: &Rectangle, scale: f32) -> Self {
        let hand = |h: &HandView| [h.angle, h.length, h.width * scale * 0.5, 1.0];
        let (second, second_color) = match &view.second {
            Some(h) => (hand(h), h.color.premultiplied()),
            None => ([0.0; 4], [0.0; 4]),
        };
        Self {
            rect: [rect.x * scale, rect.y * scale, rect.width * scale, rect.height * scale],
            face_color: view.face.color.premultiplied(),
            border_color: view.face.border_color.premultiplied(),
            hour_color: view.hour.color.premultiplied(),
            minute_color: view.minute.color.premultiplied(),
            second_color,
            hour: hand(&view.hour),
            minute: hand(&view.minute),
            second,
            params: [view.face.border_width * scale, view.face.rounding * scale, 0.0, 0.0],
        }
    }
}

pub struct ClockPipeline {
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

impl shader::Pipeline for ClockPipeline {
    fn new(device: &wgpu::Device, _queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        use wgpu::util::DeviceExt;

        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("hyprshell.clock.shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/clock.wgsl").into()),
        });
        let uniforms = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("hyprshell.clock.uniforms"),
            contents: bytemuck::bytes_of(&<Uniforms as bytemuck::Zeroable>::zeroed()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("hyprshell.clock.bind_group_layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("hyprshell.clock.bind_group"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniforms.as_entire_binding() }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("hyprshell.clock.pipeline_layout"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("hyprshell.clock.pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    // Whatever surface format iced chose; never hardcode it.
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        Self { pipeline, uniforms, bind_group }
    }
}

impl shader::Primitive for ClockPrimitive {
    type Pipeline = ClockPipeline;

    fn prepare(
        &self,
        pipeline: &mut ClockPipeline,
        _device: &wgpu::Device,
        queue: &wgpu::Queue,
        bounds: &Rectangle,
        viewport: &Viewport,
    ) {
        let uniforms = Uniforms::new(&self.view, bounds, viewport.scale_factor());
        queue.write_buffer(&pipeline.uniforms, 0, bytemuck::bytes_of(&uniforms));
        // One line per GPU submission; count them to measure idle cost.
        tracing::debug!(target: "hyprshell::gpu", "prepare {}x{} @{}", bounds.width, bounds.height, viewport.scale_factor());
    }

    /// Fast path: iced has already set the viewport to the widget's physical
    /// bounds and the scissor to its clip bounds on its own render pass.
    fn draw(&self, pipeline: &ClockPipeline, pass: &mut wgpu::RenderPass<'_>) -> bool {
        pass.set_pipeline(&pipeline.pipeline);
        pass.set_bind_group(0, &pipeline.bind_group, &[]);
        pass.draw(0..3, 0..1);
        true
    }
}
