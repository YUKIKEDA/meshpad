//! GPU 上のプロキシとチャンク。
//!
//! 頂点はファイル座標のまま載せる。AABB 中心は uniform の `origin` でシェーダが引く。
//! 全三角形を載せる。1 バッファが [`wgpu::Limits::max_buffer_size`] を超えるときは複数チャンク。
//! プロキシは空。

use std::num::NonZeroU64;

use eframe::egui_wgpu::{self, wgpu};
use glam::Vec3;

use crate::camera::Camera;
use crate::mesh::TriangleSoup;

const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const COLOR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    view_proj: [[f32; 4]; 4],
    light_dir: [f32; 3],
    _pad0: f32,
    color: [f32; 4],
    origin: [f32; 3],
    _pad1: f32,
}

/// GPU 常駐の頂点ブロック。
pub struct GpuChunk {
    vertex_buffer: wgpu::Buffer,
    vertex_count: u32,
}

/// 描画するシーン。
///
/// 遠景の [`Self::proxy`] と近景の [`Self::chunks`] を分けて持てる。
/// `proxy` は空。`chunks` は全三角形（バッファ上限で分割することがある）。
pub struct SceneGpu {
    /// 元ファイル空間での AABB 中心。シェーダが頂点から引く。
    pub origin: Vec3,
    /// ファイル空間 AABB の外接半径。カメラ距離の下限に使う。
    pub radius: f32,
    /// 遠景用の粗いメッシュ。無ければ `None`。
    pub proxy: Option<GpuChunk>,
    /// 全三角形。1 バッファに収まらなければ複数。
    pub chunks: Vec<GpuChunk>,
}

impl SceneGpu {
    /// CPU のスープをすべてアップロードする。
    ///
    /// プロキシは作らない。頂点はファイル座標のまま。
    /// 頂点バッファがデバイスの `max_buffer_size` を超えるときは三角形境界で分割する。
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let scene = SceneGpu::from_soup(&device, &soup);
    /// assert!(!scene.chunks.is_empty());
    /// ```
    pub fn from_soup(device: &wgpu::Device, soup: &TriangleSoup) -> Self {
        let cap = verts_per_chunk(device.limits().max_buffer_size);
        let chunks = if soup.positions.is_empty() {
            Vec::new()
        } else {
            soup.positions
                .chunks(cap)
                .map(|slice| upload_chunk(device, slice))
                .collect()
        };
        Self {
            origin: soup.origin,
            radius: soup.radius,
            proxy: None,
            chunks,
        }
    }

    /// 原点と半径だけ先に作り、チャンクはあとから足す。
    pub(crate) fn from_bounds(origin: Vec3, radius: f32) -> Self {
        Self {
            origin,
            radius,
            proxy: None,
            chunks: Vec::new(),
        }
    }

    pub(crate) fn push_chunk(&mut self, chunk: GpuChunk) {
        self.chunks.push(chunk);
    }

    fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        if let Some(p) = &self.proxy {
            pass.set_vertex_buffer(0, p.vertex_buffer.slice(..));
            pass.draw(0..p.vertex_count, 0..1);
        }
        for c in &self.chunks {
            pass.set_vertex_buffer(0, c.vertex_buffer.slice(..));
            pass.draw(0..c.vertex_count, 0..1);
        }
    }
}

const VERTEX_BYTES: u64 = 12;
/// 1 フレームで載せる頂点の目安。これ以上だと UI が止まる。
const UPLOAD_FRAME_BYTES: u64 = 8 * 1024 * 1024;

/// `max_buffer_size` に収まる頂点数。三角形境界に揃える。
fn verts_per_chunk(max_buffer_size: u64) -> usize {
    let verts = (max_buffer_size / VERTEX_BYTES) as usize;
    (verts / 3 * 3).max(3)
}

/// UI を止めない大きさの、1 フレーム分の頂点数。
pub(crate) fn verts_per_frame(max_buffer_size: u64) -> usize {
    verts_per_chunk(UPLOAD_FRAME_BYTES.min(max_buffer_size))
}

/// `[start, total)` から次に載せる三角形境界の範囲。
pub(crate) fn next_upload_range(start: usize, total: usize, cap: usize) -> Option<std::ops::Range<usize>> {
    if start >= total || cap < 3 {
        return None;
    }
    let mut end = (start + cap).min(total);
    end -= end % 3;
    if end <= start {
        end = (start + 3).min(total);
        end -= end % 3;
    }
    if end <= start {
        None
    } else {
        Some(start..end)
    }
}

pub(crate) fn upload_positions(device: &wgpu::Device, positions: &[[f32; 3]]) -> GpuChunk {
    upload_chunk(device, positions)
}

fn upload_chunk(device: &wgpu::Device, positions: &[[f32; 3]]) -> GpuChunk {
    let n = positions.len();
    debug_assert!(n == 0 || n <= verts_per_chunk(device.limits().max_buffer_size));
    let size = (n.max(1) as u64) * VERTEX_BYTES;
    let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("meshpad.vertices"),
        size,
        usage: wgpu::BufferUsages::VERTEX,
        mapped_at_creation: n > 0,
    });
    if n > 0 {
        {
            let mut mapped = vertex_buffer.slice(..).get_mapped_range_mut();
            let dst: &mut [[f32; 3]] = bytemuck::cast_slice_mut(&mut mapped);
            debug_assert_eq!(dst.len(), n);
            dst.copy_from_slice(positions);
        }
        vertex_buffer.unmap();
    }
    GpuChunk {
        vertex_buffer,
        vertex_count: n as u32,
    }
}

/// オフスクリーンへメッシュを描き、egui テクスチャとして出すレンダラ。
///
/// メインの egui パスには深度が無いので、カラー＋深度を別ターゲットに描いてから [`Self::egui_tex`] を `Image` に載せる。
pub struct Renderer {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniform_buffer: wgpu::Buffer,
    color: wgpu::Texture,
    color_view: wgpu::TextureView,
    depth: wgpu::Texture,
    depth_view: wgpu::TextureView,
    size: [u32; 2],
    /// egui の `Image` に渡すテクスチャ ID。
    ///
    /// [`Self::resize`] の直後は作り直しのため `None`。次の [`Self::sync_egui_tex`] で埋まる。
    pub egui_tex: Option<egui::TextureId>,
}

impl Renderer {
    /// パイプラインと最小サイズのオフスクリーンターゲットを用意する。
    ///
    /// 実ウィンドウサイズへの合わせは最初の [`Self::resize`] で行う。
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("meshpad.mesh"),
            source: wgpu::ShaderSource::Wgsl(include_str!("mesh.wgsl").into()),
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("meshpad.bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: NonZeroU64::new(std::mem::size_of::<Uniforms>() as u64),
                },
                count: None,
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("meshpad.pl"),
            bind_group_layouts: &[&bgl],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("meshpad.pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: 12,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: COLOR,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("meshpad.ubo"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meshpad.bg"),
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });
        let size = [8, 8];
        let (color, color_view, depth, depth_view) = make_targets(device, size);
        let this = Self {
            pipeline,
            bind_group,
            uniform_buffer,
            color,
            color_view,
            depth,
            depth_view,
            size,
            egui_tex: None,
        };
        this
    }

    /// オフスクリーンカラーを egui に登録または更新する。
    ///
    /// [`Self::resize`] のあと、描画の前に必ず呼ぶ。未登録なら新規 ID、既存なら中身だけ差し替える。
    pub fn sync_egui_tex(&mut self, device: &wgpu::Device, renderer: &mut egui_wgpu::Renderer) {
        match self.egui_tex {
            Some(id) => {
                renderer.update_egui_texture_from_wgpu_texture(
                    device,
                    &self.color_view,
                    wgpu::FilterMode::Linear,
                    id,
                );
            }
            None => {
                self.egui_tex = Some(renderer.register_native_texture(
                    device,
                    &self.color_view,
                    wgpu::FilterMode::Linear,
                ));
            }
        }
    }

    /// ビューポートサイズに合わせてカラー／深度を作り直す。
    ///
    /// 幅・高さとも最低 1。サイズが変わっていなければ何もしない。
    /// 作り直したあとは [`Self::egui_tex`] を捨てるので、続けて [`Self::sync_egui_tex`] が必要。
    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        let w = width.max(1);
        let h = height.max(1);
        if self.size == [w, h] {
            return;
        }
        self.size = [w, h];
        let (c, cv, d, dv) = make_targets(device, self.size);
        self.color = c;
        self.color_view = cv;
        self.depth = d;
        self.depth_view = dv;
        self.egui_tex = None;
    }

    /// オフスクリーンをクリアし、シーンがあれば描く。
    ///
    /// `scene` が `None`、または頂点が無ければ背景色だけになる。
    pub fn render(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        camera: &Camera,
        aspect: f32,
        scene: Option<&SceneGpu>,
    ) {
        let (view, proj, _) = camera.view_proj(aspect);
        let vp = proj * view;
        let origin = scene.map(|s| s.origin.to_array()).unwrap_or([0.0; 3]);
        let uniforms = Uniforms {
            view_proj: vp.to_cols_array_2d(),
            light_dir: camera.light_dir().to_array(),
            _pad0: 0.0,
            color: [0.74, 0.74, 0.76, 1.0],
            origin,
            _pad1: 0.0,
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("meshpad.enc"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("meshpad.pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.color_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.10,
                            g: 0.10,
                            b: 0.11,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            if let Some(scene) = scene {
                if scene.chunks.iter().any(|c| c.vertex_count > 0)
                    || scene.proxy.as_ref().is_some_and(|p| p.vertex_count > 0)
                {
                    pass.set_pipeline(&self.pipeline);
                    pass.set_bind_group(0, &self.bind_group, &[]);
                    scene.draw(&mut pass);
                }
            }
        }
        queue.submit(Some(encoder.finish()));
    }
}

fn make_targets(
    device: &wgpu::Device,
    size: [u32; 2],
) -> (
    wgpu::Texture,
    wgpu::TextureView,
    wgpu::Texture,
    wgpu::TextureView,
) {
    let color = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("meshpad.color"),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: COLOR,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let depth = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("meshpad.depth"),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let color_view = color.create_view(&wgpu::TextureViewDescriptor::default());
    let depth_view = depth.create_view(&wgpu::TextureViewDescriptor::default());
    (color, color_view, depth, depth_view)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_wgpu_buffer_fits_triangle_aligned_chunk() {
        let cap = verts_per_chunk(256 * 1024 * 1024);
        assert_eq!(cap % 3, 0);
        assert!(cap as u64 * VERTEX_BYTES <= 256 * 1024 * 1024);
    }

    #[test]
    fn lucy_scale_mesh_splits_under_256mb() {
        // Device::create_buffer が拒否した 1010006712 バイト（lucy 級）。
        let verts = 1_010_006_712u64 / VERTEX_BYTES;
        let cap = verts_per_chunk(256 * 1024 * 1024);
        assert!(verts as usize > cap);
        let n_chunks = (verts as usize).div_ceil(cap);
        assert_eq!(n_chunks, 4);
    }

    #[test]
    fn upload_range_stays_triangle_aligned() {
        let cap = verts_per_frame(256 * 1024 * 1024);
        assert_eq!(cap % 3, 0);
        let r = next_upload_range(0, 10_000, cap).unwrap();
        assert_eq!(r.start, 0);
        assert_eq!(r.end % 3, 0);
        assert!(r.end <= cap || r.end == 10_000 / 3 * 3);
    }
}
