use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    fs,
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};

use smithay::{
    backend::{
        allocator::{
            gbm::{GbmAllocator, GbmBufferFlags, GbmDevice},
            Format, Fourcc, Modifier,
        },
        drm::{
            compositor::FrameFlags,
            exporter::gbm::GbmFramebufferExporter,
            output::{DrmOutput, DrmOutputManager, DrmOutputRenderElements},
            DrmDevice, DrmDeviceFd, DrmEvent, DrmNode, NodeType, Planes,
        },
        egl::context::ContextPriority,
        input::{InputEvent, KeyState, KeyboardKeyEvent},
        libinput::{LibinputInputBackend, LibinputSessionInterface},
        renderer::{
            element::surface::WaylandSurfaceRenderElement,
            element::{
                memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
                solid::{SolidColorBuffer, SolidColorRenderElement},
                Element, Id, Kind, RenderElement, UnderlyingStorage,
            },
            gles::GlesRenderer,
            multigpu::{gbm::GbmGlesBackend, GpuManager, MultiRenderer},
            utils::{CommitCounter, DamageSet, OpaqueRegions},
            Color32F, ImportDma, ImportEgl, ImportMemWl, RendererSuper,
        },
        session::{libseat::LibSeatSession, Event as SessionEvent, Session},
        udev::{UdevBackend, UdevEvent},
    },
    desktop::{space::SpaceRenderElements, utils::send_frames_surface_tree, Space, Window},
    input::pointer::{CursorIcon, CursorImageStatus, CursorImageSurfaceData},
    reexports::{
        calloop::{
            timer::{TimeoutAction, Timer},
            EventLoop,
        },
        drm::control::{connector, crtc, Device as ControlDevice},
        input::{DeviceCapability, Libinput},
        rustix::fs::OFlags,
        wayland_server::protocol::wl_surface::WlSurface,
    },
    utils::{
        Buffer, DeviceFd, Logical, Physical, Point, Rectangle, Scale as RendererScale, Size,
        Transform,
    },
    wayland::compositor,
};
use tracing::{error, info, warn};

use crate::{
    config::{OutputConfig, OutputModeConfig},
    render::{dnd_icon_elements, CaptureCursor, RenderState, YawcRenderElements},
    state::OverviewWindow,
    window::{WindowFrame, WindowStore},
    CalloopData,
};

type TtyRenderBackend = GbmGlesBackend<GlesRenderer, DrmDeviceFd>;
type TtyRenderer<'a> = MultiRenderer<'a, 'a, TtyRenderBackend, TtyRenderBackend>;
type TtyOutputManager = DrmOutputManager<
    GbmAllocator<DrmDeviceFd>,
    GbmFramebufferExporter<DrmDeviceFd>,
    (),
    DrmDeviceFd,
>;
type TtyOutput =
    DrmOutput<GbmAllocator<DrmDeviceFd>, GbmFramebufferExporter<DrmDeviceFd>, (), DrmDeviceFd>;

smithay::backend::renderer::element::render_elements! {
    TtyRenderElements<='a, TtyRenderer<'a>>;
    Yawc=TtyYawcElement,
    Space=SpaceRenderElements<TtyRenderer<'a>, WaylandSurfaceRenderElement<TtyRenderer<'a>>>,
    CursorSurface=WaylandSurfaceRenderElement<TtyRenderer<'a>>,
    Memory=MemoryRenderBufferRenderElement<TtyRenderer<'a>>,
    Solid=SolidColorRenderElement,
}

struct TtyYawcElement(YawcRenderElements);

impl From<YawcRenderElements> for TtyYawcElement {
    fn from(element: YawcRenderElements) -> Self {
        Self(element)
    }
}

impl Element for TtyYawcElement {
    fn id(&self) -> &Id {
        self.0.id()
    }

    fn current_commit(&self) -> CommitCounter {
        self.0.current_commit()
    }

    fn location(&self, scale: RendererScale<f64>) -> Point<i32, Physical> {
        self.0.location(scale)
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        self.0.src()
    }

    fn transform(&self) -> Transform {
        self.0.transform()
    }

    fn geometry(&self, scale: RendererScale<f64>) -> Rectangle<i32, Physical> {
        self.0.geometry(scale)
    }

    fn damage_since(
        &self,
        scale: RendererScale<f64>,
        commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        self.0.damage_since(scale, commit)
    }

    fn opaque_regions(&self, scale: RendererScale<f64>) -> OpaqueRegions<i32, Physical> {
        self.0.opaque_regions(scale)
    }

    fn alpha(&self) -> f32 {
        self.0.alpha()
    }

    fn kind(&self) -> Kind {
        self.0.kind()
    }
}

impl<'a> RenderElement<TtyRenderer<'a>> for TtyYawcElement {
    fn draw(
        &self,
        frame: &mut <TtyRenderer<'a> as RendererSuper>::Frame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), <TtyRenderer<'a> as RendererSuper>::Error> {
        self.0
            .draw(frame.as_mut(), src, dst, damage, opaque_regions)
            .map_err(Into::into)
    }

    fn underlying_storage(&self, _renderer: &mut TtyRenderer<'a>) -> Option<UnderlyingStorage<'_>> {
        None
    }
}

struct TtyRuntime {
    node: DrmNode,
    drm_output_manager: TtyOutputManager,
    outputs: Vec<TtyOutputRuntime>,
    gpus: GpuManager<TtyRenderBackend>,
    cursor_theme: TtyCursorTheme,
    active: bool,
    reset_buffers_each_frame: bool,
    outputs_dirty: bool,
    ctrl_down: bool,
    alt_down: bool,
}

struct TtyOutputRuntime {
    connector: connector::Handle,
    name: String,
    crtc: crtc::Handle,
    drm_output: TtyOutput,
    render_state: RenderState,
    background: SolidColorBuffer,
    refresh_interval: Duration,
    connected: bool,
    primary: bool,
    frame_pending: bool,
    last_frame_queued: Option<Instant>,
    last_screencopy_frame: Option<Instant>,
    missed_vblank_warned: bool,
}

impl TtyRuntime {
    fn render(&mut self, data: &mut CalloopData) {
        if !self.active {
            return;
        }

        self.recover_missed_vblanks();

        let mut renderer = match self.gpus.single_renderer(&self.node) {
            Ok(renderer) => renderer,
            Err(error) => {
                error!(?error, "failed to acquire standalone renderer");
                data.state.loop_signal.stop();
                return;
            }
        };

        let pointer_location = data
            .state
            .seat
            .get_pointer()
            .map(|pointer| pointer.current_location());
        let cursor_image = data
            .state
            .compositor_cursor
            .map(|shape| CursorImageStatus::Named(shape.to_cursor_icon()))
            .unwrap_or_else(|| data.state.cursor_image.clone());
        if data.state.reload_config_if_changed() {
            self.outputs_dirty = true;
        }
        if self.outputs_dirty {
            self.outputs_dirty = false;
            rescan_outputs(
                &mut self.drm_output_manager,
                &mut self.outputs,
                &mut renderer,
                data,
            );
        }
        data.state.finish_close_animations();
        data.state.finish_overview_animation();
        let animation_config = data.state.config.animations();
        let render_requested = data.state.take_render_requested();
        let screencopy_pending = data.state.screencopy_state.has_pending();
        let animation_pending = data.state.windows.needs_animation_frame(animation_config);
        let overview_pending = data.state.overview_needs_animation_frame();
        let shutdown_pending = data.state.graceful_shutdown_pending();
        let should_render = render_requested
            || screencopy_pending
            || animation_pending
            || overview_pending
            || shutdown_pending;
        if !should_render {
            return;
        }

        let controls_mode = data.state.config.window_controls();
        let frames = data
            .state
            .windows
            .frames(&data.state.space, animation_config, controls_mode);
        let overview_windows = data.state.overview_windows();
        let drew_display_frame = render_requested || animation_pending || overview_pending;
        if drew_display_frame {
            for output in &mut self.outputs {
                if !output.connected || output.frame_pending {
                    continue;
                }

                let elements = render_elements(
                    &mut renderer,
                    &mut output.render_state,
                    &data.state.space,
                    &frames,
                    &data.state.windows,
                    animation_config,
                    &overview_windows,
                    &output.background,
                    &mut self.cursor_theme,
                    &cursor_image,
                    pointer_location,
                    data.state.dnd_icon.as_ref(),
                );

                if self.reset_buffers_each_frame {
                    output.drm_output.reset_buffers();
                }

                match output.drm_output.render_frame(
                    &mut renderer,
                    &elements,
                    [0.06, 0.09, 0.11, 1.0],
                    // The early standalone backend should be predictable before it is clever:
                    // force a fully composited frame instead of allowing direct scanout planes.
                    FrameFlags::empty(),
                ) {
                    Ok(result) => {
                        if !result.is_empty {
                            if let Err(error) = output.drm_output.queue_frame(()) {
                                warn!(?error, "failed to queue drm frame");
                            } else {
                                output.frame_pending = true;
                                output.last_frame_queued = Some(Instant::now());
                                output.missed_vblank_warned = false;
                            }
                        }
                    }
                    Err(error) => {
                        warn!(?error, "standalone render_frame failed");
                    }
                }
                drop(elements);
            }
        }

        if let Some(request) = data.state.screencopy_state.pop_pending() {
            let region = request.region();
            let capture_cursor =
                capture_cursor(&mut self.cursor_theme, &cursor_image, pointer_location);
            if let Some(output_index) = output_index_for_region(&self.outputs, region) {
                let output = &mut self.outputs[output_index];
                let screencopy_interval = output.refresh_interval.max(Duration::from_millis(1));
                let can_service_screencopy = output
                    .last_screencopy_frame
                    .map(|last_frame| last_frame.elapsed() >= screencopy_interval)
                    .unwrap_or(true);
                if can_service_screencopy {
                    output.last_screencopy_frame = Some(Instant::now());
                    capture_screencopy(
                        &mut output.render_state,
                        request,
                        renderer.as_mut(),
                        data,
                        &frames,
                        &overview_windows,
                        animation_config,
                        capture_cursor,
                    );
                } else {
                    data.state.screencopy_state.push_pending_front(request);
                }
            } else {
                request.cancel();
            }
        }

        if drew_display_frame {
            data.state.space.elements().for_each(|window| {
                let output = data.state.output_for_window(window).or_else(|| {
                    self.outputs
                        .iter()
                        .find(|output| output.connected && output.primary)
                        .or_else(|| self.outputs.iter().find(|output| output.connected))
                        .map(|output| output.render_state.output().clone())
                });
                let Some(output) = output else {
                    return;
                };
                window.send_frame(
                    &output,
                    data.state.start_time.elapsed(),
                    Some(Duration::ZERO),
                    |_, _| Some(output.clone()),
                );
            });
            if let Some(icon) = data.state.dnd_icon.as_ref() {
                let output = pointer_location
                    .and_then(|location| data.state.output_at(location))
                    .or_else(|| {
                        self.outputs
                            .iter()
                            .find(|output| output.connected && output.primary)
                            .or_else(|| self.outputs.iter().find(|output| output.connected))
                            .map(|output| output.render_state.output().clone())
                    });
                if let Some(output) = output {
                    send_frames_surface_tree(
                        icon,
                        &output,
                        data.state.start_time.elapsed(),
                        Some(Duration::ZERO),
                        |_, _| Some(output.clone()),
                    );
                }
            }
        }

        data.state.refresh_space_and_prune_windows();
        data.state.popups.cleanup();
        let _ = data.display_handle.flush_clients();
    }

    fn recover_missed_vblanks(&mut self) {
        for output in &mut self.outputs {
            if !output.connected || !output.frame_pending {
                continue;
            }

            let missed_vblank = output
                .last_frame_queued
                .map(|queued_at| queued_at.elapsed() > Duration::from_millis(250))
                .unwrap_or(true);

            if !missed_vblank {
                continue;
            }

            if !output.missed_vblank_warned {
                warn!(?output.crtc, "forcing standalone render after missed drm vblank");
                output.missed_vblank_warned = true;
            }

            if let Err(error) = output.drm_output.frame_submitted() {
                warn!(?error, ?output.crtc, "failed to recover from missed drm vblank");
            }
            output.frame_pending = false;
            output.last_frame_queued = None;
        }
    }

    fn handle_vblank(&mut self, crtc: crtc::Handle) {
        let Some(output) = self.outputs.iter_mut().find(|output| output.crtc == crtc) else {
            warn!(?crtc, "received vblank for unknown crtc");
            return;
        };

        output.frame_pending = false;
        output.last_frame_queued = None;
        output.missed_vblank_warned = false;
        if let Err(error) = output.drm_output.frame_submitted() {
            warn!(?error, ?crtc, "failed to mark drm frame as submitted");
        }
    }

    fn min_refresh_interval(&self) -> Duration {
        self.outputs
            .iter()
            .filter(|output| output.connected)
            .map(|output| output.refresh_interval)
            .min()
            .unwrap_or_else(|| Duration::from_millis(16))
    }

    fn handle_keyboard_shortcut(
        &mut self,
        key_code: u32,
        key_state: KeyState,
        session: &mut LibSeatSession,
        data: &mut CalloopData,
    ) -> bool {
        let pressed = key_state == KeyState::Pressed;

        match key_code {
            37 | 105 => {
                self.ctrl_down = pressed;
                return false;
            }
            64 | 108 => {
                self.alt_down = pressed;
                return false;
            }
            _ => {}
        }

        if !pressed || !self.ctrl_down || !self.alt_down {
            return false;
        }

        if let Some(vt) = function_key_to_vt(key_code) {
            info!(vt, "switching vt from emergency keyboard shortcut");
            if let Err(error) = session.change_vt(vt) {
                warn!(?error, vt, "failed to switch vt");
            }
            return true;
        }

        // Ctrl+Alt+Backspace or Ctrl+Alt+Esc: ask clients to close, then stop shortly after.
        if matches!(key_code, 22 | 9) {
            warn!("requesting graceful compositor shutdown from emergency keyboard shortcut");
            data.state.request_graceful_shutdown();
            return true;
        }

        false
    }
}

fn output_index_for_region(
    outputs: &[TtyOutputRuntime],
    region: Rectangle<i32, Logical>,
) -> Option<usize> {
    let center = Point::<i32, Logical>::from((
        region.loc.x + region.size.w / 2,
        region.loc.y + region.size.h / 2,
    ));

    outputs
        .iter()
        .enumerate()
        .filter(|(_, output)| output.connected)
        .find(|(_, output)| output.render_state.output_geometry().contains(center))
        .map(|(index, _)| index)
        .or_else(|| outputs.iter().position(|output| output.connected))
}

fn rescan_outputs(
    drm_output_manager: &mut TtyOutputManager,
    outputs: &mut Vec<TtyOutputRuntime>,
    renderer: &mut TtyRenderer<'_>,
    data: &mut CalloopData,
) {
    let configs = data.state.config.outputs();
    let selected = match select_connectors_and_modes(drm_output_manager.device(), &configs) {
        Ok(selected) => selected,
        Err(error) => {
            warn!(?error, "failed to rescan drm outputs");
            return;
        }
    };

    let mut connected = HashSet::new();
    let mut next_auto_x = 0;

    for (index, (connector_info, crtc, mode)) in selected.into_iter().enumerate() {
        let connector = connector_info.handle();
        let name = connector_name(&connector_info);
        connected.insert(connector);

        let config = output_config(&configs, &name);
        let scale = config.and_then(|config| config.scale).unwrap_or(1.0);
        let output_size = Size::<i32, Physical>::from((mode.size().0 as i32, mode.size().1 as i32));
        let logical_size = output_size.to_f64().to_logical(scale).to_i32_ceil();
        let location = configured_or_auto_location(config, &mut next_auto_x, logical_size);
        let refresh_hz = mode.vrefresh().max(1);
        let refresh_millihz = (refresh_hz as i32).saturating_mul(1000);
        let primary = config.map(|config| config.primary).unwrap_or(index == 0);

        if let Some(output) = outputs
            .iter_mut()
            .find(|output| output.connector == connector)
        {
            output.connected = true;
            output.primary = primary;
            output.refresh_interval =
                Duration::from_nanos(1_000_000_000 / refresh_hz.max(1) as u64);
            output.background = SolidColorBuffer::new(
                Size::<i32, Logical>::from((output_size.w, output_size.h)),
                Color32F::from([0.10, 0.13, 0.16, 1.0]),
            );

            if output.crtc == crtc {
                let render_elements: DrmOutputRenderElements<
                    _,
                    SpaceRenderElements<_, WaylandSurfaceRenderElement<_>>,
                > = Default::default();
                if output
                    .render_state
                    .output()
                    .current_mode()
                    .map(|current| {
                        current.size != output_size || current.refresh != refresh_millihz
                    })
                    .unwrap_or(true)
                {
                    if let Err(error) = output.drm_output.use_mode(mode, renderer, &render_elements)
                    {
                        warn!(?error, output = %name, "failed to apply configured drm mode");
                    }
                }
            } else {
                warn!(
                    output = %name,
                    old_crtc = ?output.crtc,
                    new_crtc = ?crtc,
                    "output crtc changed; keeping existing drm output until restart"
                );
            }

            output
                .render_state
                .reconfigure_output(output_size, refresh_millihz, location, scale);
            data.state
                .space
                .map_output(output.render_state.output(), location);
            info!(
                output = %name,
                x = location.x,
                y = location.y,
                scale,
                primary,
                "updated drm output"
            );
            continue;
        }

        match create_output_runtime(
            drm_output_manager,
            renderer,
            &data.display_handle,
            &mut data.state.space,
            connector_info,
            crtc,
            mode,
            index,
            config,
            &mut next_auto_x,
        ) {
            Ok(output) => outputs.push(output),
            Err(error) => warn!(?error, "failed to initialize hotplugged drm output"),
        }
    }

    for output in &mut *outputs {
        if !connected.contains(&output.connector) && output.connected {
            output.connected = false;
            output.frame_pending = false;
            output.last_frame_queued = None;
            data.state.space.unmap_output(output.render_state.output());
            info!(output = %output.name, "unmapped disconnected drm output");
        }
    }

    outputs.sort_by_key(|output| (!output.primary, output.name.clone()));
    for output in outputs.iter().filter(|output| output.connected) {
        let geometry = output.render_state.output_geometry();
        data.state.space.unmap_output(output.render_state.output());
        data.state
            .space
            .map_output(output.render_state.output(), geometry.loc);
    }

    data.state.request_render();
}

fn create_output_runtime(
    drm_output_manager: &mut TtyOutputManager,
    renderer: &mut TtyRenderer<'_>,
    display_handle: &smithay::reexports::wayland_server::DisplayHandle,
    space: &mut Space<Window>,
    connector_info: connector::Info,
    crtc: crtc::Handle,
    mode: smithay::reexports::drm::control::Mode,
    index: usize,
    config: Option<&OutputConfig>,
    next_auto_x: &mut i32,
) -> Result<TtyOutputRuntime, Box<dyn std::error::Error>> {
    let output_name = connector_name(&connector_info);
    let scale = config.and_then(|config| config.scale).unwrap_or(1.0);
    let output_size = Size::<i32, Physical>::from((mode.size().0 as i32, mode.size().1 as i32));
    let logical_size = output_size.to_f64().to_logical(scale).to_i32_ceil();
    let output_location = configured_or_auto_location(config, next_auto_x, logical_size);

    let background = SolidColorBuffer::new(
        Size::<i32, Logical>::from((output_size.w, output_size.h)),
        Color32F::from([0.10, 0.13, 0.16, 1.0]),
    );
    let refresh_hz = mode.vrefresh().max(1);
    let refresh_millihz = (refresh_hz as i32).saturating_mul(1000);
    let primary = config.map(|config| config.primary).unwrap_or(index == 0);
    info!(
        output = %output_name,
        ?crtc,
        x = output_location.x,
        y = output_location.y,
        scale,
        width = output_size.w,
        height = output_size.h,
        refresh_hz,
        primary,
        "selected drm mode for standalone output"
    );
    let render_state = RenderState::new_standalone_at(
        display_handle,
        space,
        output_size,
        refresh_millihz,
        output_name.clone(),
        output_location,
        scale,
    );
    let output = render_state.output().clone();
    let available_planes = drm_output_manager
        .device()
        .planes(&crtc)
        .map_err(|error| format!("failed to query drm planes: {error}"))?;
    let safe_planes = Planes {
        primary: available_planes.primary,
        cursor: Vec::new(),
        overlay: Vec::new(),
    };
    let render_elements: DrmOutputRenderElements<
        _,
        SpaceRenderElements<_, WaylandSurfaceRenderElement<_>>,
    > = Default::default();
    let drm_output = drm_output_manager
        .initialize_output(
            crtc,
            mode,
            &[connector_info.handle()],
            &output,
            Some(safe_planes),
            renderer,
            &render_elements,
        )
        .map_err(|error| format!("failed to initialize drm output: {error}"))?;

    let refresh_interval = Duration::from_nanos(1_000_000_000 / refresh_hz.max(1) as u64);
    Ok(TtyOutputRuntime {
        connector: connector_info.handle(),
        name: output_name,
        crtc,
        drm_output,
        render_state,
        background,
        refresh_interval,
        connected: true,
        primary,
        frame_pending: false,
        last_frame_queued: None,
        last_screencopy_frame: None,
        missed_vblank_warned: false,
    })
}

pub fn init(
    event_loop: &mut EventLoop<CalloopData>,
    data: &mut CalloopData,
) -> Result<(), Box<dyn std::error::Error>> {
    let (mut session, notifier) = LibSeatSession::new()
        .map_err(|error| format!("failed to initialize libseat session: {error}"))?;
    let seat_name = session.seat();
    let initial_active = session.is_active();

    info!(
        seat = %seat_name,
        active = initial_active,
        "initialized libseat session for standalone backend"
    );

    let udev_backend = UdevBackend::new(&seat_name)
        .map_err(|error| format!("failed to initialize udev backend: {error}"))?;
    let device_path = select_drm_device(&udev_backend)?;
    let node = DrmNode::from_path(&device_path)
        .map_err(|error| format!("failed to resolve drm node from {:?}: {error}", device_path))?;
    info!(path = ?device_path, ?node, "selected drm device for standalone backend");

    let opened_fd = session
        .open(
            &device_path,
            OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY,
        )
        .map_err(|error| format!("failed to open drm device {:?}: {error:?}", device_path))?;
    let device_fd = DrmDeviceFd::new(DeviceFd::from(opened_fd));
    let main_device = node
        .node_with_type(NodeType::Render)
        .and_then(Result::ok)
        .map(|render_node| render_node.dev_id())
        .or_else(|| match device_fd.dev_id() {
            Ok(device_id) => Some(device_id),
            Err(error) => {
                warn!(
                    ?error,
                    "failed to read DRM device id; linux-dmabuf feedback will be downgraded"
                );
                None
            }
        });
    if main_device.is_none() {
        warn!(
            ?node,
            "failed to resolve DRM render device id; linux-dmabuf feedback will be downgraded"
        );
    } else if node.ty() != NodeType::Render {
        info!(
            ?node,
            "using matching DRM render node for linux-dmabuf feedback"
        );
    }
    let (drm_device, drm_notifier) = DrmDevice::new(device_fd.clone(), true)
        .map_err(|error| format!("failed to initialize drm device: {error}"))?;

    let gbm = GbmDevice::new(device_fd.clone())
        .map_err(|error| format!("failed to create gbm device: {error}"))?;
    let allocator_flags = GbmBufferFlags::SCANOUT | GbmBufferFlags::RENDERING;
    info!(
        ?allocator_flags,
        "using gbm allocator flags for standalone primary buffers"
    );
    let allocator = GbmAllocator::new(gbm.clone(), allocator_flags);
    let exporter = GbmFramebufferExporter::new(gbm.clone(), None);

    let mut gpus = GpuManager::new(GbmGlesBackend::with_context_priority(ContextPriority::High))
        .map_err(|error| format!("failed to initialize gpu manager: {error}"))?;
    gpus.as_mut()
        .add_node(node, gbm.clone())
        .map_err(|error| format!("failed to add gbm node to gpu manager: {error}"))?;

    let mut renderer = gpus
        .single_renderer(&node)
        .map_err(|error| format!("failed to create renderer for standalone backend: {error}"))?;
    match renderer.bind_wl_display(&data.display_handle) {
        Ok(()) => info!("bound EGL display to Wayland display for wl_drm client buffers"),
        Err(error) => warn!(
            ?error,
            "failed to bind EGL display to Wayland display; clients may fall back to software EGL"
        ),
    }
    data.state.shm_state.update_formats(renderer.shm_formats());
    let renderer_dmabuf_formats = renderer.dmabuf_formats();
    let raw_dmabuf_formats = renderer_dmabuf_formats.iter().copied().collect::<Vec<_>>();
    let dmabuf_format_count = raw_dmabuf_formats.len();
    if !env_flag("YAWC_DISABLE_CLIENT_DMABUF") {
        let client_dmabuf_formats = if env_flag("YAWC_ENABLE_DMABUF_PROBE")
            && !env_flag("YAWC_ENABLE_CLIENT_DMABUF")
            && !data.state.config.screencopy_dmabuf()
        {
            raw_dmabuf_formats
                .iter()
                .copied()
                .filter(is_safe_probe_dmabuf_format)
                .collect::<Vec<_>>()
        } else {
            raw_dmabuf_formats.clone()
        };

        info!(
            raw_count = dmabuf_format_count,
            client_count = client_dmabuf_formats.len(),
            "enabling linux-dmabuf global"
        );
        data.state
            .init_dmabuf_global(client_dmabuf_formats, main_device);
    } else {
        info!(
            count = dmabuf_format_count,
            "linux-dmabuf global disabled by YAWC_DISABLE_CLIENT_DMABUF"
        );
    }

    let mut drm_output_manager = TtyOutputManager::new(
        drm_device,
        allocator,
        exporter,
        Some(gbm),
        [
            Fourcc::Xrgb8888,
            Fourcc::Xbgr8888,
            Fourcc::Argb8888,
            Fourcc::Abgr8888,
        ],
        renderer_dmabuf_formats,
    );

    let cursor_theme = TtyCursorTheme::load();
    let output_configs = data.state.config.outputs();
    let connectors = select_connectors_and_modes(drm_output_manager.device(), &output_configs)?;
    let mut outputs = Vec::new();
    let mut next_auto_x = 0;

    for (index, (connector_info, crtc, mode)) in connectors.into_iter().enumerate() {
        let output_name = connector_name(&connector_info);
        let output = create_output_runtime(
            &mut drm_output_manager,
            &mut renderer,
            &data.display_handle,
            &mut data.state.space,
            connector_info,
            crtc,
            mode,
            index,
            output_config(&output_configs, &output_name),
            &mut next_auto_x,
        )?;
        outputs.push(output);
    }

    let reset_buffers_each_frame = std::env::var("YAWC_DRM_RESET_BUFFERS_EACH_FRAME")
        .map(|value| value != "0")
        .unwrap_or(true);
    if reset_buffers_each_frame {
        warn!("resetting drm buffers before each frame to avoid damage-corruption artifacts");
    }

    let runtime = Rc::new(RefCell::new(TtyRuntime {
        node,
        drm_output_manager,
        outputs,
        gpus,
        cursor_theme,
        active: true,
        reset_buffers_each_frame,
        outputs_dirty: false,
        ctrl_down: false,
        alt_down: false,
    }));

    {
        let runtime = Rc::clone(&runtime);
        event_loop.handle().insert_source(
            drm_notifier,
            move |event, metadata, _data| match event {
                DrmEvent::VBlank(crtc) => {
                    runtime.borrow_mut().handle_vblank(crtc);
                    if metadata.is_none() {
                        warn!("drm vblank arrived without metadata");
                    }
                }
                DrmEvent::Error(error) => {
                    error!(?error, "drm device notifier reported an error");
                }
            },
        )?;
    }

    {
        let runtime = Rc::clone(&runtime);
        event_loop
            .handle()
            .insert_source(Timer::immediate(), move |_, _, data| {
                runtime.borrow_mut().render(data);
                TimeoutAction::ToDuration(runtime.borrow().min_refresh_interval())
            })?;
    }

    {
        let runtime = Rc::clone(&runtime);
        event_loop
            .handle()
            .insert_source(udev_backend, move |event, _, data| {
                runtime.borrow_mut().outputs_dirty = true;
                data.state.request_render();
                match event {
                    UdevEvent::Added { device_id, path } => {
                        info!(?device_id, ?path, "detected drm device");
                    }
                    UdevEvent::Changed { device_id } => {
                        info!(?device_id, "drm device changed");
                    }
                    UdevEvent::Removed { device_id } => {
                        warn!(?device_id, "drm device removed");
                    }
                }
            })?;
    }

    let mut libinput_context =
        Libinput::new_with_udev::<LibinputSessionInterface<LibSeatSession>>(session.clone().into());
    libinput_context
        .udev_assign_seat(&seat_name)
        .map_err(|_| format!("failed to assign libinput seat {seat_name}"))?;

    let session_for_input = Rc::new(RefCell::new(session.clone()));
    let libinput_backend = LibinputInputBackend::new(libinput_context.clone());
    let runtime_for_input = Rc::clone(&runtime);
    event_loop
        .handle()
        .insert_source(libinput_backend, move |event, _, data| {
            match &event {
                InputEvent::DeviceAdded { device } => {
                    if device.has_capability(DeviceCapability::Keyboard) {
                        info!(name = ?device.name(), "keyboard added");
                    }
                }
                InputEvent::DeviceRemoved { device } => {
                    if device.has_capability(DeviceCapability::Keyboard) {
                        info!(name = ?device.name(), "keyboard removed");
                    }
                }
                _ => {}
            }

            if let InputEvent::Keyboard { event, .. } = &event {
                let key_code = event.key_code().raw();
                let key_state = event.state();
                if runtime_for_input.borrow_mut().handle_keyboard_shortcut(
                    key_code,
                    key_state,
                    &mut session_for_input.borrow_mut(),
                    data,
                ) {
                    return;
                }
            }

            data.state.process_input_event(event);
            data.state.request_render();
        })?;

    {
        let runtime = Rc::clone(&runtime);
        event_loop
            .handle()
            .insert_source(notifier, move |event, _, _data| match event {
                SessionEvent::PauseSession => {
                    info!("standalone session paused");
                    libinput_context.suspend();
                    let mut runtime = runtime.borrow_mut();
                    runtime.active = false;
                    runtime.drm_output_manager.pause();
                }
                SessionEvent::ActivateSession => {
                    info!("standalone session activated");
                    if let Err(error) = libinput_context.resume() {
                        error!(?error, "failed to resume libinput context");
                    }
                    let mut runtime = runtime.borrow_mut();
                    if let Err(error) = runtime.drm_output_manager.activate(true) {
                        error!(?error, "failed to reactivate drm output manager");
                    }
                    runtime.active = true;
                    for output in &mut runtime.outputs {
                        output.frame_pending = false;
                        output.last_frame_queued = None;
                        output.missed_vblank_warned = false;
                    }
                    runtime.outputs_dirty = true;
                }
            })?;
    }

    std::env::set_var("WAYLAND_DISPLAY", &data.state.socket_name);
    info!(
        display = %data.state.socket_name.to_string_lossy(),
        "exported WAYLAND_DISPLAY for standalone clients"
    );

    Ok(())
}

fn is_safe_probe_dmabuf_format(format: &Format) -> bool {
    format.modifier == Modifier::Linear
        && matches!(
            format.code,
            Fourcc::Xrgb8888 | Fourcc::Xbgr8888 | Fourcc::Argb8888 | Fourcc::Abgr8888
        )
}

fn function_key_to_vt(key_code: u32) -> Option<i32> {
    // Linux KEY_F1..KEY_F12 are 59..88; Smithay/libinput adds the xkb offset of 8.
    match key_code {
        67..=76 => Some((key_code - 66) as i32),
        95 | 96 => Some((key_code - 83) as i32),
        _ => None,
    }
}

fn capture_screencopy(
    render_state: &mut RenderState,
    request: crate::screencopy::PendingScreencopy,
    renderer: &mut GlesRenderer,
    data: &mut CalloopData,
    frames: &[WindowFrame],
    overview_windows: &[OverviewWindow],
    animation_config: crate::config::AnimationConfig,
    cursor: Option<CaptureCursor>,
) {
    let region = request.region();
    if let Some(mut dmabuf) = request
        .dmabuf()
        .filter(|_| data.state.config.screencopy_dmabuf())
    {
        let captured = render_state
            .capture_scene_into_dmabuf(
                renderer,
                &data.state.space,
                frames,
                &data.state.windows,
                animation_config,
                overview_windows,
                region,
                &mut dmabuf,
                cursor,
            )
            .map_err(|error| format!("{error:?}"));
        request.finish_dmabuf(captured);
    } else {
        let captured = render_state
            .capture_scene_xrgb8888(
                renderer,
                &data.state.space,
                frames,
                &data.state.windows,
                animation_config,
                overview_windows,
                region,
                cursor,
            )
            .map_err(|error| format!("{error:?}"));
        request.finish(captured);
    }
}

fn capture_cursor(
    cursor_theme: &mut TtyCursorTheme,
    cursor_image: &CursorImageStatus,
    pointer_location: Option<Point<f64, Logical>>,
) -> Option<CaptureCursor> {
    let location = pointer_location?;
    match cursor_image {
        CursorImageStatus::Hidden => None,
        CursorImageStatus::Named(icon) => {
            let cursor = cursor_theme.cursor(*icon);
            Some(CaptureCursor {
                buffer: cursor.buffer,
                hotspot: cursor.hotspot,
                location,
            })
        }
        CursorImageStatus::Surface(_) => {
            // The common desktop capture path uses compositor-named cursors.
            // Client-provided cursor surfaces can be added later through a
            // surface-tree cursor render element without changing screencopy.
            None
        }
    }
}

fn render_elements<'a>(
    renderer: &mut TtyRenderer<'a>,
    render_state: &mut RenderState,
    space: &Space<Window>,
    frames: &[WindowFrame],
    windows: &WindowStore,
    animation_config: crate::config::AnimationConfig,
    overview_windows: &[OverviewWindow],
    background: &SolidColorBuffer,
    cursor_theme: &mut TtyCursorTheme,
    cursor_image: &CursorImageStatus,
    pointer_location: Option<Point<f64, Logical>>,
    dnd_icon: Option<&WlSurface>,
) -> Vec<TtyRenderElements<'a>> {
    let mut elements = Vec::new();

    if let Some(location) = pointer_location {
        push_cursor_element(
            renderer,
            &mut elements,
            cursor_theme,
            cursor_image,
            location,
        );
    }

    elements.extend(
        dnd_icon_elements(renderer.as_mut(), dnd_icon, pointer_location)
            .into_iter()
            .map(TtyYawcElement::from)
            .map(TtyRenderElements::from),
    );

    match render_state.tty_scene_elements(
        renderer.as_mut(),
        space,
        frames,
        windows,
        animation_config,
        overview_windows,
        pointer_location,
    ) {
        Ok(scene) => elements.extend(
            scene
                .into_iter()
                .map(TtyYawcElement::from)
                .map(TtyRenderElements::from),
        ),
        Err(error) => warn!(?error, "failed to build standalone scene elements"),
    }

    let background = SolidColorRenderElement::from_buffer(
        background,
        Point::<i32, Physical>::from((0, 0)),
        RendererScale::from(1.0_f64),
        1.0,
        Kind::Unspecified,
    );
    elements.push(TtyRenderElements::from(background));

    elements
}

fn push_cursor_element<'a>(
    renderer: &mut TtyRenderer<'a>,
    elements: &mut Vec<TtyRenderElements<'a>>,
    cursor_theme: &mut TtyCursorTheme,
    cursor_image: &CursorImageStatus,
    location: Point<f64, Logical>,
) {
    match cursor_image {
        CursorImageStatus::Hidden => {}
        CursorImageStatus::Named(icon) => {
            let cursor = cursor_theme.cursor(*icon);
            match MemoryRenderBufferRenderElement::from_buffer(
                renderer,
                Point::<f64, Physical>::from((
                    location.x - cursor.hotspot.x as f64,
                    location.y - cursor.hotspot.y as f64,
                )),
                &cursor.buffer,
                None,
                None,
                None,
                Kind::Cursor,
            ) {
                Ok(cursor) => elements.push(TtyRenderElements::from(cursor)),
                Err(error) => warn!(?error, "failed to upload standalone cursor"),
            }
        }
        CursorImageStatus::Surface(surface) => {
            push_cursor_surface_element(renderer, elements, surface, location);
        }
    }
}

fn push_cursor_surface_element<'a>(
    renderer: &mut TtyRenderer<'a>,
    elements: &mut Vec<TtyRenderElements<'a>>,
    surface: &WlSurface,
    location: Point<f64, Logical>,
) {
    let result = compositor::with_states(surface, |states| {
        let hotspot = states
            .data_map
            .get::<CursorImageSurfaceData>()
            .map(|data| data.lock().unwrap().hotspot)
            .unwrap_or_default();
        WaylandSurfaceRenderElement::from_surface(
            renderer,
            surface,
            states,
            Point::<f64, Physical>::from((
                location.x - hotspot.x as f64,
                location.y - hotspot.y as f64,
            )),
            1.0,
            Kind::Cursor,
        )
    });

    match result {
        Ok(Some(cursor)) => elements.push(TtyRenderElements::from(cursor)),
        Ok(None) => {}
        Err(error) => warn!(?error, "failed to render client cursor surface"),
    }
}

#[derive(Clone)]
struct TtyCursor {
    buffer: MemoryRenderBuffer,
    hotspot: Point<i32, Physical>,
}

struct TtyCursorTheme {
    theme: xcursor::CursorTheme,
    size: u32,
    fallback: TtyCursor,
    cache: HashMap<CursorIcon, TtyCursor>,
}

impl TtyCursorTheme {
    fn load() -> Self {
        let theme_name = std::env::var("XCURSOR_THEME")
            .ok()
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "default".to_string());
        let size = std::env::var("XCURSOR_SIZE")
            .ok()
            .and_then(|size| size.parse::<u32>().ok())
            .filter(|size| *size > 0)
            .unwrap_or(24);

        info!(theme = %theme_name, size, "loading standalone cursor theme");

        Self {
            theme: xcursor::CursorTheme::load(&theme_name),
            size,
            fallback: fallback_cursor(),
            cache: HashMap::new(),
        }
    }

    fn cursor(&mut self, icon: CursorIcon) -> TtyCursor {
        if let Some(cursor) = self.cache.get(&icon) {
            return cursor.clone();
        }

        let cursor = self
            .load_icon(icon)
            .unwrap_or_else(|| self.fallback.clone());
        self.cache.insert(icon, cursor.clone());
        cursor
    }

    fn load_icon(&self, icon: CursorIcon) -> Option<TtyCursor> {
        let names = std::iter::once(icon.name()).chain(icon.alt_names().iter().copied());

        for name in names {
            let Some(path) = self.theme.load_icon(name) else {
                continue;
            };
            match load_xcursor_file(&path, self.size) {
                Ok(cursor) => return Some(cursor),
                Err(error) => warn!(
                    ?error,
                    cursor = name,
                    path = %path.display(),
                    "failed to load cursor image"
                ),
            }
        }

        warn!(?icon, "cursor theme does not provide shape; using fallback");
        None
    }
}

fn load_xcursor_file(
    path: &std::path::Path,
    desired_size: u32,
) -> Result<TtyCursor, Box<dyn std::error::Error>> {
    let bytes = fs::read(path)?;
    let images = xcursor::parser::parse_xcursor(&bytes).ok_or("failed to parse xcursor file")?;
    let image = images
        .iter()
        .min_by_key(|image| image.size.abs_diff(desired_size))
        .ok_or("xcursor file contained no images")?;

    let buffer = MemoryRenderBuffer::from_slice(
        &image.pixels_rgba,
        Fourcc::Abgr8888,
        (image.width as i32, image.height as i32),
        1,
        Transform::Normal,
        None,
    );

    Ok(TtyCursor {
        buffer,
        hotspot: Point::from((image.xhot as i32, image.yhot as i32)),
    })
}

fn fallback_cursor() -> TtyCursor {
    let width = 32usize;
    let height = 32usize;
    let mut pixels = vec![0u8; width * height * 4];
    let outline = [
        (3.0, 2.0),
        (3.0, 25.0),
        (9.0, 19.0),
        (13.0, 30.0),
        (18.0, 28.0),
        (14.0, 18.0),
        (24.0, 18.0),
    ];
    let fill = [
        (6.0, 7.0),
        (6.0, 19.0),
        (10.0, 15.0),
        (14.0, 26.0),
        (15.0, 25.0),
        (11.0, 14.0),
        (18.0, 14.0),
    ];

    for y in 0..height {
        for x in 0..width {
            let px = x as f64 + 0.5;
            let py = y as f64 + 0.5;
            let color = if point_in_polygon(px, py, &fill) {
                Some([248, 250, 252, 255])
            } else if point_in_polygon(px, py, &outline) {
                Some([12, 15, 20, 255])
            } else {
                None
            };

            if let Some([r, g, b, a]) = color {
                let idx = (y * width + x) * 4;
                pixels[idx] = r;
                pixels[idx + 1] = g;
                pixels[idx + 2] = b;
                pixels[idx + 3] = a;
            }
        }
    }

    let buffer = MemoryRenderBuffer::from_slice(
        &pixels,
        Fourcc::Abgr8888,
        (width as i32, height as i32),
        1,
        Transform::Normal,
        None,
    );

    TtyCursor {
        buffer,
        hotspot: Point::from((3, 2)),
    }
}

fn point_in_polygon(x: f64, y: f64, points: &[(f64, f64)]) -> bool {
    let mut inside = false;
    let mut previous = points.len() - 1;

    for current in 0..points.len() {
        let (current_x, current_y) = points[current];
        let (previous_x, previous_y) = points[previous];
        let crosses = (current_y > y) != (previous_y > y);
        if crosses {
            let intersection_x =
                (previous_x - current_x) * (y - current_y) / (previous_y - current_y) + current_x;
            if x < intersection_x {
                inside = !inside;
            }
        }
        previous = current;
    }

    inside
}

fn select_drm_device(udev_backend: &UdevBackend) -> Result<PathBuf, Box<dyn std::error::Error>> {
    if let Ok(path) = std::env::var("YAWC_DRM_DEVICE") {
        return Ok(PathBuf::from(path));
    }

    udev_backend
        .device_list()
        .next()
        .map(|(_, path)| path.to_path_buf())
        .ok_or_else(|| "no drm device available for standalone backend".into())
}

fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .map(|value| {
            matches!(
                value.as_str(),
                "1" | "true" | "TRUE" | "yes" | "YES" | "on" | "ON"
            )
        })
        .unwrap_or(false)
}

fn select_connectors_and_modes(
    drm_device: &DrmDevice,
    configs: &[OutputConfig],
) -> Result<
    Vec<(
        connector::Info,
        crtc::Handle,
        smithay::reexports::drm::control::Mode,
    )>,
    Box<dyn std::error::Error>,
> {
    let resources = drm_device.resource_handles()?;
    let mut used_crtcs = HashSet::new();
    let mut outputs = Vec::new();
    let mut connected = resources
        .connectors()
        .iter()
        .filter_map(|connector| drm_device.get_connector(*connector, true).ok())
        .filter(|info| info.state() == connector::State::Connected && !info.modes().is_empty())
        .filter(|info| {
            output_config(configs, &connector_name(info))
                .and_then(|config| config.enabled)
                .unwrap_or(true)
        })
        .collect::<Vec<_>>();
    connected.sort_by_key(|info| {
        let name = connector_name(info);
        let config = output_config(configs, &name);
        (
            !config.map(|config| config.primary).unwrap_or(false),
            config.and_then(|config| config.x).unwrap_or(i32::MAX),
            config.and_then(|config| config.y).unwrap_or(i32::MAX),
            name,
        )
    });

    for connector_info in connected {
        let config = output_config(configs, &connector_name(&connector_info));
        let Some(mode) = select_best_mode(&connector_info, config.and_then(|config| config.mode))
        else {
            continue;
        };
        let Some(crtc) =
            select_crtc_for_connector(drm_device, &resources, &connector_info, &used_crtcs)?
        else {
            warn!(
                connector = %connector_name(&connector_info),
                "connected connector has no unused crtc"
            );
            continue;
        };
        used_crtcs.insert(crtc);
        outputs.push((connector_info, crtc, mode));
    }

    if outputs.is_empty() {
        return Err("no connected drm connector with a valid mode found".into());
    }

    Ok(outputs)
}

fn select_crtc_for_connector(
    drm_device: &DrmDevice,
    resources: &smithay::reexports::drm::control::ResourceHandles,
    connector_info: &connector::Info,
    used_crtcs: &HashSet<crtc::Handle>,
) -> Result<Option<crtc::Handle>, Box<dyn std::error::Error>> {
    let mut encoders = Vec::new();
    if let Some(encoder) = connector_info.current_encoder() {
        encoders.push(encoder);
    }
    for encoder in connector_info.encoders().iter().copied() {
        if !encoders.contains(&encoder) {
            encoders.push(encoder);
        }
    }

    for encoder_handle in encoders {
        let encoder_info = drm_device.get_encoder(encoder_handle)?;
        if let Some(crtc) = encoder_info
            .crtc()
            .filter(|crtc| !used_crtcs.contains(crtc))
        {
            return Ok(Some(crtc));
        }

        if let Some(crtc) = resources
            .filter_crtcs(encoder_info.possible_crtcs())
            .into_iter()
            .find(|crtc| !used_crtcs.contains(crtc))
        {
            return Ok(Some(crtc));
        }
    }

    Ok(None)
}

fn connector_name(connector_info: &connector::Info) -> String {
    format!(
        "{}-{}",
        connector_info.interface().as_str(),
        connector_info.interface_id()
    )
}

fn output_config<'a>(configs: &'a [OutputConfig], name: &str) -> Option<&'a OutputConfig> {
    configs.iter().find(|config| config.name == name)
}

fn configured_or_auto_location(
    config: Option<&OutputConfig>,
    next_auto_x: &mut i32,
    logical_size: Size<i32, Logical>,
) -> Point<i32, Logical> {
    if let Some(config) = config {
        if let (Some(x), Some(y)) = (config.x, config.y) {
            return (x, y).into();
        }
    }

    let location = (*next_auto_x, 0).into();
    *next_auto_x += logical_size.w.max(1);
    location
}

fn select_best_mode(
    connector_info: &connector::Info,
    configured: Option<OutputModeConfig>,
) -> Option<smithay::reexports::drm::control::Mode> {
    if let Some(configured) = configured {
        if let Some(mode) = connector_info.modes().iter().copied().find(|mode| {
            let (width, height) = mode.size();
            width as i32 == configured.width
                && height as i32 == configured.height
                && configured
                    .refresh_millihz
                    .map(|refresh| {
                        ((mode.vrefresh() as i32).saturating_mul(1000) - refresh).abs() <= 1000
                    })
                    .unwrap_or(true)
        }) {
            return Some(mode);
        }
        warn!(
            connector = %connector_name(connector_info),
            width = configured.width,
            height = configured.height,
            refresh_millihz = configured.refresh_millihz,
            "configured output mode is unavailable; using best mode"
        );
    }

    connector_info.modes().iter().copied().max_by_key(|mode| {
        let (width, height) = mode.size();
        let area = width as u64 * height as u64;
        (area, mode.vrefresh())
    })
}
