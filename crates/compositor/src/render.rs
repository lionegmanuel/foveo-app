//! Pipeline GPU (`wgpu`/WGSL) de crop + scale. Sin surface/ventana: renderiza
//! siempre a una textura offscreen y lee el resultado de vuelta a CPU para
//! pasarlo al exporter (ver ARQUITECTURA.md 3.2, diagrama de flujo).

use bytemuck::{Pod, Zeroable};
use project::Rect;

#[derive(Debug, thiserror::Error)]
pub enum CompositorError {
    #[error("no se encontro un adaptador de GPU compatible: {0}")]
    NoAdapter(String),
    #[error("no se pudo obtener el device/queue de GPU: {0}")]
    NoDevice(String),
    #[error("el buffer de entrada no tiene el tamanio esperado (esperado {expected}, recibido {actual})")]
    BadInputSize { expected: usize, actual: usize },
    #[error("no se pudo mapear el buffer de lectura: {0}")]
    MapFailed(String),
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct CropUniform {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

/// Formato de pixel usado en toda la pipeline (entrada, textura de render,
/// lectura de salida): BGRA8, el mismo que produce `windows-capture` y que
/// consume `ffmpeg -pix_fmt bgra`, para no tener que convertir en el medio.
pub const PIXEL_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;
const BYTES_PER_PIXEL: u32 = 4;

/// Compone frames individuales: recibe un frame crudo (BGRA8, `input_width` x
/// `input_height`) y un rect de crop normalizado, y devuelve el frame
/// compuesto ya recortado/escalado a `output_width` x `output_height`.
pub struct Compositor {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    input_texture: wgpu::Texture,
    input_view: wgpu::TextureView,
    uniform_buffer: wgpu::Buffer,
    output_texture: wgpu::Texture,
    output_view: wgpu::TextureView,
    readback_buffer: wgpu::Buffer,
    input_width: u32,
    input_height: u32,
    output_width: u32,
    output_height: u32,
    padded_bytes_per_row: u32,
    unpadded_bytes_per_row: u32,
}

impl Compositor {
    /// Crea el device de GPU y toda la pipeline. Bloqueante (usa `pollster`
    /// para esperar las requests async de wgpu) — llamar desde un thread
    /// dedicado, nunca desde el hilo de UI (ver CLAUDE.md regla 1).
    pub fn new(
        input_width: u32,
        input_height: u32,
        output_width: u32,
        output_height: u32,
    ) -> Result<Self, CompositorError> {
        // Headless/offscreen: no necesitamos un display handle (no hay ventana).
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .map_err(|e| CompositorError::NoAdapter(e.to_string()))?;

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("screenzoom-compositor-device"),
            ..Default::default()
        }))
        .map_err(|e| CompositorError::NoDevice(e.to_string()))?;

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("crop_scale"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/crop_scale.wgsl").into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("crop_scale_bind_group_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
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
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("crop_scale_pipeline_layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("crop_scale_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: PIXEL_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("crop_scale_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let input_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("input_texture"),
            size: wgpu::Extent3d { width: input_width, height: input_height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: PIXEL_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let input_view = input_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("crop_uniform"),
            size: std::mem::size_of::<CropUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let output_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("output_texture"),
            size: wgpu::Extent3d { width: output_width, height: output_height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: PIXEL_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let output_view = output_texture.create_view(&wgpu::TextureViewDescriptor::default());

        // `bytes_per_row` en una copia textura->buffer tiene que ser multiplo
        // de COPY_BYTES_PER_ROW_ALIGNMENT (256); si el ancho de salida no cae
        // justo, hay que paddear cada fila y descartar el padding al leer.
        let unpadded_bytes_per_row = output_width * BYTES_PER_PIXEL;
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded_bytes_per_row = unpadded_bytes_per_row.div_ceil(align) * align;

        let readback_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback_buffer"),
            size: (padded_bytes_per_row as u64) * (output_height as u64),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        Ok(Self {
            device,
            queue,
            pipeline,
            bind_group_layout,
            sampler,
            input_texture,
            input_view,
            uniform_buffer,
            output_texture,
            output_view,
            readback_buffer,
            input_width,
            input_height,
            output_width,
            output_height,
            padded_bytes_per_row,
            unpadded_bytes_per_row,
        })
    }

    /// Compone un frame: sube `input_bgra` a GPU, recorta/escala segun
    /// `crop_rect` (coordenadas normalizadas 0..1) y devuelve el frame
    /// resultante como bytes BGRA8 de `output_width * output_height * 4`.
    pub fn composite_frame(&mut self, input_bgra: &[u8], crop_rect: Rect) -> Result<Vec<u8>, CompositorError> {
        let expected = (self.input_width * self.input_height * BYTES_PER_PIXEL) as usize;
        if input_bgra.len() != expected {
            return Err(CompositorError::BadInputSize { expected, actual: input_bgra.len() });
        }

        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.input_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            input_bgra,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(self.input_width * BYTES_PER_PIXEL),
                rows_per_image: Some(self.input_height),
            },
            wgpu::Extent3d { width: self.input_width, height: self.input_height, depth_or_array_layers: 1 },
        );

        let crop_uniform = CropUniform { x: crop_rect.x, y: crop_rect.y, w: crop_rect.w, h: crop_rect.h };
        self.queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&crop_uniform));

        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("crop_scale_bind_group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&self.input_view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: self.uniform_buffer.as_entire_binding() },
            ],
        });

        let mut encoder =
            self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame_encoder") });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("crop_scale_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.output_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.output_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.readback_buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.padded_bytes_per_row),
                    rows_per_image: Some(self.output_height),
                },
            },
            wgpu::Extent3d { width: self.output_width, height: self.output_height, depth_or_array_layers: 1 },
        );

        self.queue.submit(Some(encoder.finish()));

        self.read_output_buffer()
    }

    fn read_output_buffer(&mut self) -> Result<Vec<u8>, CompositorError> {
        let slice = self.readback_buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| CompositorError::MapFailed(e.to_string()))?;
        rx.recv()
            .map_err(|e| CompositorError::MapFailed(e.to_string()))?
            .map_err(|e| CompositorError::MapFailed(e.to_string()))?;

        let data = slice.get_mapped_range().map_err(|e| CompositorError::MapFailed(e.to_string()))?;
        let mut out = Vec::with_capacity((self.unpadded_bytes_per_row * self.output_height) as usize);
        for row in 0..self.output_height as usize {
            let start = row * self.padded_bytes_per_row as usize;
            let end = start + self.unpadded_bytes_per_row as usize;
            out.extend_from_slice(&data[start..end]);
        }
        drop(data);
        self.readback_buffer.unmap();

        Ok(out)
    }

    #[must_use]
    pub fn output_frame_size(&self) -> usize {
        (self.unpadded_bytes_per_row * self.output_height) as usize
    }
}
