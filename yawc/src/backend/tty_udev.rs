use std::{
    cell::RefCell,
    collections::HashMap,
    fs,
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};

use smithay::{
    backend::{
        allocator::{
            gbm::{GbmAllocator, GbmBufferFlags, GbmDevice},
            Fourcc,
        },
        drm::{
            compositor::FrameFlags,
            exporter::gbm::GbmFramebufferExporter,
            output::{DrmOutput, DrmOutputManager, DrmOutputRenderElements},
            DrmDevice, DrmDeviceFd, DrmEvent, DrmNode, Planes,
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
            Color32F, ImportDma, ImportMemWl, RendererSuper,
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
    render::{dnd_icon_elements, RenderState, YawcRenderElements},
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
    drm_output: TtyOutput,
    gpus: GpuManager<TtyRenderBackend>,
    render_state: RenderState,
    background: SolidColorBuffer,
    cursor_theme: TtyCursorTheme,
    refresh_interval: Duration,
    active: bool,
    frame_pending: bool,
    last_frame_queued: Option<Instant>,
    missed_vblank_warned: bool,
    reset_buffers_each_frame: bool,
    ctrl_down: bool,
    alt_down: bool,
}

impl TtyRuntime {
    fn render(&mut self, data: &mut CalloopData) {
        if !self.active {
            return;
        }

        if self.frame_pending {
            let missed_vblank = self
                .last_frame_queued
                .map(|queued_at| queued_at.elapsed() > Duration::from_millis(250))
                .unwrap_or(true);

            if !missed_vblank {
                return;
            }

            if !self.missed_vblank_warned {
                warn!("forcing standalone render after missed drm vblank");
                self.missed_vblank_warned = true;
            }

            if let Err(error) = self.drm_output.frame_submitted() {
                warn!(?error, "failed to recover from missed drm vblank");
            }
            self.frame_pending = false;
            self.last_frame_queued = None;
        }

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
        data.state.reload_config_if_changed();
        data.state.finish_close_animations();
        let animation_config = data.state.config.animations();
        let controls_mode = data.state.config.window_controls();
        let frames = data
            .state
            .windows
            .frames(&data.state.space, animation_config, controls_mode);
        let elements = render_elements(
            &mut renderer,
            &mut self.render_state,
            &data.state.space,
            &frames,
            &data.state.windows,
            animation_config,
            &self.background,
            &mut self.cursor_theme,
            &cursor_image,
            pointer_location,
            data.state.dnd_icon.as_ref(),
        );

        if self.reset_buffers_each_frame {
            self.drm_output.reset_buffers();
        }

        match self.drm_output.render_frame(
            &mut renderer,
            &elements,
            [0.06, 0.09, 0.11, 1.0],
            // The early standalone backend should be predictable before it is clever:
            // force a fully composited frame instead of allowing direct scanout planes.
            FrameFlags::empty(),
        ) {
            Ok(result) => {
                if !result.is_empty {
                    if let Err(error) = self.drm_output.queue_frame(()) {
                        warn!(?error, "failed to queue drm frame");
                    } else {
                        self.frame_pending = true;
                        self.last_frame_queued = Some(Instant::now());
                        self.missed_vblank_warned = false;
                    }
                }
            }
            Err(error) => {
                warn!(?error, "standalone render_frame failed");
            }
        }
        drop(elements);

        if data.state.screencopy_state.has_pending() {
            let requests = data.state.screencopy_state.take_pending();
            for request in requests {
                let region = request.region();
                let captured = self
                    .render_state
                    .capture_scene_xrgb8888(
                        renderer.as_mut(),
                        &data.state.space,
                        &frames,
                        &data.state.windows,
                        animation_config,
                        region,
                    )
                    .map_err(|error| format!("{error:?}"));
                request.finish(captured);
            }
        }

        let output = self.render_state.output().clone();
        data.state.space.elements().for_each(|window| {
            window.send_frame(
                &output,
                data.state.start_time.elapsed(),
                Some(Duration::ZERO),
                |_, _| Some(output.clone()),
            );
        });
        if let Some(icon) = data.state.dnd_icon.as_ref() {
            send_frames_surface_tree(
                icon,
                &output,
                data.state.start_time.elapsed(),
                Some(Duration::ZERO),
                |_, _| Some(output.clone()),
            );
        }

        data.state.space.refresh();
        data.state.prune_windows();
        data.state.popups.cleanup();
        let _ = data.display_handle.flush_clients();
    }

    fn handle_vblank(&mut self) {
        self.frame_pending = false;
        self.last_frame_queued = None;
        self.missed_vblank_warned = false;
        if let Err(error) = self.drm_output.frame_submitted() {
            warn!(?error, "failed to mark drm frame as submitted");
        }
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

        // Ctrl+Alt+Backspace or Ctrl+Alt+Esc: emergency compositor stop.
        if matches!(key_code, 22 | 9) {
            warn!("stopping compositor from emergency keyboard shortcut");
            data.state.loop_signal.stop();
            return true;
        }

        false
    }
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
    data.state.shm_state.update_formats(renderer.shm_formats());

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
        renderer.dmabuf_formats(),
    );

    let (connector_info, crtc, mode) = select_connector_and_mode(drm_output_manager.device())?;
    let output_size = Size::<i32, Physical>::from((mode.size().0 as i32, mode.size().1 as i32));
    let background = SolidColorBuffer::new(
        Size::<i32, Logical>::from((output_size.w, output_size.h)),
        Color32F::from([0.10, 0.13, 0.16, 1.0]),
    );
    let cursor_theme = TtyCursorTheme::load();
    let refresh_hz = mode.vrefresh().max(1);
    let refresh_millihz = (refresh_hz as i32).saturating_mul(1000);
    info!(
        width = output_size.w,
        height = output_size.h,
        refresh_hz,
        "selected drm mode for standalone output"
    );
    let render_state = RenderState::new_standalone(
        &data.display_handle,
        &mut data.state.space,
        output_size,
        refresh_millihz,
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
            &mut renderer,
            &render_elements,
        )
        .map_err(|error| format!("failed to initialize drm output: {error}"))?;

    let refresh_interval = Duration::from_nanos(1_000_000_000 / refresh_hz.max(1) as u64);
    let reset_buffers_each_frame = std::env::var("YAWC_DRM_RESET_BUFFERS_EACH_FRAME")
        .map(|value| value != "0")
        .unwrap_or(false);
    if reset_buffers_each_frame {
        warn!("resetting drm buffers before each frame as a damage-corruption workaround");
    }

    let runtime = Rc::new(RefCell::new(TtyRuntime {
        node,
        drm_output_manager,
        drm_output,
        gpus,
        render_state,
        background,
        cursor_theme,
        refresh_interval,
        active: true,
        frame_pending: false,
        last_frame_queued: None,
        missed_vblank_warned: false,
        reset_buffers_each_frame,
        ctrl_down: false,
        alt_down: false,
    }));

    {
        let runtime = Rc::clone(&runtime);
        event_loop.handle().insert_source(
            drm_notifier,
            move |event, metadata, _data| match event {
                DrmEvent::VBlank(_) => {
                    runtime.borrow_mut().handle_vblank();
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
                TimeoutAction::ToDuration(runtime.borrow().refresh_interval)
            })?;
    }

    event_loop
        .handle()
        .insert_source(udev_backend, |event, _, _data| match event {
            UdevEvent::Added { device_id, path } => {
                info!(?device_id, ?path, "detected drm device");
            }
            UdevEvent::Changed { device_id } => {
                info!(?device_id, "drm device changed");
            }
            UdevEvent::Removed { device_id } => {
                warn!(?device_id, "drm device removed");
            }
        })?;

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
                    runtime.frame_pending = false;
                    runtime.last_frame_queued = None;
                    runtime.missed_vblank_warned = false;
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

fn function_key_to_vt(key_code: u32) -> Option<i32> {
    // Linux KEY_F1..KEY_F12 are 59..88; Smithay/libinput adds the xkb offset of 8.
    match key_code {
        67..=76 => Some((key_code - 66) as i32),
        95 | 96 => Some((key_code - 83) as i32),
        _ => None,
    }
}

fn render_elements<'a>(
    renderer: &mut TtyRenderer<'a>,
    render_state: &mut RenderState,
    space: &Space<Window>,
    frames: &[WindowFrame],
    windows: &WindowStore,
    animation_config: crate::config::AnimationConfig,
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

fn select_connector_and_mode(
    drm_device: &DrmDevice,
) -> Result<
    (
        connector::Info,
        crtc::Handle,
        smithay::reexports::drm::control::Mode,
    ),
    Box<dyn std::error::Error>,
> {
    let resources = drm_device.resource_handles()?;
    let connector_info = resources
        .connectors()
        .iter()
        .filter_map(|connector| drm_device.get_connector(*connector, true).ok())
        .find(|info| info.state() == connector::State::Connected && !info.modes().is_empty())
        .ok_or("no connected drm connector with a valid mode found")?;

    let encoder_handle = connector_info
        .current_encoder()
        .or_else(|| connector_info.encoders().first().copied())
        .ok_or("connected connector has no encoder")?;
    let encoder_info = drm_device.get_encoder(encoder_handle)?;
    let crtc = encoder_info
        .crtc()
        .or_else(|| {
            resources
                .filter_crtcs(encoder_info.possible_crtcs())
                .first()
                .copied()
        })
        .ok_or("encoder has no usable crtc")?;
    let mode = select_best_mode(&connector_info).ok_or("connected connector reported no modes")?;

    Ok((connector_info, crtc, mode))
}

fn select_best_mode(
    connector_info: &connector::Info,
) -> Option<smithay::reexports::drm::control::Mode> {
    connector_info.modes().iter().copied().max_by_key(|mode| {
        let (width, height) = mode.size();
        let area = width as u64 * height as u64;
        (area, mode.vrefresh())
    })
}
