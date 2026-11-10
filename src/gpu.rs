//! GPU render backend (cargo feature `gpu`).
//!
//! The GPU runs a normal `wgpu` render pipeline: a vertex shader rotates and
//! projects triangles with exactly the CPU renderer's camera ([`Projection`]),
//! the depth buffer resolves visibility, and a fragment shader applies the same
//! point light, smooth/flat normals and texture lookup. The result is read back
//! into a [`GpuImage`] (brightness + colour per pixel) which the caller merges
//! into a [`crate::display::Canvas`], so everything after rasterisation
//! (ASCII / colour / half-block encoding) is shared with the CPU path.
//!
//! Without the feature, every entry point returns an error explaining that
//! GPU support was not compiled in, and callers fall back to the CPU.

use crate::image::Image;
use crate::mesh::Mesh;

/// What kind of adapter is available.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GpuInfo {
    pub name: String,
    /// e.g. "discrete GPU", "integrated GPU", "software (CPU)".
    pub kind: String,
    /// e.g. "Vulkan", "Metal".
    pub api: String,
}

impl GpuInfo {
    pub fn summary(&self) -> String {
        format!("{} ({}, {})", self.name, self.kind, self.api)
    }
}

/// One GPU draw: render the top `height` canvas rows of a `full_h`-row canvas.
#[derive(Clone, Copy, Debug)]
pub struct GpuParams {
    pub width: usize,
    /// Rows to render (counted from the top of the canvas).
    pub height: usize,
    /// Height of the whole canvas; the projection uses this so the GPU band
    /// lines up with a CPU band drawn into the same canvas.
    pub full_h: usize,
    pub pixels_per_row: usize,
    pub ax: f32,
    pub ay: f32,
    pub cam: f32,
    pub smooth: bool,
    pub two_sided: bool,
}

/// Result of a GPU draw. `lum < 0` marks an empty pixel, as in `Canvas`.
#[derive(Clone, Debug)]
pub struct GpuImage {
    pub w: usize,
    pub h: usize,
    pub lum: Vec<f32>,
    pub base: Vec<[f32; 3]>,
}

pub use imp::{probe, GpuRenderer};

#[cfg(not(feature = "gpu"))]
mod imp {
    use super::*;

    const MSG: &str = "this build has no GPU support (rebuild with `cargo build --release --features gpu`)";

    pub fn probe() -> Result<GpuInfo, String> {
        Err(MSG.into())
    }

    pub struct GpuRenderer {
        info: GpuInfo,
    }

    impl GpuRenderer {
        pub fn new() -> Result<GpuRenderer, String> {
            Err(MSG.into())
        }
        pub fn info(&self) -> &GpuInfo {
            &self.info
        }
        pub fn render(&mut self, _: &Mesh, _: Option<&Image>, _: &GpuParams) -> Result<GpuImage, String> {
            Err(MSG.into())
        }
    }
}

#[cfg(feature = "gpu")]
mod imp {
    use super::*;
    use crate::raster::{light_position, Projection};
    use crate::vec3::V3;
    use wgpu::util::DeviceExt;

    const SHADER: &str = include_str!("gpu.wgsl");
    const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;
    const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

    /// Drivers (notably Mesa without a display session) sometimes print to
    /// stderr while initialising; that would scribble over the terminal UI.
    #[cfg(unix)]
    fn quiet<T>(f: impl FnOnce() -> T) -> T {
        use std::io::Write;
        let _ = std::io::stderr().flush();
        // SAFETY: plain fd juggling; stderr is restored before returning.
        unsafe {
            let saved = libc::dup(2);
            let null = libc::open(b"/dev/null\0".as_ptr().cast(), libc::O_WRONLY);
            if saved < 0 || null < 0 {
                return f();
            }
            libc::dup2(null, 2);
            libc::close(null);
            let out = f();
            libc::dup2(saved, 2);
            libc::close(saved);
            out
        }
    }
    #[cfg(not(unix))]
    fn quiet<T>(f: impl FnOnce() -> T) -> T {
        f()
    }

    fn describe(a: &wgpu::Adapter) -> GpuInfo {
        let i = a.get_info();
        GpuInfo {
            name: i.name,
            kind: match i.device_type {
                wgpu::DeviceType::DiscreteGpu => "discrete GPU",
                wgpu::DeviceType::IntegratedGpu => "integrated GPU",
                wgpu::DeviceType::VirtualGpu => "virtual GPU",
                wgpu::DeviceType::Cpu => "software (CPU)",
                wgpu::DeviceType::Other => "GPU",
            }
            .into(),
            api: format!("{:?}", i.backend),
        }
    }

    fn adapter() -> Result<(wgpu::Instance, wgpu::Adapter), String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..Default::default()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
        }))
        .ok_or_else(|| "no GPU adapter found (no Vulkan / Metal / DX12 driver)".to_string())?;
        Ok((instance, adapter))
    }

    pub fn probe() -> Result<GpuInfo, String> {
        quiet(|| adapter().map(|(_i, a)| describe(&a)))
    }

    #[repr(C)]
    #[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
    struct Vertex {
        pos: [f32; 4], // xyz, w = 1 if this corner has texture coordinates
        n: [f32; 4],   // smooth normal
        nf: [f32; 4],  // flat (face) normal
        col: [f32; 4],
        uv: [f32; 4],
    }

    #[repr(C)]
    #[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
    struct Uniforms {
        rot: [[f32; 4]; 3], // columns of the rotation matrix
        dims: [f32; 4],     // target w, target h, cx, cy
        proj: [f32; 4],     // x_scale, focal, cam
        light: [f32; 4],
        opts: [f32; 4], // smooth, two_sided, use_tex
    }

    pub struct GpuRenderer {
        info: GpuInfo,
        device: wgpu::Device,
        queue: wgpu::Queue,
        layout: wgpu::BindGroupLayout,
        pipe_cull: wgpu::RenderPipeline,
        pipe_both: wgpu::RenderPipeline,
        uniforms: wgpu::Buffer,
        sampler: wgpu::Sampler,
        /// (mesh pointer, triangle count) the vertex buffer was built from.
        mesh_key: Option<(usize, usize)>,
        vertices: Option<(wgpu::Buffer, u32)>,
        /// (pixel pointer, w, h) of the uploaded texture, or None for the 1x1 dummy.
        tex_key: Option<(usize, usize, usize)>,
        bind: Option<wgpu::BindGroup>,
        target: Option<Target>,
    }

    struct Target {
        w: usize,
        h: usize,
        color: wgpu::Texture,
        depth: wgpu::Texture,
        readback: wgpu::Buffer,
        padded_row: usize,
    }

    impl GpuRenderer {
        pub fn new() -> Result<GpuRenderer, String> {
            quiet(Self::init)
        }

        fn init() -> Result<GpuRenderer, String> {
            let (_instance, adapter) = adapter()?;
            let info = describe(&adapter);
            let (device, queue) = pollster::block_on(adapter.request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("cubicle"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                },
                None,
            ))
            .map_err(|e| format!("cannot open the GPU: {e}"))?;

            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("cubicle"),
                source: wgpu::ShaderSource::Wgsl(SHADER.into()),
            });
            let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: None,
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });
            let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &[&layout],
                push_constant_ranges: &[],
            });
            let attrs = wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32x4, 4 => Float32x4];
            let make = |cull: Option<wgpu::Face>| {
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: None,
                    layout: Some(&pl),
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: "vs",
                        buffers: &[wgpu::VertexBufferLayout {
                            array_stride: std::mem::size_of::<Vertex>() as u64,
                            step_mode: wgpu::VertexStepMode::Vertex,
                            attributes: &attrs,
                        }],
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: "fs",
                        targets: &[Some(wgpu::ColorTargetState {
                            format: COLOR_FORMAT,
                            blend: None,
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                    }),
                    primitive: wgpu::PrimitiveState {
                        topology: wgpu::PrimitiveTopology::TriangleList,
                        front_face: wgpu::FrontFace::Ccw,
                        cull_mode: cull,
                        ..Default::default()
                    },
                    depth_stencil: Some(wgpu::DepthStencilState {
                        format: DEPTH_FORMAT,
                        depth_write_enabled: true,
                        depth_compare: wgpu::CompareFunction::Less,
                        stencil: wgpu::StencilState::default(),
                        bias: wgpu::DepthBiasState::default(),
                    }),
                    multisample: wgpu::MultisampleState::default(),
                    multiview: None,
                })
            };
            let pipe_cull = make(Some(wgpu::Face::Back));
            let pipe_both = make(None);
            let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("uniforms"),
                size: std::mem::size_of::<Uniforms>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
                address_mode_u: wgpu::AddressMode::Repeat,
                address_mode_v: wgpu::AddressMode::Repeat,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            });
            Ok(GpuRenderer {
                info,
                device,
                queue,
                layout,
                pipe_cull,
                pipe_both,
                uniforms,
                sampler,
                mesh_key: None,
                vertices: None,
                tex_key: Some((usize::MAX, 0, 0)), // forces the first bind-group build
                bind: None,
                target: None,
            })
        }

        pub fn info(&self) -> &GpuInfo {
            &self.info
        }

        fn upload_mesh(&mut self, mesh: &Mesh) {
            let key = (mesh.tris.as_ptr() as usize, mesh.tris.len());
            if self.mesh_key == Some(key) {
                return;
            }
            let mut verts = Vec::with_capacity(mesh.tris.len() * 3);
            for t in &mesh.tris {
                let flat = t.v[1].sub(t.v[0]).cross(t.v[2].sub(t.v[0])).normalize();
                for k in 0..3 {
                    let (uv, has) = match t.uv {
                        Some(uv) => (uv[k], 1.0),
                        None => ([0.0, 0.0], 0.0),
                    };
                    verts.push(Vertex {
                        pos: [t.v[k].x, t.v[k].y, t.v[k].z, has],
                        n: [t.n[k].x, t.n[k].y, t.n[k].z, 0.0],
                        nf: [flat.x, flat.y, flat.z, 0.0],
                        col: [t.color[0], t.color[1], t.color[2], 0.0],
                        uv: [uv[0], uv[1], 0.0, 0.0],
                    });
                }
            }
            let buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("mesh"),
                contents: bytemuck::cast_slice(&verts),
                usage: wgpu::BufferUsages::VERTEX,
            });
            self.vertices = Some((buf, verts.len() as u32));
            self.mesh_key = Some(key);
        }

        fn upload_texture(&mut self, tex: Option<&Image>) {
            let key = tex.map(|i| (i.rgb.as_ptr() as usize, i.w, i.h));
            if self.bind.is_some() && self.tex_key == key {
                return;
            }
            let (w, h, rgba): (u32, u32, Vec<u8>) = match tex {
                Some(i) => (
                    i.w as u32,
                    i.h as u32,
                    i.rgb.iter().flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
                ),
                None => (1, 1, vec![255; 4]),
            };
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("texture"),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                // Not sRGB: the CPU path uses the PNG's bytes as-is too.
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            self.queue.write_texture(
                texture.as_image_copy(),
                &rgba,
                wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * w),
                    rows_per_image: Some(h),
                },
                wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
            );
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            self.bind = Some(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.uniforms.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            }));
            self.tex_key = key;
        }

        fn ensure_target(&mut self, w: usize, h: usize) {
            if self.target.as_ref().is_some_and(|t| t.w == w && t.h == h) {
                return;
            }
            let size = wgpu::Extent3d {
                width: w as u32,
                height: h as u32,
                depth_or_array_layers: 1,
            };
            let mk = |format, usage| {
                self.device.create_texture(&wgpu::TextureDescriptor {
                    label: None,
                    size,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
            };
            let padded_row = (w * 16).div_ceil(256) * 256;
            self.target = Some(Target {
                w,
                h,
                color: mk(
                    COLOR_FORMAT,
                    wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                ),
                depth: mk(DEPTH_FORMAT, wgpu::TextureUsages::RENDER_ATTACHMENT),
                readback: self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("readback"),
                    size: (padded_row * h) as u64,
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                padded_row,
            });
        }

        pub fn render(
            &mut self,
            mesh: &Mesh,
            tex: Option<&Image>,
            p: &GpuParams,
        ) -> Result<GpuImage, String> {
            let (w, h) = (p.width, p.height);
            if w == 0 || h == 0 {
                return Ok(GpuImage {
                    w,
                    h,
                    lum: vec![],
                    base: vec![],
                });
            }
            self.upload_mesh(mesh);
            self.upload_texture(tex);
            self.ensure_target(w, h);

            let proj = Projection::new(p.width, p.full_h, p.pixels_per_row, p.cam);
            let rot = |v: V3| v.rot_y(-p.ay).rot_x(-p.ax);
            let cols = [
                rot(V3::new(1.0, 0.0, 0.0)),
                rot(V3::new(0.0, 1.0, 0.0)),
                rot(V3::new(0.0, 0.0, 1.0)),
            ];
            let l = light_position(p.cam);
            let u = Uniforms {
                rot: [
                    [cols[0].x, cols[0].y, cols[0].z, 0.0],
                    [cols[1].x, cols[1].y, cols[1].z, 0.0],
                    [cols[2].x, cols[2].y, cols[2].z, 0.0],
                ],
                dims: [w as f32, h as f32, proj.cx, proj.cy],
                proj: [proj.x_scale, proj.focal, p.cam, 0.0],
                light: [l.x, l.y, l.z, 0.0],
                opts: [
                    f32::from(u8::from(p.smooth)),
                    f32::from(u8::from(p.two_sided)),
                    f32::from(u8::from(tex.is_some())),
                    0.0,
                ],
            };
            self.queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&u));

            let target = self.target.as_ref().expect("target");
            let color_view = target.color.create_view(&wgpu::TextureViewDescriptor::default());
            let depth_view = target.depth.create_view(&wgpu::TextureViewDescriptor::default());
            let mut enc = self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
            {
                let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &color_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            // alpha = brightness; -1 marks "nothing drawn here".
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: 0.0,
                                g: 0.0,
                                b: 0.0,
                                a: -1.0,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &depth_view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Discard,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                if let (Some((buf, count)), Some(bind)) = (&self.vertices, &self.bind) {
                    pass.set_pipeline(if p.two_sided {
                        &self.pipe_both
                    } else {
                        &self.pipe_cull
                    });
                    pass.set_bind_group(0, bind, &[]);
                    pass.set_vertex_buffer(0, buf.slice(..));
                    pass.draw(0..*count, 0..1);
                }
            }
            enc.copy_texture_to_buffer(
                target.color.as_image_copy(),
                wgpu::ImageCopyBuffer {
                    buffer: &target.readback,
                    layout: wgpu::ImageDataLayout {
                        offset: 0,
                        bytes_per_row: Some(target.padded_row as u32),
                        rows_per_image: Some(h as u32),
                    },
                },
                wgpu::Extent3d {
                    width: w as u32,
                    height: h as u32,
                    depth_or_array_layers: 1,
                },
            );
            self.queue.submit(Some(enc.finish()));

            let slice = target.readback.slice(..);
            let (tx, rx) = std::sync::mpsc::channel();
            slice.map_async(wgpu::MapMode::Read, move |r| {
                let _ = tx.send(r);
            });
            self.device.poll(wgpu::Maintain::Wait);
            rx.recv()
                .map_err(|_| "GPU readback was cancelled".to_string())?
                .map_err(|e| format!("GPU readback failed: {e}"))?;
            let data = slice.get_mapped_range();
            let floats: &[f32] = bytemuck::cast_slice(&data);
            let stride = target.padded_row / 4;
            let mut lum = Vec::with_capacity(w * h);
            let mut base = Vec::with_capacity(w * h);
            for y in 0..h {
                for x in 0..w {
                    let i = y * stride + x * 4;
                    base.push([floats[i], floats[i + 1], floats[i + 2]]);
                    lum.push(floats[i + 3]);
                }
            }
            drop(data);
            target.readback.unmap();
            Ok(GpuImage { w, h, lum, base })
        }
    }
}

#[cfg(all(test, feature = "gpu"))]
mod parity_tests {
    use super::*;
    use crate::raster::{render_textured, Frame, RenderOpts};

    fn renderer() -> Option<GpuRenderer> {
        match GpuRenderer::new() {
            Ok(r) => Some(r),
            Err(e) => {
                eprintln!("skipping GPU test: {e}");
                None
            }
        }
    }

    /// (coverage mismatch, mean |brightness diff|, mean |colour diff|) between CPU and GPU frames.
    fn compare(
        gpu: &mut GpuRenderer,
        mesh: &Mesh,
        tex: Option<&Image>,
        p: GpuParams,
        smooth: bool,
    ) -> (f32, f32, f32) {
        let opts = RenderOpts {
            smooth,
            two_sided: p.two_sided,
            pixels_per_row: p.pixels_per_row,
            y_range: None,
        };
        let mut cpu = Frame::new(p.width, p.full_h);
        render_textured(mesh, p.ax, p.ay, p.cam, opts, tex, &mut cpu);
        let g = gpu.render(mesh, tex, &p).expect("gpu render");
        assert_eq!((g.w, g.h), (p.width, p.full_h));
        let (mut covered, mut mismatch, mut both) = (0usize, 0usize, 0usize);
        let (mut dl, mut dc) = (0.0f32, 0.0f32);
        for i in 0..g.lum.len() {
            let (c, v) = (cpu.canvas.lum[i] >= 0.0, g.lum[i] >= 0.0);
            covered += (c || v) as usize;
            if c != v {
                mismatch += 1;
            } else if c {
                both += 1;
                dl += (cpu.canvas.lum[i] - g.lum[i]).abs();
                for k in 0..3 {
                    dc += (cpu.canvas.base[i][k] - g.base[i][k]).abs() / 3.0;
                }
            }
        }
        assert!(covered > 50, "scene should not be empty");
        (
            mismatch as f32 / covered as f32,
            dl / both.max(1) as f32,
            dc / both.max(1) as f32,
        )
    }

    fn params(w: usize, h: usize, ppr: usize, ax: f32, ay: f32) -> GpuParams {
        GpuParams {
            width: w,
            height: h,
            full_h: h,
            pixels_per_row: ppr,
            ax,
            ay,
            cam: 5.0,
            smooth: true,
            two_sided: false,
        }
    }

    #[test]
    fn gpu_matches_cpu_on_cube_and_sphere() {
        let Some(mut gpu) = renderer() else { return };
        let sphere = crate::obj::load(std::path::Path::new("models/sphere.obj"), 60.0).unwrap();
        for (name, mesh) in [("cube", Mesh::cube()), ("sphere", sphere)] {
            for (w, h, ppr, ax, ay) in [
                (60, 24, 1, 0.4, 0.6),
                (80, 40, 2, -0.8, 2.5),
                (33, 17, 1, 0.0, 0.0),
            ] {
                for smooth in [true, false] {
                    let mut p = params(w, h, ppr, ax, ay);
                    p.smooth = smooth;
                    let (cov, lum, col) = compare(&mut gpu, &mesh, None, p, smooth);
                    eprintln!("{name} {w}x{h} smooth={smooth}: coverage {cov:.4} lum {lum:.4} col {col:.4}");
                    assert!(cov < 0.04, "{name}: coverage differs by {cov}");
                    assert!(lum < 0.03, "{name}: brightness differs by {lum}");
                    assert!(col < 0.02, "{name}: colour differs by {col}");
                }
            }
        }
    }

    #[test]
    fn gpu_matches_cpu_with_textures_and_two_sided() {
        let Some(mut gpu) = renderer() else { return };
        let tex = Image::load(std::path::Path::new("examples/checker.png")).unwrap();
        let (cov, lum, col) = compare(
            &mut gpu,
            &Mesh::cube(),
            Some(&tex),
            params(60, 24, 1, 0.5, 0.9),
            true,
        );
        eprintln!("textured cube: coverage {cov:.4} lum {lum:.4} col {col:.4}");
        assert!(cov < 0.04 && lum < 0.03);
        assert!(col < 0.08, "texture colours differ by {col}");

        // A lone triangle facing away is visible only when two-sided.
        let raw = vec![crate::mesh::RawTri::plain([
            crate::vec3::V3::new(-1.0, -1.0, 0.0),
            crate::vec3::V3::new(0.0, 1.0, 0.0),
            crate::vec3::V3::new(1.0, -1.0, 0.0),
        ])];
        let tri = Mesh::from_raw(raw, 60.0).unwrap();
        for two_sided in [false, true] {
            let mut p = params(40, 20, 1, 0.0, 0.0); // faces away from the camera
            p.two_sided = two_sided;
            let g = gpu.render(&tri, None, &p).unwrap();
            let shown = g.lum.iter().filter(|&&l| l >= 0.0).count();
            assert_eq!(shown > 0, two_sided, "two_sided={two_sided}, shown={shown}");
        }
    }

    #[test]
    fn gpu_band_lines_up_with_cpu_band() {
        // The GPU draws the top rows of a taller canvas: it must equal the same rows of a full render.
        let Some(mut gpu) = renderer() else { return };
        let mesh = Mesh::cube();
        let mut full = params(60, 24, 1, 0.4, 0.6);
        let whole = gpu.render(&mesh, None, &full).unwrap();
        full.height = 11;
        let top = gpu.render(&mesh, None, &full).unwrap();
        assert_eq!((top.w, top.h), (60, 11));
        for (i, (a, b)) in whole.lum[..60 * 11].iter().zip(&top.lum).enumerate() {
            assert_eq!(*a >= 0.0, *b >= 0.0, "coverage differs at pixel {i}");
            assert!(
                (a - b).abs() < 1e-3,
                "brightness differs at pixel {i}: {a} vs {b}"
            );
        }
    }

    #[test]
    fn probe_describes_the_adapter() {
        if let Ok(info) = probe() {
            assert!(!info.name.is_empty() && !info.api.is_empty());
        }
    }
}
