//! The example's OWN rendering stack — the plugin packages bevy and renders the
//! wizard-hat cone into a dmabuf it allocates itself, handing only the fd across
//! the contract (`draw_dmabuf`). The contract is engine-agnostic; this example
//! borrows the repo's vendored bevy/wgpu fork (path deps) because the fork carries
//! the dmabuf import helper — an out-of-tree artifact would ship its own engine +
//! import plumbing.
//!
//! Pipeline: gbm LINEAR BO → export fd → import into wgpu as a render target →
//! headless bevy `App` (manual render resources) with a mini-bridge swapping the
//! camera's placeholder `GpuImage` for the imported texture → spinning cone.

use bevy::asset::RenderAssetUsages;
use bevy::camera::{Camera, Camera3d, ClearColorConfig, ImageRenderTarget, RenderTarget};
use bevy::image::Image;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{
    Extent3d, TextureDescriptor, TextureDimension, TextureFormat, TextureUsages,
};
use bevy::render::renderer::{
    RenderAdapter, RenderAdapterInfo, RenderDevice, RenderInstance, RenderQueue, WgpuWrapper,
};
use bevy::render::settings::{RenderCreation, RenderResources};
use bevy::render::texture::GpuImage;
use bevy::render::{Render, RenderApp, RenderDebugFlags, RenderPlugin, RenderSystems};
use bevy::render::render_asset::RenderAssets;
use gbm::{BufferObjectFlags, Device as GbmDevice, Format as GbmFormat};
use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::{Arc, Mutex};

/// Buffer pixel size (square). The quad rect stretches it wherever it's placed.
pub const BUFFER_PX: u32 = 256;
/// DRM_FORMAT_ARGB8888 ('AR24').
pub const FOURCC_ARGB8888: u32 = 0x3432_5241;

pub struct HatRenderer {
    app: App,
    pub stride: u32,
    pub offset: u32,
    pub modifier: u64,
    pub fourcc: u32,
    fd: OwnedFd,
    // Keep the allocation alive for the plugin's lifetime (drop after `fd`).
    _bo: gbm::BufferObject<()>,
    _gbm: GbmDevice<OwnedFd>,
}

#[derive(Component)]
struct Spin;

/// The one bridge entry: swap the placeholder `GpuImage` for the imported dmabuf
/// texture (mirror of the compositor's `BridgeRegistryPlugin`, single-entry).
#[derive(Resource, Clone)]
struct HatBridge {
    texture: Arc<wgpu::Texture>,
    handle: Handle<Image>,
    installed: Arc<Mutex<bool>>,
}

impl HatRenderer {
    pub fn raw_fd(&self) -> i32 {
        self.fd.as_raw_fd()
    }

    /// Render one frame into the dmabuf.
    pub fn frame(&mut self) {
        self.app.update();
    }

    /// `fourcc`/`modifiers` are the HOST-NEGOTIATED importable format from
    /// `DmabufCtx` — allocate with exactly these so the compositor imports
    /// zero-copy, falling back to the linear/implicit cascade only when the
    /// negotiated list is empty or the driver refuses it.
    pub fn new(fourcc: u32, modifiers: &[u64]) -> Result<Self, String> {
        // 1. Allocate a gbm BO and export it. Preference order: the negotiated
        //    modifier list → explicit-LINEAR via the modifiers API → LINEAR flag →
        //    implicit; trying every render node (env override first).
        let mut nodes: Vec<String> = std::env::var("COMPOSITOR_RENDER_NODE")
            .ok()
            .filter(|n| !n.trim().is_empty())
            .into_iter()
            .collect();
        if let Ok(dir) = std::fs::read_dir("/dev/dri") {
            let mut found: Vec<String> = dir
                .flatten()
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|n| n.starts_with("renderD"))
                .map(|n| format!("/dev/dri/{n}"))
                .collect();
            found.sort();
            nodes.extend(found);
        }
        let gbm_fmt = match fourcc {
            FOURCC_ARGB8888 => GbmFormat::Argb8888,
            _ => GbmFormat::Argb8888, // v1: the host negotiates ARGB8888
        };
        let mut picked: Option<(GbmDevice<OwnedFd>, gbm::BufferObject<()>, String)> = None;
        let mut errors = String::new();
        'nodes: for node in &nodes {
            let Ok(drm) = std::fs::OpenOptions::new().read(true).write(true).open(node) else {
                continue;
            };
            let Ok(gbm) = GbmDevice::new(OwnedFd::from(drm)) else { continue };
            let mut attempts: Vec<(&str, Result<gbm::BufferObject<()>, std::io::Error>)> = Vec::new();
            if !modifiers.is_empty() {
                attempts.push((
                    "negotiated",
                    gbm.create_buffer_object_with_modifiers2::<()>(
                        BUFFER_PX,
                        BUFFER_PX,
                        gbm_fmt,
                        modifiers.iter().map(|m| gbm::Modifier::from(*m)),
                        BufferObjectFlags::RENDERING,
                    ),
                ));
            }
            attempts.push((
                "linear-modifiers",
                gbm.create_buffer_object_with_modifiers2::<()>(
                    BUFFER_PX,
                    BUFFER_PX,
                    gbm_fmt,
                    [gbm::Modifier::Linear].into_iter(),
                    BufferObjectFlags::RENDERING,
                ),
            ));
            attempts.push((
                "linear-flag",
                gbm.create_buffer_object::<()>(
                    BUFFER_PX,
                    BUFFER_PX,
                    gbm_fmt,
                    BufferObjectFlags::RENDERING | BufferObjectFlags::LINEAR,
                ),
            ));
            attempts.push((
                "implicit",
                gbm.create_buffer_object::<()>(BUFFER_PX, BUFFER_PX, gbm_fmt, BufferObjectFlags::RENDERING),
            ));
            for (kind, res) in attempts {
                match res {
                    Ok(bo) => {
                        if bo.plane_count() != 1 {
                            errors.push_str(&format!("[{node} {kind}: {} planes] ", bo.plane_count()));
                            continue;
                        }
                        eprintln!("wizard plugin: gbm bo via {kind} on {node} (modifier {:?})", bo.modifier());
                        picked = Some((gbm, bo, node.clone()));
                        break 'nodes;
                    }
                    Err(e) => errors.push_str(&format!("[{node} {kind}: {e}] ")),
                }
            }
        }
        let (gbm, bo, _node) = picked.ok_or(format!("gbm bo: no allocation worked: {errors}"))?;
        let modifier: u64 = Into::<u64>::into(bo.modifier());
        let fd = bo.fd_for_plane(0).map_err(|e| format!("bo fd: {e}"))?;
        let stride = bo.stride_for_plane(0);
        let offset = bo.offset(0);

        // 2. Own wgpu (vendored fork; mirror of the compositor's own context
        //    creation — the dmabuf import features must be explicitly requested).
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            flags: wgpu::InstanceFlags::empty(),
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
            backend_options: wgpu::BackendOptions::default(),
            display: None,
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
            apply_limit_buckets: false,
        }))
        .map_err(|e| format!("adapter: {e:?}"))?;
        let required = wgpu::Features::VULKAN_EXTERNAL_MEMORY_FD
            | wgpu::Features::VULKAN_EXTERNAL_MEMORY_DMA_BUF;
        let available = adapter.features();
        if !(required - available).is_empty() {
            return Err(format!("missing dmabuf features: {:?}", required - available));
        }
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            label: Some("wizard-plugin"),
            required_features: required,
            required_limits: wgpu::Limits::default(),
            memory_hints: wgpu::MemoryHints::default(),
            trace: wgpu::Trace::Off,
        }))
        .map_err(|e| format!("device: {e}"))?;

        // 3. Import the BO as a wgpu render-target texture (mirror of the host's
        //    `import_dmabuf_to_wgpu`, single-plane LINEAR).
        let dup = fd.try_clone().map_err(|e| format!("fd dup: {e}"))?;
        let hal_desc = wgpu::hal::TextureDescriptor {
            label: Some("wizard_dmabuf"),
            size: wgpu::Extent3d { width: BUFFER_PX, height: BUFFER_PX, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Bgra8UnormSrgb,
            usage: wgpu::TextureUses::COLOR_TARGET
                | wgpu::TextureUses::RESOURCE
                | wgpu::TextureUses::COPY_SRC
                | wgpu::TextureUses::COPY_DST,
            memory_flags: wgpu::hal::MemoryFlags::empty(),
            view_formats: vec![],
        };
        let hal_texture = unsafe {
            let guard = device.as_hal::<wgpu::hal::api::Vulkan>();
            let hal_device = guard.as_ref().ok_or("not a vulkan device")?;
            hal_device
                .texture_from_dmabuf_fd(dup, &hal_desc, modifier, stride as u64, offset as u64)
                .map_err(|e| format!("dmabuf import: {e:?}"))?
        };
        let texture = unsafe {
            device.create_texture_from_hal::<wgpu::hal::api::Vulkan>(
                hal_texture,
                &wgpu::TextureDescriptor {
                    label: Some("wizard_dmabuf"),
                    size: wgpu::Extent3d { width: BUFFER_PX, height: BUFFER_PX, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Bgra8UnormSrgb,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::COPY_SRC
                        | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                },
            )
        };

        // 4. Headless bevy on OUR device (manual render resources, mirror of the
        //    compositor's app boot).
        let mut app = App::new();
        let render_plugin = RenderPlugin {
            debug_flags: RenderDebugFlags::all(),
            render_creation: RenderCreation::Manual(RenderResources(
                RenderDevice::from(device.clone()),
                RenderQueue(Arc::new(WgpuWrapper::new(queue.clone()))),
                RenderAdapterInfo(WgpuWrapper::new(adapter.get_info())),
                RenderAdapter(Arc::new(WgpuWrapper::new(adapter.clone()))),
                RenderInstance(Arc::new(WgpuWrapper::new(instance.clone()))),
            )),
            synchronous_pipeline_compilation: false,
        };
        app.add_plugins(DefaultPlugins.build().set(render_plugin));

        // 5. Output placeholder image the bridge later swaps for the imported texture.
        let extent = Extent3d { width: BUFFER_PX, height: BUFFER_PX, depth_or_array_layers: 1 };
        let handle = {
            let mut images = app.world_mut().resource_mut::<Assets<Image>>();
            let mut image = Image {
                texture_descriptor: TextureDescriptor {
                    label: Some("wizard_output_placeholder"),
                    size: extent,
                    dimension: TextureDimension::D2,
                    format: TextureFormat::Bgra8UnormSrgb,
                    mip_level_count: 1,
                    sample_count: 1,
                    usage: TextureUsages::TEXTURE_BINDING
                        | TextureUsages::COPY_DST
                        | TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                },
                asset_usage: RenderAssetUsages::all(),
                ..Default::default()
            };
            image.resize(extent);
            images.add(image)
        };

        // 6. Scene: camera into the output, light, spinning purple cone.
        let (mesh, material) = {
            let world = app.world_mut();
            let mesh = world.resource_mut::<Assets<Mesh>>().add(cone_mesh());
            let material = world.resource_mut::<Assets<StandardMaterial>>().add(StandardMaterial {
                base_color: Color::srgb(0.45, 0.20, 0.85),
                metallic: 0.1,
                perceptual_roughness: 0.55,
                ..Default::default()
            });
            (mesh, material)
        };
        app.world_mut().spawn((
            Camera3d::default(),
            Camera { clear_color: ClearColorConfig::Custom(Color::NONE), ..Default::default() },
            Projection::Perspective(PerspectiveProjection {
                fov: 0.9,
                aspect_ratio: 1.0,
                near: 0.05,
                far: 100.0,
                ..Default::default()
            }),
            Transform::from_xyz(0.0, 0.8, 2.2).looking_at(Vec3::new(0.0, 0.45, 0.0), Vec3::Y),
            RenderTarget::Image(ImageRenderTarget::from(handle.clone())),
        ));
        app.world_mut().spawn((
            DirectionalLight { illuminance: 12_000.0, ..Default::default() },
            Transform::from_xyz(2.0, 4.0, 3.0).looking_at(Vec3::ZERO, Vec3::Y),
        ));
        app.world_mut().spawn((Mesh3d(mesh), MeshMaterial3d(material), Transform::IDENTITY, Spin));
        app.add_systems(Update, spin);

        // 7. Mini-bridge in the render app.
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app
                .insert_resource(HatBridge {
                    texture: Arc::new(texture),
                    handle: handle.clone(),
                    installed: Arc::new(Mutex::new(false)),
                })
                .add_systems(Render, install_bridge.in_set(RenderSystems::Prepare));
        }

        app.finish();
        app.cleanup();

        Ok(Self { app, stride, offset, modifier, fourcc, fd, _bo: bo, _gbm: gbm })
    }
}

fn spin(time: Res<Time>, mut query: Query<&mut Transform, With<Spin>>) {
    for mut transform in &mut query {
        transform.rotate_y(time.delta_secs() * 0.9);
    }
}

fn install_bridge(
    bridge: Res<HatBridge>,
    mut gpu_images: ResMut<RenderAssets<GpuImage>>,
    render_device: Res<RenderDevice>,
) {
    let Ok(mut installed) = bridge.installed.lock() else { return };
    if *installed {
        return;
    }
    let Some(existing) = gpu_images.get_mut(&bridge.handle) else { return };
    let bevy_texture: bevy::render::render_resource::Texture = (*bridge.texture).clone().into();
    let texture_view = bevy_texture.create_view(&Default::default());
    let sampler = render_device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("wizard_output_sampler"),
        ..Default::default()
    });
    existing.texture = bevy_texture;
    existing.texture_view = texture_view;
    existing.sampler = sampler;
    *installed = true;
}

/// The wizard-hat cone, built procedurally (24 segments, smooth sides + base disk).
fn cone_mesh() -> Mesh {
    const N: usize = 24;
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut indices: Vec<u16> = Vec::new();

    let apex = [0.0f32, 1.0, 0.0];
    let slant = (0.5f32 * 0.5 + 1.0).sqrt();
    let ny = 0.5 / slant;
    let nr = 1.0 / slant;
    let ring: Vec<[f32; 3]> = (0..N)
        .map(|i| {
            let a = std::f32::consts::TAU * i as f32 / N as f32;
            [0.5 * a.cos(), 0.0, 0.5 * a.sin()]
        })
        .collect();
    for i in 0..N {
        let a = ring[i];
        let b = ring[(i + 1) % N];
        let base = positions.len() as u16;
        for p in [apex, b, a] {
            positions.push(p);
            if p == apex {
                let mx = (a[0] + b[0]) / 2.0;
                let mz = (a[2] + b[2]) / 2.0;
                let l = (mx * mx + mz * mz).sqrt().max(1e-6);
                normals.push([mx / l * nr, ny, mz / l * nr]);
            } else {
                let l = (p[0] * p[0] + p[2] * p[2]).sqrt().max(1e-6);
                normals.push([p[0] / l * nr, ny, p[2] / l * nr]);
            }
        }
        indices.extend_from_slice(&[base, base + 1, base + 2]);
    }
    let center = positions.len() as u16;
    positions.push([0.0, 0.0, 0.0]);
    normals.push([0.0, -1.0, 0.0]);
    let ring_start = positions.len() as u16;
    for p in &ring {
        positions.push(*p);
        normals.push([0.0, -1.0, 0.0]);
    }
    for i in 0..N as u16 {
        indices.extend_from_slice(&[center, ring_start + i, ring_start + (i + 1) % N as u16]);
    }

    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::all())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_indices(Indices::U16(indices))
}
