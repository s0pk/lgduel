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

/// The six faces of a box as (4 corners counter-clockwise seen from outside, outward
/// normal, brightness): +z, -z, +x, -x, +y, -y.
pub fn box_faces(b: &Aabb) -> [([[f32; 3]; 4], Vec3, f32); 6] {
    let (n, x) = (b.min, b.max);
    [
        (
            [
                [n.x, n.y, x.z],
                [x.x, n.y, x.z],
                [x.x, x.y, x.z],
                [n.x, x.y, x.z],
            ],
            Vec3::new(0.0, 0.0, 1.0),
            FACE_SHADE[0],
        ),
        (
            [
                [n.x, n.y, n.z],
                [n.x, x.y, n.z],
                [x.x, x.y, n.z],
                [x.x, n.y, n.z],
            ],
            Vec3::new(0.0, 0.0, -1.0),
            FACE_SHADE[1],
        ),
        (
            [
                [x.x, n.y, n.z],
                [x.x, x.y, n.z],
                [x.x, x.y, x.z],
                [x.x, n.y, x.z],
            ],
            Vec3::new(1.0, 0.0, 0.0),
            FACE_SHADE[2],
        ),
        (
            [
                [n.x, n.y, n.z],
                [n.x, n.y, x.z],
                [n.x, x.y, x.z],
                [n.x, x.y, n.z],
            ],
            Vec3::new(-1.0, 0.0, 0.0),
            FACE_SHADE[3],
        ),
        (
            [
                [n.x, x.y, n.z],
                [n.x, x.y, x.z],
                [x.x, x.y, x.z],
                [x.x, x.y, n.z],
            ],
            Vec3::new(0.0, 1.0, 0.0),
            FACE_SHADE[4],
        ),
        (
            [
                [n.x, n.y, n.z],
                [x.x, n.y, n.z],
                [x.x, n.y, x.z],
                [n.x, n.y, x.z],
            ],
            Vec3::new(0.0, -1.0, 0.0),
            FACE_SHADE[5],
        ),
    ]
}

/// Append the 12 triangles of a box, shaded per face.
pub fn push_box(out: &mut Vec<Vertex>, b: &Aabb, color: [f32; 3]) {
    for (q, _, s) in box_faces(b) {
        let col = [color[0] * s, color[1] * s, color[2] * s];
        for i in [0, 1, 2, 0, 2, 3] {
            out.push(Vertex {
                pos: q[i],
                color: col,
            });
        }
    }
}

/// A world vertex: position, texture coordinate, tint (face brightness included)
/// and the texture-array layer. Layer 0 is white, so the colour alone is shown.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug, PartialEq)]
pub struct WorldVertex {
    pub pos: [f32; 3],
    pub uv: [f32; 2],
    pub color: [f32; 3],
    pub layer: u32,
}

/// What a world face is, as far as a style can tell.
// Themes (next milestone) read the tag, normal and kind.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub struct Surface<'a> {
    /// The map entry's material tag.
    pub material: Option<&'a str>,
    /// Outward face normal.
    pub normal: Vec3,
    /// The colour the map gives this solid.
    pub map_color: [f32; 3],
    /// A ramp / convex solid (else a box).
    pub is_convex: bool,
}

/// How one face is drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Look {
    pub layer: u32,
    /// Tint before face brightness.
    pub color: [f32; 3],
    /// World units per texture repeat.
    pub scale: f32,
    /// Apply the per-face brightness (FACE_SHADE).
    pub shade: bool,
}

/// Decides how the world looks: which textures exist and how each face uses them.
pub trait WorldStyle {
    /// Changes whenever the look changes (the world is re-uploaded then).
    fn key(&self) -> u64;
    fn textures(&self) -> &crate::textures::TextureSet;
    fn look(&self, map: &str, s: Surface) -> Look;
}

/// The original flat look: map colours, per-face brightness, no texture.
pub struct ClassicStyle {
    textures: crate::textures::TextureSet,
}

impl ClassicStyle {
    pub fn new() -> Self {
        Self {
            textures: crate::textures::TextureSet::new(),
        }
    }
}

impl Default for ClassicStyle {
    fn default() -> Self {
        Self::new()
    }
}

impl WorldStyle for ClassicStyle {
    fn key(&self) -> u64 {
        0
    }
    fn textures(&self) -> &crate::textures::TextureSet {
        &self.textures
    }
    fn look(&self, _map: &str, s: Surface) -> Look {
        Look {
            layer: 0,
            color: s.map_color,
            scale: 128.0,
            shade: true,
        }
    }
}

/// Texture coordinate of `p` on a face with normal `n`: projected along the
/// dominant normal axis, `scale` world units per repeat.
pub fn planar_uv(p: Vec3, n: Vec3, scale: f32) -> [f32; 2] {
    let (ax, ay, az) = (n.x.abs(), n.y.abs(), n.z.abs());
    let (u, v) = if az >= ax && az >= ay {
        (p.x, p.y)
    } else if ax >= ay {
        (p.y, -p.z)
    } else {
        (p.x, -p.z)
    };
    let s = scale.max(1.0);
    [u / s, v / s]
}

/// The world as textured triangles, styled by `style`. With `ClassicStyle` the
/// positions, colours and order equal `world_mesh`.
pub fn world_mesh_textured(w: &World, style: &dyn WorldStyle) -> Vec<WorldVertex> {
    let mut out = Vec::with_capacity(w.brushes.len() * 36);
    let mut face = |poly: &[[f32; 3]],
                    normal: Vec3,
                    shade: f32,
                    material: Option<&str>,
                    map_color: [f32; 3],
                    is_convex: bool| {
        let look = style.look(
            &w.name,
            Surface {
                material,
                normal,
                map_color,
                is_convex,
            },
        );
        let k = if look.shade { shade } else { 1.0 };
        let color = [look.color[0] * k, look.color[1] * k, look.color[2] * k];
        let vert = |p: [f32; 3]| WorldVertex {
            pos: p,
            uv: planar_uv(Vec3::from_array(p), normal, look.scale),
            color,
            layer: look.layer,
        };
        for k in 1..poly.len() - 1 {
            for p in [poly[0], poly[k], poly[k + 1]] {
                out.push(vert(p));
            }
        }
    };
    for (i, (b, c)) in w.brushes.iter().zip(&w.colors).enumerate() {
        let material = w.materials.get(i).and_then(|m| m.as_deref());
        for (q, n, shade) in box_faces(b) {
            face(&q, n, shade, material, *c, false);
        }
    }
    for b in &w.convex {
        for (i, p) in b.planes.iter().enumerate() {
            let poly = b.face(i);
            if poly.len() < 3 {
                continue;
            }
            let pts: Vec<[f32; 3]> = poly.iter().map(|v| v.to_array()).collect();
            face(
                &pts,
                p.normal,
                shade_for(p.normal),
                b.material.as_deref(),
                b.color,
                true,
            );
        }
    }
    out
}

/// The original flat world mesh (kept as the reference the classic style must equal).
#[cfg(test)]
pub fn world_mesh(w: &World) -> Vec<Vertex> {
    let mut v = Vec::with_capacity(w.brushes.len() * 36);
    for (b, c) in w.brushes.iter().zip(&w.colors) {
        push_box(&mut v, b, *c);
    }
    for b in &w.convex {
        push_convex(&mut v, b);
    }
    v
}

/// Fullbright shade for an arbitrary face normal: blends the box face shades
/// (top 1.0, ±x 0.85, ±y 0.72, bottom 0.55) by the squared normal components.
pub fn shade_for(n: Vec3) -> f32 {
    let up = n.z.max(0.0);
    let down = (-n.z).max(0.0);
    up * up * FACE_SHADE[0]
        + down * down * FACE_SHADE[1]
        + n.x * n.x * FACE_SHADE[2]
        + n.y * n.y * FACE_SHADE[4]
}

/// Triangulate every real face of a convex brush (fan from the first vertex).
#[cfg(test)]
pub fn push_convex(out: &mut Vec<Vertex>, b: &lgduel_sim::brush::ConvexBrush) {
    for (i, p) in b.planes.iter().enumerate() {
        let poly = b.face(i);
        if poly.len() < 3 {
            continue;
        }
        let s = shade_for(p.normal);
        let color = [b.color[0] * s, b.color[1] * s, b.color[2] * s];
        for k in 1..poly.len() - 1 {
            for v in [poly[0], poly[k], poly[k + 1]] {
                out.push(Vertex {
                    pos: v.to_array(),
                    color,
                });
            }
        }
    }
}

/// Camera in Quake terms.
#[derive(Debug, Clone, Copy, PartialEq)]
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
    hud_pipeline: wgpu::RenderPipeline,
    hud_vb: wgpu::Buffer,
    hud_capacity: u64,
    hud_count: u32,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    world_pipeline: wgpu::RenderPipeline,
    tex_bgl: wgpu::BindGroupLayout,
    tex_bind: Option<wgpu::BindGroup>,
    world_vb: Option<(wgpu::Buffer, u32)>,
    /// (map content hash, style key) of the uploaded world.
    world_key: Option<(u64, u64)>,
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
@vertex fn vs_hud(@location(0) pos: vec3<f32>, @location(1) color: vec3<f32>) -> VOut {
    var o: VOut;
    o.pos = vec4<f32>(pos.xy, 0.0, 1.0);
    o.color = color;
    return o;
}
@fragment fn fs(i: VOut) -> @location(0) vec4<f32> { return vec4<f32>(i.color, 1.0); }
"#;

const WORLD_SHADER: &str = r#"
struct U { view_proj: mat4x4<f32> };
@group(0) @binding(0) var<uniform> u: U;
@group(1) @binding(0) var tex: texture_2d_array<f32>;
@group(1) @binding(1) var samp: sampler;
struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec3<f32>,
    @location(2) @interpolate(flat) layer: u32,
};
@vertex fn vs_world(
    @location(0) pos: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec3<f32>,
    @location(3) layer: u32,
) -> VOut {
    var o: VOut;
    o.pos = u.view_proj * vec4<f32>(pos, 1.0);
    o.uv = uv;
    o.color = color;
    o.layer = layer;
    return o;
}
@fragment fn fs_world(i: VOut) -> @location(0) vec4<f32> {
    let t = textureSample(tex, samp, i.uv, i.layer);
    return vec4<f32>(t.rgb * i.color, 1.0);
}
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
        let make = |entry: &str, write: bool, compare: wgpu::CompareFunction| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(entry),
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
                    depth_write_enabled: Some(write),
                    depth_compare: Some(compare),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let tex_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("world textures"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let world_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("world"),
            source: wgpu::ShaderSource::Wgsl(WORLD_SHADER.into()),
        });
        let world_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("world"),
            bind_group_layouts: &[Some(&bgl), Some(&tex_bgl)],
            ..Default::default()
        });
        let world_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("world"),
            layout: Some(&world_layout),
            vertex: wgpu::VertexState {
                module: &world_shader,
                entry_point: Some("vs_world"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<WorldVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x3, 1 => Float32x2, 2 => Float32x3, 3 => Uint32
                    ],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &world_shader,
                entry_point: Some("fs_world"),
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
        let pipeline = make("vs", true, wgpu::CompareFunction::Less);
        // HUD: screen-space, always on top, leaves depth untouched.
        let hud_pipeline = make("vs_hud", false, wgpu::CompareFunction::Always);
        let dyn_capacity = 4096 * std::mem::size_of::<Vertex>() as u64;
        let dyn_vb = Self::dyn_buffer(device, dyn_capacity);
        let hud_vb = Self::dyn_buffer(device, dyn_capacity);
        Self {
            pipeline,
            hud_pipeline,
            hud_vb,
            hud_capacity: dyn_capacity,
            hud_count: 0,
            uniforms,
            bind_group,
            world_pipeline,
            tex_bgl,
            tex_bind: None,
            world_vb: None,
            world_key: None,
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

    /// Upload the world mesh and textures (no-op if this map in this style is already uploaded).
    pub fn set_world(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        w: &World,
        style: &dyn WorldStyle,
    ) {
        let key = (w.content_hash(), style.key());
        if self.world_key == Some(key) {
            return;
        }
        self.world_key = Some(key);
        let verts = world_mesh_textured(w, style);
        let buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("world"),
            contents: bytemuck::cast_slice(&verts),
            usage: wgpu::BufferUsages::VERTEX,
        });
        self.world_vb = Some((buf, verts.len() as u32));
        let gpu = crate::textures::upload(style.textures(), device, queue);
        self.tex_bind = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("world textures"),
            layout: &self.tex_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&gpu.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&gpu.sampler),
                },
            ],
        }));
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

    /// Per-frame HUD geometry (positions in NDC).
    pub fn set_hud(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, verts: &[Vertex]) {
        let bytes = std::mem::size_of_val(verts) as u64;
        if bytes > self.hud_capacity {
            self.hud_capacity = bytes.next_power_of_two();
            self.hud_vb = Self::dyn_buffer(device, self.hud_capacity);
        }
        if !verts.is_empty() {
            queue.write_buffer(&self.hud_vb, 0, bytemuck::cast_slice(verts));
        }
        self.hud_count = verts.len() as u32;
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
            if let (Some((vb, n)), Some(tex)) = (&self.world_vb, &self.tex_bind) {
                pass.set_pipeline(&self.world_pipeline);
                pass.set_bind_group(0, &self.bind_group, &[]);
                pass.set_bind_group(1, tex, &[]);
                pass.set_vertex_buffer(0, vb.slice(..));
                pass.draw(0..*n, 0..1);
            }
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            if self.dyn_count > 0 {
                pass.set_vertex_buffer(0, self.dyn_vb.slice(..));
                pass.draw(0..self.dyn_count, 0..1);
            }
            if self.hud_count > 0 {
                pass.set_pipeline(&self.hud_pipeline);
                pass.set_vertex_buffer(0, self.hud_vb.slice(..));
                pass.draw(0..self.hud_count, 0..1);
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
    fn convex_ramp_mesh_and_shades() {
        use lgduel_sim::brush::{ConvexBrush, Rise};
        let ramp = ConvexBrush::ramp(
            Vec3::ZERO,
            Vec3::new(100.0, 50.0, 40.0),
            Rise::PosX,
            [1.0, 1.0, 1.0],
        )
        .unwrap();
        let mut v = vec![];
        push_convex(&mut v, &ramp);
        // 3 quads (2 triangles each) + 2 triangles = 8 triangles.
        assert_eq!(v.len(), 8 * 3);
        assert_eq!(shade_for(Vec3::new(0.0, 0.0, 1.0)), FACE_SHADE[0]);
        assert_eq!(shade_for(Vec3::new(0.0, 0.0, -1.0)), FACE_SHADE[1]);
        assert_eq!(shade_for(Vec3::new(1.0, 0.0, 0.0)), FACE_SHADE[2]);
        assert_eq!(shade_for(Vec3::new(0.0, -1.0, 0.0)), FACE_SHADE[4]);
        let slope = shade_for(ramp.planes[1].normal);
        assert!(slope > 0.85 && slope < 1.0, "{slope}");
    }

    #[test]
    fn classic_textured_world_equals_the_flat_mesh() {
        use lgduel_sim::config::PhysicsConfig;
        let p = PhysicsConfig::default();
        let hull = (
            Vec3::from_array(p.player_mins),
            Vec3::from_array(p.player_maxs),
        );
        for (name, _) in lgduel_sim::world::builtin::ALL {
            let w = lgduel_sim::world::builtin::load(name, hull).unwrap();
            let flat = world_mesh(&w);
            let tex = world_mesh_textured(&w, &ClassicStyle::new());
            assert_eq!(flat.len(), tex.len(), "{name}");
            for (a, b) in flat.iter().zip(&tex) {
                assert_eq!((a.pos, a.color), (b.pos, b.color), "{name}");
                assert_eq!(b.layer, 0);
            }
        }
    }

    #[test]
    fn planar_uv_projects_along_the_dominant_axis() {
        let p = Vec3::new(256.0, 128.0, 64.0);
        assert_eq!(planar_uv(p, Vec3::new(0.0, 0.0, 1.0), 128.0), [2.0, 1.0]);
        assert_eq!(planar_uv(p, Vec3::new(1.0, 0.0, 0.0), 128.0), [1.0, -0.5]);
        assert_eq!(planar_uv(p, Vec3::new(0.0, -1.0, 0.0), 128.0), [2.0, -0.5]);
        // A slope facing mostly up uses the floor projection.
        assert_eq!(planar_uv(p, Vec3::new(-0.4, 0.0, 0.9), 128.0), [2.0, 1.0]);
    }

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
