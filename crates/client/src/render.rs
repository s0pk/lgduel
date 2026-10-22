//! Flat-shaded, fullbright scene renderer: world boxes and player boxes as
//! solid-colour triangles with a fixed per-face brightness so geometry reads
//! clearly without lighting.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3 as GVec3, Vec4};
use lgduel_sim::math::{Angle, Vec3, angle_vectors};
use lgduel_sim::world::{Aabb, World};
use wgpu::util::DeviceExt;

pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug, PartialEq)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub color: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    view_proj: [[f32; 4]; 4],
}

/// Per-face brightness: top, bottom, ±x, ±y. Keeps edges readable while staying fullbright.
const FACE_SHADE: [f32; 6] = [1.0, 0.55, 0.85, 0.85, 0.72, 0.72];

/// Append the 12 triangles of a box, shaded per face.
pub fn push_box(out: &mut Vec<Vertex>, b: &Aabb, color: [f32; 3]) {
    let (n, x) = (b.min, b.max);
    let c = |s: f32| [color[0] * s, color[1] * s, color[2] * s];
    // Faces as 4 corners, counter-clockwise seen from outside.
    let faces: [([[f32; 3]; 4], f32); 6] = [
        (
            [
                [n.x, n.y, x.z],
                [x.x, n.y, x.z],
                [x.x, x.y, x.z],
                [n.x, x.y, x.z],
            ],
            FACE_SHADE[0],
        ), // +z
        (
            [
                [n.x, n.y, n.z],
                [n.x, x.y, n.z],
                [x.x, x.y, n.z],
                [x.x, n.y, n.z],
            ],
            FACE_SHADE[1],
        ), // -z
        (
            [
                [x.x, n.y, n.z],
                [x.x, x.y, n.z],
                [x.x, x.y, x.z],
                [x.x, n.y, x.z],
            ],
            FACE_SHADE[2],
        ), // +x
        (
            [
                [n.x, n.y, n.z],
                [n.x, n.y, x.z],
                [n.x, x.y, x.z],
                [n.x, x.y, n.z],
            ],
            FACE_SHADE[3],
        ), // -x
        (
            [
                [n.x, x.y, n.z],
                [n.x, x.y, x.z],
                [x.x, x.y, x.z],
                [x.x, x.y, n.z],
            ],
            FACE_SHADE[4],
        ), // +y
        (
            [
                [n.x, n.y, n.z],
                [x.x, n.y, n.z],
                [x.x, n.y, x.z],
                [n.x, n.y, x.z],
            ],
            FACE_SHADE[5],
        ), // -y
    ];
    for (q, s) in faces {
        let col = c(s);
        for i in [0, 1, 2, 0, 2, 3] {
            out.push(Vertex {
                pos: q[i],
                color: col,
            });
        }
    }
}

pub fn world_mesh(w: &World) -> Vec<Vertex> {
    let mut v = Vec::with_capacity(w.brushes.len() * 36);
    for (b, c) in w.brushes.iter().zip(&w.colors) {
        push_box(&mut v, b, *c);
    }
    v
}

/// Camera in Quake terms.
#[derive(Debug, Clone, Copy)]
pub struct Camera {
    pub eye: Vec3,
    pub pitch: Angle,
    pub yaw: Angle,
    /// Horizontal FOV in degrees, defined at 4:3 (Quake convention), clamped to 10..=130.
    pub fov: f32,
}

impl Camera {
    /// Vertical FOV (radians) from a Quake 4:3 horizontal FOV; wider screens see more (Hor+).
    pub fn fov_y(&self) -> f32 {
        let fx = self.fov.clamp(10.0, 130.0).to_radians();
        2.0 * ((fx / 2.0).tan() * 0.75).atan()
    }

    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        let (f, r, u) = angle_vectors(self.pitch, self.yaw);
        let g = |v: Vec3| GVec3::new(v.x, v.y, v.z);
        let (f, r, u, e) = (g(f), g(r), g(u), g(self.eye));
        // View space: +x right, +y up, -z forward.
        let view = Mat4::from_cols(
            Vec4::new(r.x, u.x, -f.x, 0.0),
            Vec4::new(r.y, u.y, -f.y, 0.0),
            Vec4::new(r.z, u.z, -f.z, 0.0),
            Vec4::new(-r.dot(e), -u.dot(e), f.dot(e), 1.0),
        );
        let proj = perspective_rh(self.fov_y(), aspect, 4.0, 8192.0);
        proj * view
    }
}

/// Right-handed perspective projection with wgpu's 0..1 depth range.
pub fn perspective_rh(fov_y: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
    let f = 1.0 / (fov_y / 2.0).tan();
    let r = far / (near - far);
    Mat4::from_cols(
        Vec4::new(f / aspect, 0.0, 0.0, 0.0),
        Vec4::new(0.0, f, 0.0, 0.0),
        Vec4::new(0.0, 0.0, r, -1.0),
        Vec4::new(0.0, 0.0, r * near, 0.0),
    )
}

pub struct Renderer {
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    world_vb: Option<(wgpu::Buffer, u32)>,
    dyn_vb: wgpu::Buffer,
    dyn_capacity: u64,
    dyn_count: u32,
}

const SHADER: &str = r#"
struct U { view_proj: mat4x4<f32> };
@group(0) @binding(0) var<uniform> u: U;
struct VOut { @builtin(position) pos: vec4<f32>, @location(0) color: vec3<f32> };
@vertex fn vs(@location(0) pos: vec3<f32>, @location(1) color: vec3<f32>) -> VOut {
    var o: VOut;
    o.pos = u.view_proj * vec4<f32>(pos, 1.0);
    o.color = color;
    return o;
}
@fragment fn fs(i: VOut) -> @location(0) vec4<f32> { return vec4<f32>(i.color, 1.0); }
"#;

impl Renderer {
    pub fn new(device: &wgpu::Device, color_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("flat"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("flat"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("flat"),
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("flat"),
            bind_group_layouts: &[Some(&bgl)],
            ..Default::default()
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("flat"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(color_format.into())],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let dyn_capacity = 4096 * std::mem::size_of::<Vertex>() as u64;
        let dyn_vb = Self::dyn_buffer(device, dyn_capacity);
        Self {
            pipeline,
            uniforms,
            bind_group,
            world_vb: None,
            dyn_vb,
            dyn_capacity,
            dyn_count: 0,
        }
    }

    fn dyn_buffer(device: &wgpu::Device, size: u64) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("dynamic"),
            size,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    pub fn set_world(&mut self, device: &wgpu::Device, w: &World) {
        let verts = world_mesh(w);
        let buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("world"),
            contents: bytemuck::cast_slice(&verts),
            usage: wgpu::BufferUsages::VERTEX,
        });
        self.world_vb = Some((buf, verts.len() as u32));
    }

    /// Per-frame geometry (players, beams).
    pub fn set_dynamic(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, verts: &[Vertex]) {
        let bytes = std::mem::size_of_val(verts) as u64;
        if bytes > self.dyn_capacity {
            self.dyn_capacity = bytes.next_power_of_two();
            self.dyn_vb = Self::dyn_buffer(device, self.dyn_capacity);
        }
        if !verts.is_empty() {
            queue.write_buffer(&self.dyn_vb, 0, bytemuck::cast_slice(verts));
        }
        self.dyn_count = verts.len() as u32;
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        color: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        size: (u32, u32),
        cam: &Camera,
        clear: wgpu::Color,
    ) {
        let vp = cam.view_proj(size.0 as f32 / size.1.max(1) as f32);
        queue.write_buffer(
            &self.uniforms,
            0,
            bytemuck::bytes_of(&Uniforms {
                view_proj: vp.to_cols_array_2d(),
            }),
        );
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("scene"),
        });
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: color,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            if let Some((vb, n)) = &self.world_vb {
                pass.set_vertex_buffer(0, vb.slice(..));
                pass.draw(0..*n, 0..1);
            }
            if self.dyn_count > 0 {
                pass.set_vertex_buffer(0, self.dyn_vb.slice(..));
                pass.draw(0..self.dyn_count, 0..1);
            }
        }
        queue.submit([enc.finish()]);
    }
}

pub fn depth_texture(device: &wgpu::Device, w: u32, h: u32) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("depth"),
            size: wgpu::Extent3d {
                width: w.max(1),
                height: h.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&Default::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn box_mesh_has_36_vertices_with_face_shades() {
        let mut v = vec![];
        push_box(
            &mut v,
            &Aabb::new(Vec3::ZERO, Vec3::new(1.0, 1.0, 1.0)),
            [1.0, 0.0, 0.0],
        );
        assert_eq!(v.len(), 36);
        assert_eq!(v[0].color, [1.0, 0.0, 0.0]); // top face full brightness
        assert_eq!(v[6].color, [0.55, 0.0, 0.0]); // bottom darker
    }

    #[test]
    fn camera_projects_forward_point_to_center() {
        let cam = Camera {
            eye: Vec3::new(10.0, 20.0, 30.0),
            pitch: Angle::ZERO,
            yaw: Angle::from_degrees(90.0),
            fov: 90.0,
        };
        let vp = cam.view_proj(16.0 / 9.0);
        // 100 units straight ahead (+y at yaw 90) lands at screen centre, in depth range.
        let p = vp * Vec4::new(10.0, 120.0, 30.0, 1.0);
        let ndc = p / p.w;
        assert!(ndc.x.abs() < 1e-5 && ndc.y.abs() < 1e-5, "{ndc:?}");
        assert!((0.0..1.0).contains(&ndc.z));
        // A point to the right (+x when facing +y in Quake axes) has positive screen x; above → positive y.
        let right = vp * Vec4::new(20.0, 120.0, 40.0, 1.0);
        assert!(right.x / right.w > 0.0 && right.y / right.w > 0.0);
        let left = vp * Vec4::new(0.0, 120.0, 30.0, 1.0);
        assert!(left.x / left.w < 0.0);
        // Behind the camera is clipped (w < 0).
        assert!((vp * Vec4::new(10.0, -80.0, 30.0, 1.0)).w < 0.0);
    }

    #[test]
    fn fov_is_quake_style_hor_plus() {
        let mut cam = Camera {
            eye: Vec3::ZERO,
            pitch: Angle::ZERO,
            yaw: Angle::ZERO,
            fov: 90.0,
        };
        // 90° at 4:3 ⇒ vertical ≈ 73.74°.
        assert!((cam.fov_y().to_degrees() - 73.7398).abs() < 1e-3);
        cam.fov = 200.0;
        let clamped = cam.fov_y();
        cam.fov = 130.0;
        assert_eq!(clamped, cam.fov_y());
    }
}
