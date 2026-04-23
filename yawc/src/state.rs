use std::{
    collections::HashMap,
    ffi::OsString,
    process::Command,
    sync::Arc,
    time::{Duration, Instant},
};

use smithay::{
    backend::allocator::Format,
    desktop::{PopupManager, Space, Window, WindowSurfaceType},
    input::{
        pointer::{CursorIcon, CursorImageStatus},
        Seat, SeatState,
    },
    output::Output,
    reexports::{
        calloop::{generic::Generic, EventLoop, Interest, LoopSignal, Mode, PostAction},
        wayland_protocols::xdg::shell::server::xdg_toplevel,
        wayland_server::{
            backend::{ClientData, ClientId, DisconnectReason},
            protocol::{wl_output::WlOutput, wl_surface::WlSurface},
            Client, Display, DisplayHandle, Resource,
        },
    },
    utils::{Logical, Point, Rectangle, Size, SERIAL_COUNTER},
    wayland::{
        compositor::{CompositorClientState, CompositorState},
        cursor_shape::CursorShapeManagerState,
        dmabuf::{DmabufFeedbackBuilder, DmabufGlobal, DmabufState},
        output::OutputManagerState,
        selection::data_device::DataDeviceState,
        shell::xdg::{decoration::XdgDecorationState, XdgShellState},
        shm::ShmState,
        socket::ListeningSocketSource,
        viewporter::ViewporterState,
    },
};
#[cfg(feature = "xwayland")]
use smithay::{
    wayland::{
        xwayland_keyboard_grab::XWaylandKeyboardGrabState, xwayland_shell::XWaylandShellState,
    },
    xwayland::X11Wm,
};
use tracing::{info, warn};

use crate::{
    config::{Config, KeyboardConfig},
    cursor::CursorShape,
    focus::FocusTarget,
    screencopy::ScreencopyState,
    window::{SnapSide, WindowStore},
    CalloopData,
};

const GRACEFUL_SHUTDOWN_TIMEOUT: Duration = Duration::from_millis(2500);
const OVERVIEW_ANIMATION_DURATION: Duration = Duration::from_millis(240);
const OVERVIEW_SELECTION_ANIMATION_DURATION: Duration = Duration::from_millis(120);

#[derive(Clone)]
pub struct TitlebarClick {
    pub window: Window,
    pub time_msec: u32,
    pub location: Point<f64, Logical>,
}

#[derive(Clone)]
pub struct OverviewWindow {
    pub window: Window,
    pub source: Rectangle<i32, Logical>,
    pub target: Rectangle<i32, Logical>,
    pub active: bool,
    pub selected: bool,
    pub selection_alpha: f32,
    pub progress: f64,
}

#[derive(Clone)]
struct OverviewLayoutItem {
    window: Window,
    target: Rectangle<i32, Logical>,
}

pub struct Yawc {
    pub start_time: Instant,
    pub socket_name: OsString,
    pub display_handle: DisplayHandle,
    pub space: Space<Window>,
    pub loop_signal: LoopSignal,
    pub windows: WindowStore,
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    #[cfg(feature = "xwayland")]
    pub xwayland_shell_state: XWaylandShellState,
    #[cfg(feature = "xwayland")]
    pub xwayland_keyboard_grab_state: XWaylandKeyboardGrabState,
    #[cfg(feature = "xwayland")]
    pub xwm: Option<X11Wm>,
    #[cfg(feature = "xwayland")]
    pub xwayland_display: Option<u32>,
    #[cfg(feature = "xwayland")]
    pub xwayland_failed: bool,
    pub xdg_decoration_state: XdgDecorationState,
    pub shm_state: ShmState,
    pub output_manager_state: OutputManagerState,
    pub viewporter_state: ViewporterState,
    pub cursor_shape_state: CursorShapeManagerState,
    pub seat_state: SeatState<Self>,
    pub data_device_state: DataDeviceState,
    pub dmabuf_state: DmabufState,
    pub dmabuf_formats: Vec<Format>,
    dmabuf_global: Option<DmabufGlobal>,
    pub screencopy_state: ScreencopyState,
    pub popups: PopupManager,
    pub seat: Seat<Self>,
    pub pending_cursor: CursorShape,
    pub compositor_cursor: Option<CursorShape>,
    pub cursor_image: CursorImageStatus,
    pub dnd_icon: Option<WlSurface>,
    pub config: Config,
    pub titlebar_right_press: Option<Window>,
    pub last_titlebar_click: Option<TitlebarClick>,
    pub snap_memory: HashMap<String, SnapSide>,
    pub overview_active: bool,
    pub overview_visible: bool,
    pub overview_closing: bool,
    pub overview_started_at: Instant,
    pub super_overview_armed: bool,
    overview_layout: Vec<OverviewLayoutItem>,
    overview_selection: Option<Window>,
    overview_previous_selection: Option<Window>,
    overview_hovered: Option<Window>,
    overview_previous_hovered: Option<Window>,
    overview_selection_started_at: Instant,
    pub render_requested: bool,
    applied_keyboard_config: Option<KeyboardConfig>,
    shutdown_requested: bool,
    shutdown_started_at: Option<Instant>,
}

impl Yawc {
    pub fn new(event_loop: &mut EventLoop<CalloopData>, display: Display<Self>) -> Self {
        let start_time = Instant::now();
        let display_handle = display.handle();
        let compositor_state = CompositorState::new::<Self>(&display_handle);
        let xdg_shell_state = XdgShellState::new::<Self>(&display_handle);
        #[cfg(feature = "xwayland")]
        let xwayland_shell_state = XWaylandShellState::new::<Self>(&display_handle);
        #[cfg(feature = "xwayland")]
        let xwayland_keyboard_grab_state = XWaylandKeyboardGrabState::new::<Self>(&display_handle);
        let xdg_decoration_state = XdgDecorationState::new::<Self>(&display_handle);
        let shm_state = ShmState::new::<Self>(&display_handle, vec![]);
        let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(&display_handle);
        let viewporter_state = ViewporterState::new::<Self>(&display_handle);
        let cursor_shape_state = CursorShapeManagerState::new::<Self>(&display_handle);
        let mut seat_state = SeatState::new();
        let data_device_state = DataDeviceState::new::<Self>(&display_handle);
        let dmabuf_state = DmabufState::new();
        let screencopy_state = ScreencopyState::new(&display_handle);
        let popups = PopupManager::default();
        let windows = WindowStore::default();

        let mut seat: Seat<Self> = seat_state.new_wl_seat(&display_handle, "seat0");
        seat.add_keyboard(Default::default(), 200, 25).unwrap();
        seat.add_pointer();

        let space = Space::default();
        let socket_name = Self::init_wayland_listener(display, event_loop);
        let loop_signal = event_loop.get_signal();
        let config = Config::load_or_create();

        let mut state = Self {
            start_time,
            socket_name,
            display_handle,
            space,
            loop_signal,
            windows,
            compositor_state,
            xdg_shell_state,
            #[cfg(feature = "xwayland")]
            xwayland_shell_state,
            #[cfg(feature = "xwayland")]
            xwayland_keyboard_grab_state,
            #[cfg(feature = "xwayland")]
            xwm: None,
            #[cfg(feature = "xwayland")]
            xwayland_display: None,
            #[cfg(feature = "xwayland")]
            xwayland_failed: false,
            xdg_decoration_state,
            shm_state,
            output_manager_state,
            viewporter_state,
            cursor_shape_state,
            seat_state,
            data_device_state,
            dmabuf_state,
            dmabuf_formats: Vec::new(),
            dmabuf_global: None,
            screencopy_state,
            popups,
            seat,
            pending_cursor: CursorShape::Default,
            compositor_cursor: None,
            cursor_image: CursorImageStatus::Named(CursorIcon::Default),
            dnd_icon: None,
            config,
            titlebar_right_press: None,
            last_titlebar_click: None,
            snap_memory: HashMap::new(),
            overview_active: false,
            overview_visible: false,
            overview_closing: false,
            overview_started_at: Instant::now(),
            super_overview_armed: false,
            overview_layout: Vec::new(),
            overview_selection: None,
            overview_previous_selection: None,
            overview_hovered: None,
            overview_previous_hovered: None,
            overview_selection_started_at: Instant::now(),
            render_requested: true,
            applied_keyboard_config: None,
            shutdown_requested: false,
            shutdown_started_at: None,
        };
        state.apply_keyboard_config();
        state
    }

    pub fn init_dmabuf_global(&mut self, formats: Vec<Format>, main_device: Option<libc::dev_t>) {
        if formats.is_empty() || self.dmabuf_global.is_some() {
            return;
        }

        self.dmabuf_formats = formats.clone();
        let display_handle = self.display_handle.clone();
        let filter_display = display_handle.clone();
        let hide_portal_bridge = !self.config.screencopy_dmabuf();
        let filter = move |client: &Client| {
            should_show_dmabuf_global(client, &filter_display, hide_portal_bridge)
        };
        let global = if let Some(main_device) = main_device {
            match DmabufFeedbackBuilder::new(main_device, formats.clone()).build() {
                Ok(default_feedback) => self
                    .dmabuf_state
                    .create_global_with_filter_and_default_feedback::<Self, _>(
                        &display_handle,
                        &default_feedback,
                        filter,
                    ),
                Err(error) => {
                    warn!(
                        ?error,
                        "failed to build dmabuf feedback; falling back to v3 linux-dmabuf"
                    );
                    let filter_display = display_handle.clone();
                    self.dmabuf_state.create_global_with_filter::<Self, _>(
                        &display_handle,
                        formats,
                        move |client| {
                            should_show_dmabuf_global(client, &filter_display, hide_portal_bridge)
                        },
                    )
                }
            }
        } else {
            warn!("missing DRM device id; creating v3 linux-dmabuf without feedback");
            self.dmabuf_state
                .create_global_with_filter::<Self, _>(&display_handle, formats, filter)
        };
        self.dmabuf_global = Some(global);
    }

    fn init_wayland_listener(
        display: Display<Self>,
        event_loop: &mut EventLoop<CalloopData>,
    ) -> OsString {
        let listening_socket = ListeningSocketSource::new_auto().unwrap();
        let socket_name = listening_socket.socket_name().to_os_string();
        let loop_handle = event_loop.handle();

        loop_handle
            .insert_source(listening_socket, move |client_stream, _, data| {
                data.display_handle
                    .insert_client(client_stream, Arc::new(ClientState::default()))
                    .unwrap();
            })
            .expect("failed to insert Wayland socket source");

        loop_handle
            .insert_source(
                Generic::new(display, Interest::READ, Mode::Level),
                |_, display, data| {
                    unsafe {
                        display.get_mut().dispatch_clients(&mut data.state).unwrap();
                    }
                    data.state.flush_wayland_clients();
                    Ok(PostAction::Continue)
                },
            )
            .expect("failed to insert Wayland display source");

        info!(
            display = %socket_name.to_string_lossy(),
            "listening for Wayland clients"
        );

        socket_name
    }

    pub fn surface_under(
        &self,
        position: Point<f64, Logical>,
    ) -> Option<(FocusTarget, Point<f64, Logical>)> {
        self.space
            .element_under(position)
            .and_then(|(window, location)| {
                #[cfg(feature = "xwayland")]
                if let Some(surface) = window.x11_surface() {
                    return window
                        .surface_under(position - location.to_f64(), WindowSurfaceType::ALL)
                        .map(|(_, point)| {
                            (
                                FocusTarget::X11(surface.clone()),
                                (point + location).to_f64(),
                            )
                        });
                }

                window
                    .surface_under(position - location.to_f64(), WindowSurfaceType::ALL)
                    .map(|(surface, point)| {
                        (FocusTarget::Wayland(surface), (point + location).to_f64())
                    })
            })
    }

    pub fn send_pending_configures(&mut self) {
        self.space.elements().for_each(|window| {
            if let Some(toplevel) = window.toplevel() {
                toplevel.send_pending_configure();
            }
        });
        self.flush_wayland_clients();
    }

    pub fn flush_wayland_clients(&mut self) {
        if let Err(error) = self.display_handle.flush_clients() {
            warn!(?error, "failed to flush Wayland clients");
        }
    }

    pub fn request_render(&mut self) {
        self.render_requested = true;
    }

    pub fn take_render_requested(&mut self) -> bool {
        std::mem::take(&mut self.render_requested)
    }

    pub fn set_overview_active(&mut self, active: bool) {
        if self.overview_active == active
            && self.overview_visible == active
            && !self.overview_closing
        {
            return;
        }

        if active {
            let layout = self.build_overview_layout(0.0);
            if layout.is_empty() {
                return;
            }
            self.overview_layout = layout
                .into_iter()
                .map(|item| OverviewLayoutItem {
                    window: item.window,
                    target: item.target,
                })
                .collect();
            self.overview_selection = self
                .windows
                .active_window()
                .filter(|window| {
                    self.overview_layout
                        .iter()
                        .any(|item| item.window == *window)
                })
                .or_else(|| self.overview_layout.first().map(|item| item.window.clone()));
            self.overview_previous_selection = None;
            self.overview_hovered = None;
            self.overview_previous_hovered = None;
            self.overview_selection_started_at = Instant::now();
        }

        self.overview_active = active;
        self.overview_visible = true;
        self.overview_closing = !active;
        self.overview_started_at = Instant::now();
        self.super_overview_armed = false;
        self.windows.clear_decoration_pressed();
        self.windows.clear_titlebar_close_pressed();
        self.titlebar_right_press = None;
        self.request_render();
        info!(active, "overview state changed");
    }

    pub fn toggle_overview(&mut self) {
        self.set_overview_active(!self.overview_active);
    }

    pub fn overview_needs_animation_frame(&self) -> bool {
        self.overview_visible
            && (self.overview_started_at.elapsed() < OVERVIEW_ANIMATION_DURATION
                || (self.overview_active
                    && self.overview_selection_started_at.elapsed()
                        < OVERVIEW_SELECTION_ANIMATION_DURATION))
    }

    pub fn finish_overview_animation(&mut self) {
        if self.overview_closing
            && self.overview_started_at.elapsed() >= OVERVIEW_ANIMATION_DURATION
        {
            self.overview_visible = false;
            self.overview_closing = false;
            self.overview_layout.clear();
            self.overview_selection = None;
            self.overview_previous_selection = None;
            self.overview_hovered = None;
            self.overview_previous_hovered = None;
            self.request_render();
        }
    }

    fn overview_progress(&self) -> f64 {
        let raw = if OVERVIEW_ANIMATION_DURATION.is_zero() {
            1.0
        } else {
            (self.overview_started_at.elapsed().as_secs_f64()
                / OVERVIEW_ANIMATION_DURATION.as_secs_f64())
            .clamp(0.0, 1.0)
        };
        let eased = ease_out_cubic(raw);
        if self.overview_closing {
            1.0 - eased
        } else {
            eased
        }
    }

    pub fn overview_windows(&self) -> Vec<OverviewWindow> {
        if !self.overview_visible {
            return Vec::new();
        }

        let progress = self.overview_progress();
        if !self.overview_layout.is_empty() {
            let active_window = self.windows.active_window();
            let mut items = self
                .overview_layout
                .iter()
                .filter(|item| {
                    !self.windows.is_window_minimized(&item.window)
                        && !self.windows.is_window_fullscreen(&item.window)
                        && is_overview_window(&item.window)
                })
                .filter_map(|item| {
                    let source = self.window_visual_rect(&item.window)?;
                    let active = active_window
                        .as_ref()
                        .map(|active| active == &item.window)
                        .unwrap_or(false);
                    let selected = self
                        .overview_selection
                        .as_ref()
                        .map(|selected| selected == &item.window)
                        .unwrap_or(false);
                    let selection_alpha = self.overview_selection_alpha(&item.window, active);
                    Some(OverviewWindow {
                        window: item.window.clone(),
                        source,
                        target: item.target,
                        active,
                        selected,
                        selection_alpha,
                        progress,
                    })
                })
                .collect::<Vec<_>>();
            if self.overview_closing {
                self.sort_overview_items_by_space_order(&mut items);
            }
            return items;
        }

        self.build_overview_layout(progress)
    }

    fn build_overview_layout(&self, progress: f64) -> Vec<OverviewWindow> {
        let Some(output_geometry) = self.placement_output_geometry(None) else {
            return Vec::new();
        };

        let windows = self
            .space
            .elements()
            .filter(|window| {
                !self.windows.is_window_minimized(window)
                    && !self.windows.is_window_fullscreen(window)
                    && is_overview_window(window)
            })
            .filter_map(|window| Some((window.clone(), self.window_visual_rect(window)?)))
            .collect::<Vec<_>>();

        overview_layout(
            windows,
            output_geometry,
            self.windows.active_window(),
            self.overview_selection.as_ref(),
            progress,
        )
    }

    pub fn overview_window_at(&self, location: Point<f64, Logical>) -> Option<Window> {
        self.overview_windows()
            .into_iter()
            .rev()
            .find(|item| item.target.to_f64().contains(location))
            .map(|item| item.window)
    }

    pub fn set_overview_selection_at(&mut self, location: Point<f64, Logical>) {
        let hovered = self.overview_window_at(location);
        self.set_overview_hovered(hovered);
    }

    pub fn select_overview_window(&mut self, window: Window) {
        self.set_overview_hovered(None);
        self.set_overview_selection(Some(window));
    }

    fn set_overview_selection(&mut self, selection: Option<Window>) {
        if self.overview_selection == selection {
            return;
        }
        self.overview_previous_selection = self.overview_selection.clone();
        self.overview_selection = selection;
        self.overview_selection_started_at = Instant::now();
        self.request_render();
    }

    fn set_overview_hovered(&mut self, hovered: Option<Window>) {
        if self.overview_hovered == hovered {
            return;
        }
        self.overview_previous_hovered = self.overview_hovered.clone();
        self.overview_hovered = hovered;
        self.overview_selection_started_at = Instant::now();
        self.request_render();
    }

    fn overview_selection_alpha(&self, window: &Window, active: bool) -> f32 {
        let target = self.overview_window_alpha(
            window,
            active,
            self.overview_selection.as_ref(),
            self.overview_hovered.as_ref(),
        );
        let from = self.overview_window_alpha(
            window,
            active,
            self.overview_previous_selection.as_ref(),
            self.overview_previous_hovered.as_ref(),
        );
        let progress = (self.overview_selection_started_at.elapsed().as_secs_f32()
            / OVERVIEW_SELECTION_ANIMATION_DURATION.as_secs_f32())
        .clamp(0.0, 1.0);
        let eased = ease_out_cubic(progress as f64) as f32;
        from + (target - from) * eased
    }

    fn overview_window_alpha(
        &self,
        window: &Window,
        active: bool,
        selected: Option<&Window>,
        hovered: Option<&Window>,
    ) -> f32 {
        if selected.map(|selected| selected == window).unwrap_or(false)
            || hovered.map(|hovered| hovered == window).unwrap_or(false)
        {
            0.50
        } else if active {
            0.30
        } else {
            0.0
        }
    }

    pub fn activate_overview_selection(&mut self, serial: smithay::utils::Serial) -> bool {
        let selected = self
            .overview_selection
            .clone()
            .or_else(|| self.windows.active_window())
            .or_else(|| {
                self.overview_windows()
                    .first()
                    .map(|item| item.window.clone())
            });
        let Some(window) = selected else {
            return false;
        };
        self.focus_window(&window, serial);
        self.set_overview_hovered(None);
        self.set_overview_selection(Some(window));
        self.set_overview_active(false);
        true
    }

    pub fn move_overview_selection(&mut self, dx: i32, dy: i32) {
        let items = self.overview_windows();
        if items.is_empty() {
            return;
        }

        let active_window = self.windows.active_window();
        let selected = self.overview_selection.as_ref().or(active_window.as_ref());
        let current_index = selected
            .and_then(|selected| items.iter().position(|item| item.window == *selected))
            .unwrap_or(0);
        let current = &items[current_index];
        let current_center = rect_center(current.target);

        let mut best: Option<(usize, f64)> = None;
        for (index, item) in items.iter().enumerate() {
            if index == current_index {
                continue;
            }

            let center = rect_center(item.target);
            let delta_x = center.x - current_center.x;
            let delta_y = center.y - current_center.y;
            if dx < 0 && delta_x >= -1.0 {
                continue;
            }
            if dx > 0 && delta_x <= 1.0 {
                continue;
            }
            if dy < 0 && delta_y >= -1.0 {
                continue;
            }
            if dy > 0 && delta_y <= 1.0 {
                continue;
            }

            let primary = if dx != 0 {
                delta_x.abs()
            } else {
                delta_y.abs()
            };
            let secondary = if dx != 0 {
                delta_y.abs()
            } else {
                delta_x.abs()
            };
            let score = primary * 4.0 + secondary;
            if best
                .map(|(_, best_score)| score < best_score)
                .unwrap_or(true)
            {
                best = Some((index, score));
            }
        }

        let next_index = best
            .map(|(index, _)| index)
            .unwrap_or_else(|| wrap_overview_index(current_index, items.len(), dx, dy));
        self.set_overview_hovered(None);
        self.set_overview_selection(Some(items[next_index].window.clone()));
    }

    fn sort_overview_items_by_space_order(&self, items: &mut [OverviewWindow]) {
        let order = self
            .space
            .elements()
            .enumerate()
            .map(|(index, window)| (window.clone(), index))
            .collect::<HashMap<_, _>>();
        items.sort_by_key(|item| order.get(&item.window).copied().unwrap_or(usize::MAX));
    }

    pub fn focus_window(&mut self, window: &Window, serial: smithay::utils::Serial) {
        self.space.raise_element(window, true);
        self.windows.activate_window(window);
        if let (Some(keyboard), Some(target)) = (
            self.seat.get_keyboard(),
            self.focus_target_for_window(window),
        ) {
            keyboard.set_focus(self, Some(target), serial);
        }
        self.send_pending_configures();
        self.request_render();
    }

    pub fn output_at(&self, location: Point<f64, Logical>) -> Option<Output> {
        self.space
            .outputs()
            .find(|output| {
                self.space
                    .output_geometry(output)
                    .map(|geometry| geometry.to_f64().contains(location))
                    .unwrap_or(false)
            })
            .cloned()
    }

    pub fn output_for_window(&self, window: &Window) -> Option<Output> {
        let rect = self.window_visual_rect(window)?;
        self.output_for_rect(rect).cloned()
    }

    pub fn focus_target_for_window(&self, window: &Window) -> Option<FocusTarget> {
        if let Some(toplevel) = window.toplevel() {
            return Some(FocusTarget::Wayland(toplevel.wl_surface().clone()));
        }

        #[cfg(feature = "xwayland")]
        if let Some(surface) = window.x11_surface() {
            return Some(FocusTarget::X11(surface.clone()));
        }

        None
    }

    fn output_for_rect(&self, rect: Rectangle<i32, Logical>) -> Option<&Output> {
        let center = Point::<i32, Logical>::from((
            rect.loc.x + rect.size.w / 2,
            rect.loc.y + rect.size.h / 2,
        ));

        self.space
            .outputs()
            .find(|output| {
                self.space
                    .output_geometry(output)
                    .map(|geometry| geometry.contains(center))
                    .unwrap_or(false)
            })
            .or_else(|| {
                self.space.outputs().max_by_key(|output| {
                    self.space
                        .output_geometry(output)
                        .and_then(|geometry| geometry.intersection(rect))
                        .map(|overlap| overlap.size.w.max(0) * overlap.size.h.max(0))
                        .unwrap_or(0)
                })
            })
    }

    pub fn output_geometry_at(
        &self,
        location: Point<f64, Logical>,
    ) -> Option<Rectangle<i32, Logical>> {
        self.output_at(location)
            .and_then(|output| self.space.output_geometry(&output))
    }

    pub fn virtual_output_geometry(&self) -> Option<Rectangle<i32, Logical>> {
        let mut outputs = self.space.outputs();
        let first = outputs.next()?;
        let mut bounds = self.space.output_geometry(first)?;
        for output in outputs {
            if let Some(geometry) = self.space.output_geometry(output) {
                bounds = rect_union(bounds, geometry);
            }
        }
        Some(bounds)
    }

    pub(crate) fn initial_window_location(&self, window: &Window) -> Point<i32, Logical> {
        let Some(surface) = window
            .toplevel()
            .map(|toplevel| toplevel.wl_surface().clone())
        else {
            return Point::from((48, 48));
        };
        let uses_server_decoration = self.windows.uses_server_decoration(&surface);
        let content_size = non_empty_size(window.geometry().size, default_initial_window_size());

        self.centered_content_location_for(window, content_size, uses_server_decoration)
            .unwrap_or_else(|| Point::from((48, 48)))
    }

    pub(crate) fn position_new_window_if_needed(&mut self, window: &Window, surface: &WlSurface) {
        if !self.windows.needs_initial_position(surface) {
            return;
        }

        let content_size = non_empty_size(window.geometry().size, window.bbox().size);
        if content_size.w <= 0 || content_size.h <= 0 {
            return;
        }

        let uses_server_decoration = self.windows.uses_server_decoration(surface);
        if let Some(side) = self.snap_side_for_new_window(surface, window) {
            let output_geometry = self
                .placement_output_geometry(Some(window))
                .or_else(|| self.output_geometry_for(None));
            if let Some((_, location, size)) = output_geometry
                .and_then(|geometry| self.snap_layout(side, uses_server_decoration, geometry))
            {
                if let Some(toplevel) = window.toplevel() {
                    self.windows.set_snap(surface, side, None);
                    self.space.map_element(window.clone(), location, false);
                    toplevel.with_pending_state(|state| {
                        state.states.unset(xdg_toplevel::State::Maximized);
                        state.states.unset(xdg_toplevel::State::Fullscreen);
                        state.fullscreen_output = None;
                        state.bounds = Some(size);
                        state.size = Some(size);
                    });
                    toplevel.send_pending_configure();
                    info!(
                        ?side,
                        x = location.x,
                        y = location.y,
                        width = size.w,
                        height = size.h,
                        "placed new window using last snap side"
                    );
                    return;
                }
            }
        }

        let location = self
            .centered_content_location_for(window, content_size, uses_server_decoration)
            .unwrap_or_else(|| Point::from((48, 48)));

        self.space.map_element(window.clone(), location, false);
        self.windows.mark_initial_positioned(surface);
        info!(
            x = location.x,
            y = location.y,
            width = content_size.w,
            height = content_size.h,
            "centered new window"
        );
    }

    fn centered_content_location_for(
        &self,
        window: &Window,
        content_size: Size<i32, Logical>,
        uses_server_decoration: bool,
    ) -> Option<Point<i32, Logical>> {
        self.centered_content_location_for_window(
            Some(window),
            content_size,
            uses_server_decoration,
        )
    }

    fn centered_content_location_for_window(
        &self,
        window: Option<&Window>,
        content_size: Size<i32, Logical>,
        uses_server_decoration: bool,
    ) -> Option<Point<i32, Logical>> {
        let output_geometry = self.placement_output_geometry(window)?;
        let titlebar_height = if uses_server_decoration {
            crate::window::TITLEBAR_HEIGHT
        } else {
            0
        };
        let visual_width = content_size.w.max(1);
        let visual_height = (content_size.h + titlebar_height).max(1);
        let visual_x = output_geometry.loc.x + ((output_geometry.size.w - visual_width).max(0) / 2);
        let visual_y =
            output_geometry.loc.y + ((output_geometry.size.h - visual_height).max(0) / 2);

        Some(Point::from((visual_x, visual_y + titlebar_height)))
    }

    fn window_visual_rect(&self, window: &Window) -> Option<Rectangle<i32, Logical>> {
        let location = self.space.element_location(window)?;
        let Some(surface) = window
            .toplevel()
            .map(|toplevel| toplevel.wl_surface().clone())
        else {
            let bbox = window.bbox();
            return Some(Rectangle::new(
                location + bbox.loc,
                non_empty_size(bbox.size, window.geometry().size),
            ));
        };

        if self.windows.uses_server_decoration(&surface) && !self.windows.is_fullscreen(&surface) {
            let content_size = non_empty_size(window.geometry().size, window.bbox().size);
            return Some(Rectangle::new(
                (location.x, location.y - crate::window::TITLEBAR_HEIGHT).into(),
                (
                    content_size.w.max(1),
                    (content_size.h + crate::window::TITLEBAR_HEIGHT).max(1),
                )
                    .into(),
            ));
        }

        let bbox = window.bbox();
        let geometry = window.geometry();
        Some(Rectangle::new(
            location,
            non_empty_size(geometry.size, bbox.size),
        ))
    }

    fn visual_rect_from_content(
        content_rect: Rectangle<i32, Logical>,
        uses_server_decoration: bool,
    ) -> Rectangle<i32, Logical> {
        if !uses_server_decoration {
            return Rectangle::new(
                content_rect.loc,
                non_empty_size(content_rect.size, (1, 1).into()),
            );
        }

        Rectangle::new(
            (
                content_rect.loc.x,
                content_rect.loc.y - crate::window::TITLEBAR_HEIGHT,
            )
                .into(),
            (
                content_rect.size.w.max(1),
                content_rect.size.h.max(1) + crate::window::TITLEBAR_HEIGHT,
            )
                .into(),
        )
    }

    fn content_location_from_visual(
        visual_rect: Rectangle<i32, Logical>,
        uses_server_decoration: bool,
    ) -> Point<i32, Logical> {
        if uses_server_decoration {
            Point::from((
                visual_rect.loc.x,
                visual_rect.loc.y + crate::window::TITLEBAR_HEIGHT,
            ))
        } else {
            visual_rect.loc
        }
    }

    fn restore_rect_candidate_for_window(
        &self,
        window: &Window,
        uses_server_decoration: bool,
        output_geometry: Rectangle<i32, Logical>,
    ) -> Option<Rectangle<i32, Logical>> {
        let visual_rect = self.window_visual_rect(window)?;
        if rect_covers_output(visual_rect, output_geometry) {
            return None;
        }

        let content_location =
            Self::content_location_from_visual(visual_rect, uses_server_decoration);
        let content_size = if uses_server_decoration {
            (
                visual_rect.size.w.max(1),
                (visual_rect.size.h - crate::window::TITLEBAR_HEIGHT).max(1),
            )
                .into()
        } else {
            non_empty_size(visual_rect.size, window.geometry().size)
        };

        Some(Rectangle::new(content_location, content_size))
    }

    fn fallback_restore_rect_for_output(
        &self,
        window: &Window,
        uses_server_decoration: bool,
        output_geometry: Rectangle<i32, Logical>,
    ) -> Rectangle<i32, Logical> {
        let current_size = non_empty_size(window.geometry().size, window.bbox().size);
        let titlebar_height = if uses_server_decoration {
            crate::window::TITLEBAR_HEIGHT
        } else {
            0
        };
        let max_content_width = ((output_geometry.size.w * 3) / 4).max(1);
        let max_content_height =
            (((output_geometry.size.h - titlebar_height).max(1) * 3) / 4).max(1);
        let preferred_size = if current_size.w < output_geometry.size.w - 64
            && current_size.h < output_geometry.size.h - titlebar_height - 64
        {
            current_size
        } else {
            default_initial_window_size()
        };
        let content_size: Size<i32, Logical> = (
            preferred_size.w.min(max_content_width).max(1),
            preferred_size.h.min(max_content_height).max(1),
        )
            .into();
        let visual_width = content_size.w;
        let visual_height = content_size.h + titlebar_height;
        let visual_x = output_geometry.loc.x + ((output_geometry.size.w - visual_width).max(0) / 2);
        let visual_y =
            output_geometry.loc.y + ((output_geometry.size.h - visual_height).max(0) / 2);

        Rectangle::new((visual_x, visual_y + titlebar_height).into(), content_size)
    }

    fn usable_restore_rect(
        restore_rect: Rectangle<i32, Logical>,
        uses_server_decoration: bool,
        output_geometry: Option<Rectangle<i32, Logical>>,
    ) -> Option<Rectangle<i32, Logical>> {
        if let Some(output_geometry) = output_geometry {
            let visual_rect = Self::visual_rect_from_content(restore_rect, uses_server_decoration);
            if rect_covers_output(visual_rect, output_geometry) {
                return None;
            }
        }

        Some(restore_rect)
    }

    fn snap_layout(
        &self,
        side: SnapSide,
        uses_server_decoration: bool,
        output_geometry: Rectangle<i32, Logical>,
    ) -> Option<(
        Rectangle<i32, Logical>,
        Point<i32, Logical>,
        Size<i32, Logical>,
    )> {
        let titlebar_height = if uses_server_decoration {
            crate::window::TITLEBAR_HEIGHT
        } else {
            0
        };
        let content_width = (output_geometry.size.w / 2).max(1);
        let content_height = (output_geometry.size.h - titlebar_height).max(1);
        let visual_x = match side {
            SnapSide::Left => output_geometry.loc.x,
            SnapSide::Right => output_geometry.loc.x + output_geometry.size.w - content_width,
        };
        let visual_rect = Rectangle::new(
            (visual_x, output_geometry.loc.y).into(),
            (content_width, output_geometry.size.h).into(),
        );
        let location = Self::content_location_from_visual(visual_rect, uses_server_decoration);
        let size = (content_width, content_height).into();

        Some((visual_rect, location, size))
    }

    pub fn prune_windows(&mut self) {
        self.windows.prune_dead(self.config.animations());
        self.finish_graceful_shutdown_if_ready();
    }

    pub fn refresh_space_and_prune_windows(&mut self) {
        let animations = self.config.animations();
        self.space.refresh();
        self.windows.prune_dead(animations);
        self.finish_graceful_shutdown_if_ready();
    }

    pub fn finish_close_animations(&mut self) {
        let windows = self.windows.close_requests_ready(self.config.animations());
        for window in windows {
            if let Some(toplevel) = window.toplevel() {
                toplevel.send_close();
            } else {
                #[cfg(feature = "xwayland")]
                if let Some(surface) = window.x11_surface() {
                    if let Err(error) = surface.close() {
                        warn!(?error, "failed to close X11 window after animation");
                    }
                }
            }
        }
        self.finish_graceful_shutdown_if_ready();
    }

    pub fn request_graceful_shutdown(&mut self) {
        if self.shutdown_requested {
            return;
        }

        let windows = self.windows.managed_windows();
        if windows.is_empty() {
            self.loop_signal.stop();
            return;
        }

        self.shutdown_requested = true;
        self.shutdown_started_at = Some(Instant::now());
        for window in windows {
            self.close_window(&window);
        }
        self.request_render();
    }

    pub fn graceful_shutdown_pending(&self) -> bool {
        self.shutdown_requested
    }

    fn finish_graceful_shutdown_if_ready(&mut self) {
        if !self.shutdown_requested {
            return;
        }

        let timed_out = self
            .shutdown_started_at
            .map(|started_at| started_at.elapsed() >= GRACEFUL_SHUTDOWN_TIMEOUT)
            .unwrap_or(true);
        if self.windows.is_empty() || timed_out {
            self.loop_signal.stop();
        }
    }

    pub fn close_window(&mut self, window: &Window) {
        if let Some(toplevel) = window.toplevel() {
            if self
                .windows
                .request_close(toplevel.wl_surface(), self.config.animations())
            {
                toplevel.send_close();
            }
            return;
        }

        #[cfg(feature = "xwayland")]
        if let Some(surface) = window.x11_surface() {
            if self
                .windows
                .request_close_window(window, self.config.animations())
            {
                if let Err(error) = surface.close() {
                    warn!(?error, "failed to close X11 window");
                }
            }
        }
    }

    pub fn close_active_window(&mut self) {
        if let Some(window) = self.windows.active_window() {
            self.close_window(&window);
        }
    }

    pub fn kill_active_window(&mut self) {
        let Some(window) = self.windows.active_window() else {
            return;
        };
        if self.active_window_is_nirvana(&window) {
            info!("ignoring kill hotkey for focused Nirvana window");
            return;
        }
        #[cfg(feature = "xwayland")]
        if let Some(surface) = window.x11_surface() {
            let Some(pid) = surface.pid().or_else(|| surface.get_client_pid().ok()) else {
                warn!("failed to read active X11 window pid for kill hotkey");
                return;
            };
            kill_pid(pid);
            return;
        }

        let Some(surface) = window.toplevel().map(|toplevel| toplevel.wl_surface()) else {
            return;
        };

        let Ok(client) = self.display_handle.get_client(surface.id()) else {
            warn!("failed to resolve active window client for kill hotkey");
            return;
        };
        let Ok(credentials) = client.get_credentials(&self.display_handle) else {
            warn!("failed to read active window client credentials for kill hotkey");
            return;
        };

        kill_pid(credentials.pid as u32);
    }

    fn active_window_is_nirvana(&self, window: &Window) -> bool {
        #[cfg(feature = "xwayland")]
        if let Some(surface) = window.x11_surface() {
            if surface
                .class()
                .as_str()
                .trim()
                .eq_ignore_ascii_case("nirvana")
            {
                return true;
            }

            if let Some(pid) = surface.pid().or_else(|| surface.get_client_pid().ok()) {
                if pid_command_looks_like_nirvana(pid) {
                    return true;
                }
            }
        }

        let Some(surface) = window.toplevel().map(|toplevel| toplevel.wl_surface()) else {
            return false;
        };

        if self
            .windows
            .app_id(&surface)
            .as_deref()
            .is_some_and(is_nirvana_app_id)
        {
            return true;
        }

        let Ok(client) = self.display_handle.get_client(surface.id()) else {
            return false;
        };

        client_command_name(&client, &self.display_handle)
            .as_deref()
            .is_some_and(command_looks_like_nirvana)
    }

    pub fn toggle_window_fullscreen(&mut self, window: &Window) {
        #[cfg(feature = "xwayland")]
        if window.x11_surface().is_some() {
            self.set_x11_window_fullscreen(window, !self.windows.is_window_fullscreen(window));
            return;
        }

        let Some(surface) = window
            .toplevel()
            .map(|toplevel| toplevel.wl_surface().clone())
        else {
            return;
        };

        if self.windows.is_fullscreen(&surface) {
            self.set_window_fullscreen(window, false, None);
        } else {
            self.set_window_fullscreen(window, true, None);
        }
    }

    pub fn toggle_active_window_fullscreen(&mut self) {
        if let Some(window) = self.windows.active_window() {
            self.toggle_window_fullscreen(&window);
        }
    }

    pub fn toggle_window_maximized(&mut self, window: &Window) {
        #[cfg(feature = "xwayland")]
        if window.x11_surface().is_some() {
            self.set_x11_window_maximized(window, !self.windows.is_window_maximized(window));
            return;
        }

        let Some(surface) = window
            .toplevel()
            .map(|toplevel| toplevel.wl_surface().clone())
        else {
            return;
        };

        if self.windows.is_maximized(&surface) {
            self.set_window_maximized(window, false);
        } else {
            self.set_window_maximized(window, true);
        }
    }

    pub fn toggle_active_window_maximized(&mut self) {
        if let Some(window) = self.windows.active_window() {
            self.toggle_window_maximized(&window);
        }
    }

    pub fn toggle_active_window_minimized(&mut self) {
        if let Some(window) = self.windows.active_window() {
            self.set_window_minimized(&window);
        } else {
            self.restore_last_minimized_window();
        }
    }

    pub fn snap_active_window(&mut self, side: SnapSide) {
        if let Some(window) = self.windows.active_window() {
            self.snap_window(&window, side);
        }
    }

    pub fn snap_window(&mut self, window: &Window, side: SnapSide) {
        let Some(toplevel) = window.toplevel() else {
            return;
        };
        let surface = toplevel.wl_surface().clone();

        if self.windows.snap_side(&surface) == Some(side) {
            let restore_rect = self
                .windows
                .snap_restore_rect(&surface)
                .or_else(|| {
                    self.space
                        .element_location(window)
                        .map(|location| window_content_rect(window, location))
                })
                .unwrap_or_else(|| {
                    Rectangle::from_size(non_empty_size(window.geometry().size, window.bbox().size))
                });

            self.windows.clear_snap(&surface);
            if let Some(from_rect) = self.window_visual_rect(window) {
                let uses_server_decoration = self.windows.uses_server_decoration(&surface);
                let to_rect = Self::visual_rect_from_content(restore_rect, uses_server_decoration);
                self.windows
                    .start_geometry_animation(&surface, from_rect, to_rect);
            }
            self.space.raise_element(window, true);
            self.space
                .map_element(window.clone(), restore_rect.loc, false);
            toplevel.with_pending_state(|state| {
                state.states.unset(xdg_toplevel::State::Maximized);
                state.states.unset(xdg_toplevel::State::Fullscreen);
                state.fullscreen_output = None;
                state.bounds = None;
                state.size = Some(restore_rect.size);
            });
            toplevel.send_pending_configure();
            info!(
                ?side,
                x = restore_rect.loc.x,
                y = restore_rect.loc.y,
                width = restore_rect.size.w,
                height = restore_rect.size.h,
                "window restored from snap"
            );
            return;
        }

        let restore_rect = self.windows.snap_restore_rect(&surface).or_else(|| {
            self.space
                .element_location(window)
                .map(|location| window_content_rect(window, location))
        });
        let uses_server_decoration = self.windows.uses_server_decoration(&surface);
        let Some(output_geometry) = self.output_geometry_for_window(window) else {
            return;
        };
        let Some((target_visual_rect, window_location, window_size)) =
            self.snap_layout(side, uses_server_decoration, output_geometry)
        else {
            return;
        };

        if let Some(from_rect) = self.window_visual_rect(window) {
            self.windows
                .start_geometry_animation(&surface, from_rect, target_visual_rect);
        }

        self.windows.set_snap(&surface, side, restore_rect);
        if let Some(app_id) = self.windows.app_id(&surface) {
            self.snap_memory.insert(app_id, side);
        }
        self.space.raise_element(window, true);
        self.space
            .map_element(window.clone(), window_location, false);
        toplevel.with_pending_state(|state| {
            state.states.unset(xdg_toplevel::State::Maximized);
            state.states.unset(xdg_toplevel::State::Fullscreen);
            state.fullscreen_output = None;
            state.bounds = Some(window_size);
            state.size = Some(window_size);
        });
        toplevel.send_pending_configure();
        info!(
            ?side,
            x = window_location.x,
            y = window_location.y,
            width = window_size.w,
            height = window_size.h,
            "window snapped"
        );
    }

    pub fn set_window_maximized(&mut self, window: &Window, maximized: bool) {
        #[cfg(feature = "xwayland")]
        if window.x11_surface().is_some() {
            self.set_x11_window_maximized(window, maximized);
            return;
        }

        let Some(toplevel) = window.toplevel() else {
            return;
        };
        let surface = toplevel.wl_surface().clone();

        if maximized {
            let Some(output_geometry) = self.output_geometry_for_window(window) else {
                return;
            };
            let uses_server_decoration = self.windows.uses_server_decoration(&surface);
            let restore_rect = if self.windows.is_maximized(&surface) {
                self.windows
                    .restore_rect(&surface)
                    .and_then(|rect| {
                        Self::usable_restore_rect(
                            rect,
                            uses_server_decoration,
                            Some(output_geometry),
                        )
                    })
                    .or_else(|| {
                        Some(self.fallback_restore_rect_for_output(
                            window,
                            uses_server_decoration,
                            output_geometry,
                        ))
                    })
            } else {
                self.restore_rect_candidate_for_window(
                    window,
                    uses_server_decoration,
                    output_geometry,
                )
                .or_else(|| {
                    Some(self.fallback_restore_rect_for_output(
                        window,
                        uses_server_decoration,
                        output_geometry,
                    ))
                })
            };

            let titlebar_height = if uses_server_decoration {
                crate::window::TITLEBAR_HEIGHT
            } else {
                0
            };
            let window_location = Point::from((
                output_geometry.loc.x,
                output_geometry.loc.y + titlebar_height,
            ));
            let window_size = (
                output_geometry.size.w,
                (output_geometry.size.h - titlebar_height).max(1),
            )
                .into();
            let target_visual_rect = Rectangle::new(output_geometry.loc, output_geometry.size);

            if let Some(from_rect) = self.window_visual_rect(window) {
                self.windows
                    .start_geometry_animation(&surface, from_rect, target_visual_rect);
            }

            self.windows
                .set_maximized(&surface, true, restore_rect, Some(uses_server_decoration));
            self.space.raise_element(window, true);
            self.space
                .map_element(window.clone(), window_location, false);
            toplevel.with_pending_state(|state| {
                state.states.set(xdg_toplevel::State::Maximized);
                state.states.unset(xdg_toplevel::State::Fullscreen);
                state.fullscreen_output = None;
                state.bounds = Some(window_size);
                state.size = Some(window_size);
            });
            toplevel.send_pending_configure();
            let geometry = window.geometry();
            let bbox = window.bbox();
            info!(
                uses_server_decoration,
                x = window_location.x,
                y = window_location.y,
                width = window_size.w,
                height = window_size.h,
                geometry_x = geometry.loc.x,
                geometry_y = geometry.loc.y,
                geometry_w = geometry.size.w,
                geometry_h = geometry.size.h,
                bbox_x = bbox.loc.x,
                bbox_y = bbox.loc.y,
                bbox_w = bbox.size.w,
                bbox_h = bbox.size.h,
                "window entered maximized state"
            );
        } else {
            let uses_server_decoration = self.windows.uses_server_decoration(&surface);
            let output_geometry = self.output_geometry_for_window(window);
            let current_visual_covers_output = output_geometry
                .and_then(|geometry| {
                    self.window_visual_rect(window)
                        .map(|visual_rect| rect_covers_output(visual_rect, geometry))
                })
                .unwrap_or(false);

            if !self.windows.is_maximized(&surface)
                && self.windows.restore_rect(&surface).is_none()
                && !current_visual_covers_output
            {
                self.windows.set_maximized(&surface, false, None, None);
                toplevel.with_pending_state(|state| {
                    state.states.unset(xdg_toplevel::State::Maximized);
                    state.bounds = None;
                    state.size = None;
                });
                toplevel.send_pending_configure();
                info!("ignored redundant unmaximize request before window had a restore size");
                return;
            }

            let restore_rect = self
                .windows
                .restore_rect(&surface)
                .and_then(|rect| {
                    Self::usable_restore_rect(rect, uses_server_decoration, output_geometry)
                })
                .or_else(|| {
                    output_geometry.and_then(|geometry| {
                        self.restore_rect_candidate_for_window(
                            window,
                            uses_server_decoration,
                            geometry,
                        )
                    })
                })
                .or_else(|| {
                    output_geometry.map(|geometry| {
                        self.fallback_restore_rect_for_output(
                            window,
                            uses_server_decoration,
                            geometry,
                        )
                    })
                })
                .unwrap_or_else(|| {
                    Rectangle::from_size(non_empty_size(window.geometry().size, window.bbox().size))
                });

            if let Some(from_rect) = self.window_visual_rect(window) {
                let to_rect = Self::visual_rect_from_content(restore_rect, uses_server_decoration);
                self.windows
                    .start_geometry_animation(&surface, from_rect, to_rect);
            }
            self.windows.set_maximized(&surface, false, None, None);
            self.space
                .map_element(window.clone(), restore_rect.loc, false);
            toplevel.with_pending_state(|state| {
                state.states.unset(xdg_toplevel::State::Maximized);
                state.bounds = None;
                state.size = Some(restore_rect.size);
            });
            toplevel.send_pending_configure();
            info!(
                x = restore_rect.loc.x,
                y = restore_rect.loc.y,
                width = restore_rect.size.w,
                height = restore_rect.size.h,
                "window left maximized state"
            );
        }
    }

    #[cfg(feature = "xwayland")]
    fn set_x11_window_maximized(&mut self, window: &Window, maximized: bool) {
        let Some(surface) = window.x11_surface() else {
            return;
        };

        if maximized {
            let Some(output_geometry) = self.output_geometry_for_window(window) else {
                return;
            };
            let restore_rect = if self.windows.is_window_maximized(window) {
                self.windows.window_restore_rect(window)
            } else {
                self.space
                    .element_location(window)
                    .map(|location| window_content_rect(window, location))
            };

            if let Err(error) = surface.configure(output_geometry) {
                warn!(?error, "failed to configure X11 maximized window");
                return;
            }
            self.windows
                .set_window_maximized(window, true, restore_rect);
            self.space
                .map_element(window.clone(), output_geometry.loc, true);
            info!(
                width = output_geometry.size.w,
                height = output_geometry.size.h,
                "X11 window entered maximized state"
            );
        } else {
            let restore_rect = self
                .windows
                .window_restore_rect(window)
                .or_else(|| {
                    self.space
                        .element_location(window)
                        .map(|location| window_content_rect(window, location))
                })
                .unwrap_or_else(|| {
                    Rectangle::from_size(non_empty_size(window.geometry().size, window.bbox().size))
                });

            if let Err(error) = surface.configure(restore_rect) {
                warn!(?error, "failed to configure X11 restored window");
                return;
            }
            self.windows.set_window_maximized(window, false, None);
            self.space
                .map_element(window.clone(), restore_rect.loc, true);
            info!(
                width = restore_rect.size.w,
                height = restore_rect.size.h,
                "X11 window left maximized state"
            );
        }
    }

    pub fn set_window_minimized(&mut self, window: &Window) {
        #[cfg(feature = "xwayland")]
        if window.x11_surface().is_some() {
            let restore_rect = self
                .space
                .element_location(window)
                .map(|location| window_content_rect(window, location));
            self.windows
                .set_window_minimized(window, true, restore_rect);
            self.space.unmap_elem(window);
            self.seat.get_keyboard().unwrap().set_focus(
                self,
                Option::<FocusTarget>::None,
                SERIAL_COUNTER.next_serial(),
            );
            info!("X11 window minimized");
            return;
        }

        let Some(toplevel) = window.toplevel() else {
            return;
        };
        let surface = toplevel.wl_surface().clone();
        let restore_rect = self
            .space
            .element_location(window)
            .map(|location| window_content_rect(window, location));

        self.windows.set_minimized(&surface, true, restore_rect);
        self.space.unmap_elem(window);
        self.seat.get_keyboard().unwrap().set_focus(
            self,
            Option::<FocusTarget>::None,
            SERIAL_COUNTER.next_serial(),
        );
        info!("window minimized");
    }

    pub fn restore_last_minimized_window(&mut self) {
        let Some(window) = self.windows.last_minimized_window() else {
            return;
        };
        #[cfg(feature = "xwayland")]
        if let Some(surface) = window.x11_surface().cloned() {
            let restore_rect = self
                .windows
                .window_minimized_rect(&window)
                .or_else(|| self.windows.window_restore_rect(&window))
                .or_else(|| {
                    self.space
                        .element_location(&window)
                        .map(|location| window_content_rect(&window, location))
                })
                .unwrap_or_else(|| {
                    Rectangle::from_size(non_empty_size(window.geometry().size, window.bbox().size))
                });
            if let Err(error) = surface.configure(restore_rect) {
                warn!(?error, "failed to configure restored X11 window");
            }
            self.windows.set_window_minimized(&window, false, None);
            self.windows.activate_x11(&surface);
            self.space
                .map_element(window.clone(), restore_rect.loc, true);
            self.seat.get_keyboard().unwrap().set_focus(
                self,
                Some(FocusTarget::X11(surface)),
                SERIAL_COUNTER.next_serial(),
            );
            info!("X11 window restored from minimized state");
            return;
        }

        let Some(toplevel) = window.toplevel() else {
            return;
        };
        let surface = toplevel.wl_surface().clone();
        let restore_rect = self
            .windows
            .minimized_rect(&surface)
            .or_else(|| self.windows.restore_rect(&surface))
            .unwrap_or_else(|| {
                Rectangle::from_size(non_empty_size(window.geometry().size, window.bbox().size))
            });

        self.windows.set_minimized(&surface, false, None);
        self.windows.activate(&surface);
        self.space
            .map_element(window.clone(), restore_rect.loc, true);
        self.seat.get_keyboard().unwrap().set_focus(
            self,
            Some(FocusTarget::Wayland(surface)),
            SERIAL_COUNTER.next_serial(),
        );
        info!("window restored from minimized state");
    }

    pub fn reload_config_if_changed(&mut self) -> bool {
        let changed = self.config.reload_if_changed();
        if changed {
            self.apply_keyboard_config();
        }
        changed
    }

    pub fn apply_keyboard_config(&mut self) {
        let Some(keyboard) = self.seat.get_keyboard() else {
            return;
        };
        let keyboard_config = self.config.keyboard();
        if self.applied_keyboard_config.as_ref() == Some(&keyboard_config) {
            return;
        }

        let xkb_config = keyboard_config.xkb_config();
        if let Err(error) = keyboard.set_xkb_config(self, xkb_config) {
            warn!(
                ?error,
                layouts = %keyboard_config.layouts,
                "failed to apply keyboard config"
            );
        } else {
            info!(layouts = %keyboard_config.layouts, "applied keyboard config");
            self.applied_keyboard_config = Some(keyboard_config);
        }
    }

    pub fn cycle_keyboard_layout(&mut self) {
        let Some(keyboard) = self.seat.get_keyboard() else {
            return;
        };

        let layout_name = keyboard.with_xkb_state(self, |mut xkb_context| {
            xkb_context.cycle_next_layout();
            let xkb = xkb_context.xkb().lock().unwrap();
            let layout = xkb.active_layout();
            xkb.layout_name(layout).to_string()
        });

        info!(layout = %layout_name, "keyboard layout changed");
    }

    pub fn set_window_fullscreen(
        &mut self,
        window: &Window,
        fullscreen: bool,
        requested_output: Option<WlOutput>,
    ) {
        #[cfg(feature = "xwayland")]
        if window.x11_surface().is_some() {
            self.set_x11_window_fullscreen(window, fullscreen);
            return;
        }

        let Some(toplevel) = window.toplevel() else {
            return;
        };
        let surface = toplevel.wl_surface().clone();

        if fullscreen {
            let Some(output_geometry) = self
                .output_geometry_for(requested_output.as_ref())
                .or_else(|| self.output_geometry_for_window(window))
            else {
                return;
            };
            let restore_rect = if self.windows.is_fullscreen(&surface) {
                self.windows.restore_rect(&surface)
            } else {
                self.space
                    .element_location(window)
                    .map(|location| window_content_rect(window, location))
            };

            self.windows.set_fullscreen(&surface, true, restore_rect);
            self.space.raise_element(window, true);
            self.space
                .map_element(window.clone(), output_geometry.loc, false);
            toplevel.with_pending_state(|state| {
                state.states.set(xdg_toplevel::State::Fullscreen);
                state.states.unset(xdg_toplevel::State::Maximized);
                state.fullscreen_output = requested_output;
                state.size = Some(output_geometry.size);
            });
            toplevel.send_pending_configure();
            info!(
                width = output_geometry.size.w,
                height = output_geometry.size.h,
                "window entered fullscreen"
            );
        } else {
            if !self.windows.is_fullscreen(&surface)
                && self.windows.restore_rect(&surface).is_none()
            {
                self.windows.set_fullscreen(&surface, false, None);
                toplevel.with_pending_state(|state| {
                    state.states.unset(xdg_toplevel::State::Fullscreen);
                    state.fullscreen_output = None;
                    state.size = None;
                });
                toplevel.send_pending_configure();
                info!("ignored redundant unfullscreen request before window had a restore size");
                return;
            }

            let restore_rect = self
                .windows
                .restore_rect(&surface)
                .or_else(|| {
                    self.space
                        .element_location(window)
                        .map(|location| window_content_rect(window, location))
                })
                .unwrap_or_else(|| {
                    Rectangle::from_size(non_empty_size(window.geometry().size, window.bbox().size))
                });

            self.windows.set_fullscreen(&surface, false, None);
            self.space
                .map_element(window.clone(), restore_rect.loc, false);
            toplevel.with_pending_state(|state| {
                state.states.unset(xdg_toplevel::State::Fullscreen);
                state.fullscreen_output = None;
                state.size = Some(restore_rect.size);
            });
            toplevel.send_pending_configure();
            info!(
                x = restore_rect.loc.x,
                y = restore_rect.loc.y,
                width = restore_rect.size.w,
                height = restore_rect.size.h,
                "window left fullscreen"
            );
        }
    }

    #[cfg(feature = "xwayland")]
    fn set_x11_window_fullscreen(&mut self, window: &Window, fullscreen: bool) {
        let Some(surface) = window.x11_surface() else {
            return;
        };

        if fullscreen {
            let Some(output_geometry) = self.output_geometry_for_window(window) else {
                return;
            };
            let restore_rect = if self.windows.is_window_fullscreen(window) {
                self.windows.window_restore_rect(window)
            } else {
                self.space
                    .element_location(window)
                    .map(|location| window_content_rect(window, location))
            };

            if let Err(error) = surface.configure(output_geometry) {
                warn!(?error, "failed to configure X11 fullscreen window");
                return;
            }
            self.windows
                .set_window_fullscreen(window, true, restore_rect);
            self.space
                .map_element(window.clone(), output_geometry.loc, true);
            info!("X11 window entered fullscreen");
        } else {
            let restore_rect = self
                .windows
                .window_restore_rect(window)
                .or_else(|| {
                    self.space
                        .element_location(window)
                        .map(|location| window_content_rect(window, location))
                })
                .unwrap_or_else(|| {
                    Rectangle::from_size(non_empty_size(window.geometry().size, window.bbox().size))
                });

            if let Err(error) = surface.configure(restore_rect) {
                warn!(?error, "failed to configure X11 unfullscreen window");
                return;
            }
            self.windows.set_window_fullscreen(window, false, None);
            self.space
                .map_element(window.clone(), restore_rect.loc, true);
            info!("X11 window left fullscreen");
        }
    }

    fn output_geometry_for(
        &self,
        requested_output: Option<&WlOutput>,
    ) -> Option<Rectangle<i32, Logical>> {
        self.output_for(requested_output)
            .and_then(|output| self.space.output_geometry(output))
    }

    fn output_for(&self, requested_output: Option<&WlOutput>) -> Option<&Output> {
        if let Some(requested_output) = requested_output {
            if let Some(output) = self
                .space
                .outputs()
                .find(|output| output.owns(requested_output))
            {
                return Some(output);
            }
        }

        self.space.outputs().next()
    }

    fn output_geometry_for_window(&self, window: &Window) -> Option<Rectangle<i32, Logical>> {
        self.output_for_window(window)
            .and_then(|output| self.space.output_geometry(&output))
            .or_else(|| self.placement_output_geometry(Some(window)))
    }

    fn placement_output_geometry(
        &self,
        window: Option<&Window>,
    ) -> Option<Rectangle<i32, Logical>> {
        if let Some(window) = window {
            if let Some(parent_output) = self.parent_output_geometry(window) {
                return Some(parent_output);
            }
            if let Some(output) = self.output_for_window(window) {
                return self.space.output_geometry(&output);
            }
        }

        if let Some(active) = self.windows.active_window() {
            if let Some(output) = self.output_for_window(&active) {
                return self.space.output_geometry(&output);
            }
        }

        if let Some(pointer) = self.seat.get_pointer() {
            if let Some(geometry) = self.output_geometry_at(pointer.current_location()) {
                return Some(geometry);
            }
        }

        self.space
            .outputs()
            .next()
            .and_then(|output| self.space.output_geometry(output))
    }

    fn parent_output_geometry(&self, window: &Window) -> Option<Rectangle<i32, Logical>> {
        let parent_surface = window.toplevel()?.parent()?;
        self.space
            .elements()
            .find(|candidate| {
                candidate
                    .toplevel()
                    .map(|toplevel| toplevel.wl_surface() == &parent_surface)
                    .unwrap_or(false)
            })
            .and_then(|candidate| self.output_geometry_for_window(candidate))
    }

    fn snap_side_for_new_window(&self, surface: &WlSurface, window: &Window) -> Option<SnapSide> {
        if is_transient_toplevel(window) {
            return None;
        }

        self.windows
            .app_id(surface)
            .and_then(|app_id| self.snap_memory.get(&app_id).copied())
    }
}

fn default_initial_window_size() -> Size<i32, Logical> {
    (900, 640).into()
}

fn is_transient_toplevel(window: &Window) -> bool {
    window
        .toplevel()
        .and_then(|toplevel| toplevel.parent())
        .is_some()
}

fn is_overview_window(window: &Window) -> bool {
    if window.toplevel().is_some() {
        return true;
    }

    #[cfg(feature = "xwayland")]
    {
        window.x11_surface().is_some()
    }
    #[cfg(not(feature = "xwayland"))]
    {
        false
    }
}

fn window_content_rect(window: &Window, location: Point<i32, Logical>) -> Rectangle<i32, Logical> {
    Rectangle::new(
        location,
        non_empty_size(window.geometry().size, window.bbox().size),
    )
}

fn overview_layout(
    mut windows: Vec<(Window, Rectangle<i32, Logical>)>,
    output_geometry: Rectangle<i32, Logical>,
    active_window: Option<Window>,
    selected_window: Option<&Window>,
    progress: f64,
) -> Vec<OverviewWindow> {
    let count = windows.len();
    if count == 0 {
        return Vec::new();
    }

    windows.sort_by(|(_, left), (_, right)| {
        let left_center = rect_center(*left);
        let right_center = rect_center(*right);
        left_center
            .y
            .partial_cmp(&right_center.y)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                left_center
                    .x
                    .partial_cmp(&right_center.x)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });

    let aspect = output_geometry.size.w.max(1) as f64 / output_geometry.size.h.max(1) as f64;
    let rows = overview_row_count(count, aspect);
    let row_lengths = overview_row_lengths(count, rows);
    let cols = row_lengths.iter().copied().max().unwrap_or(1).max(1);
    let margin = (output_geometry.size.w.min(output_geometry.size.h) / 16).clamp(48, 120);
    let gap = 36;
    let available_w =
        (output_geometry.size.w - margin * 2 - gap * (cols.saturating_sub(1) as i32)).max(1);
    let available_h =
        (output_geometry.size.h - margin * 2 - gap * (rows.saturating_sub(1) as i32)).max(1);
    let cell_w = (available_w / cols as i32).max(1);
    let cell_h = (available_h / rows as i32).max(1);

    let mut row_offsets = Vec::with_capacity(row_lengths.len());
    let mut offset = 0usize;
    for length in &row_lengths {
        row_offsets.push(offset);
        offset += *length;
    }

    let mut result = Vec::with_capacity(count);
    for (row, row_len) in row_lengths.into_iter().enumerate() {
        if row_len == 0 {
            continue;
        }
        let row_width = cell_w * row_len as i32 + gap * row_len.saturating_sub(1) as i32;
        let row_x = output_geometry.loc.x + margin + ((available_w - row_width).max(0) / 2);
        let row_y = output_geometry.loc.y + margin + row as i32 * (cell_h + gap);
        let offset = row_offsets[row];
        result.extend(windows.iter().skip(offset).take(row_len).enumerate().map(
            |(col, (window, source))| {
                overview_layout_item(
                    window.clone(),
                    *source,
                    row_x,
                    row_y,
                    col,
                    cell_w,
                    cell_h,
                    gap,
                    active_window.as_ref(),
                    selected_window,
                    progress,
                )
            },
        ));
    }

    result
}

fn overview_layout_item(
    window: Window,
    source: Rectangle<i32, Logical>,
    row_x: i32,
    row_y: i32,
    col: usize,
    cell_w: i32,
    cell_h: i32,
    gap: i32,
    active_window: Option<&Window>,
    selected_window: Option<&Window>,
    progress: f64,
) -> OverviewWindow {
    let scale = (cell_w as f64 / source.size.w.max(1) as f64)
        .min(cell_h as f64 / source.size.h.max(1) as f64)
        .min(1.0);
    let target_size = Size::from((
        (source.size.w as f64 * scale).round().max(1.0) as i32,
        (source.size.h as f64 * scale).round().max(1.0) as i32,
    ));
    let cell_x = row_x + col as i32 * (cell_w + gap);
    let target = Rectangle::new(
        (
            cell_x + ((cell_w - target_size.w).max(0) / 2),
            row_y + ((cell_h - target_size.h).max(0) / 2),
        )
            .into(),
        target_size,
    );
    let active = active_window
        .map(|active| active == &window)
        .unwrap_or(false);
    let selected = selected_window
        .map(|selected| selected == &window)
        .unwrap_or(active);

    OverviewWindow {
        window,
        source,
        target,
        active,
        selected,
        selection_alpha: if selected {
            0.50
        } else if active {
            0.30
        } else {
            0.0
        },
        progress,
    }
}

fn overview_row_count(count: usize, aspect: f64) -> usize {
    if count <= 2 {
        return 1;
    }

    ((count as f64 / aspect.max(1.0)).sqrt().ceil() as usize)
        .max(1)
        .min(count)
}

fn overview_row_lengths(count: usize, rows: usize) -> Vec<usize> {
    let rows = rows.max(1).min(count.max(1));
    let base = count / rows;
    let remainder = count % rows;

    (0..rows)
        .map(|row| base + usize::from(row < remainder))
        .collect()
}

fn rect_center(rect: Rectangle<i32, Logical>) -> Point<f64, Logical> {
    Point::from((
        rect.loc.x as f64 + rect.size.w as f64 * 0.5,
        rect.loc.y as f64 + rect.size.h as f64 * 0.5,
    ))
}

fn wrap_overview_index(current_index: usize, len: usize, dx: i32, dy: i32) -> usize {
    if len <= 1 {
        return current_index;
    }
    if dx > 0 || dy > 0 {
        (current_index + 1) % len
    } else {
        (current_index + len - 1) % len
    }
}

fn ease_out_cubic(t: f64) -> f64 {
    1.0 - (1.0 - t).powi(3)
}

fn non_empty_size(
    preferred: Size<i32, Logical>,
    fallback: Size<i32, Logical>,
) -> Size<i32, Logical> {
    if preferred.w > 0 && preferred.h > 0 {
        preferred
    } else if fallback.w > 0 && fallback.h > 0 {
        fallback
    } else {
        default_initial_window_size()
    }
}

fn rect_covers_output(rect: Rectangle<i32, Logical>, output: Rectangle<i32, Logical>) -> bool {
    const TOLERANCE: i32 = 8;

    rect.loc.x <= output.loc.x + TOLERANCE
        && rect.loc.y <= output.loc.y + TOLERANCE
        && rect.loc.x + rect.size.w >= output.loc.x + output.size.w - TOLERANCE
        && rect.loc.y + rect.size.h >= output.loc.y + output.size.h - TOLERANCE
}

fn kill_pid(pid: u32) {
    if pid <= 1 {
        warn!(pid, "refusing to kill active window with invalid pid");
        return;
    }

    match Command::new("kill")
        .arg("-KILL")
        .arg(pid.to_string())
        .status()
    {
        Ok(status) if status.success() => {
            info!(pid, "killed active window process");
        }
        Ok(status) => {
            warn!(pid, code = status.code(), "kill command failed");
        }
        Err(error) => {
            warn!(?error, pid, "failed to execute kill command");
        }
    }
}

fn rect_union(
    lhs: Rectangle<i32, Logical>,
    rhs: Rectangle<i32, Logical>,
) -> Rectangle<i32, Logical> {
    let min_x = lhs.loc.x.min(rhs.loc.x);
    let min_y = lhs.loc.y.min(rhs.loc.y);
    let max_x = (lhs.loc.x + lhs.size.w).max(rhs.loc.x + rhs.size.w);
    let max_y = (lhs.loc.y + lhs.size.h).max(rhs.loc.y + rhs.size.h);
    Rectangle::new((min_x, min_y).into(), (max_x - min_x, max_y - min_y).into())
}

#[derive(Default)]
pub struct ClientState {
    pub compositor_state: CompositorClientState,
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}

    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}

fn should_show_dmabuf_global(
    client: &Client,
    display_handle: &DisplayHandle,
    hide_portal_bridge: bool,
) -> bool {
    if env_flag("YAWC_DISABLE_CLIENT_DMABUF") {
        return false;
    }

    let Some(command) = client_command_name(client, display_handle) else {
        return true;
    };

    if hide_portal_bridge && command.contains("xdg-desktop-portal-wlr") {
        info!(client = %command, "hiding linux-dmabuf global from portal bridge");
        return false;
    }

    if env_flag("YAWC_ENABLE_DMABUF_PROBE") && !env_flag("YAWC_ENABLE_CLIENT_DMABUF") {
        let visible = is_dmabuf_probe_client(&command);
        info!(
            client = %command,
            visible,
            "evaluated linux-dmabuf probe visibility"
        );
        return visible;
    }

    true
}

fn is_dmabuf_probe_client(command: &str) -> bool {
    [
        "obs",
        "obs-studio",
        "eglinfo",
        "weston-simple-dmabuf",
        "weston-simple-egl",
    ]
    .iter()
    .any(|name| command.contains(name))
}

fn client_command_name(client: &Client, display_handle: &DisplayHandle) -> Option<String> {
    let Ok(credentials) = client.get_credentials(display_handle) else {
        return None;
    };
    let pid = credentials.pid;
    if pid <= 0 {
        return None;
    }

    let cmdline_path = format!("/proc/{pid}/cmdline");
    if let Ok(cmdline) = std::fs::read(&cmdline_path) {
        let command = String::from_utf8_lossy(&cmdline)
            .replace('\0', " ")
            .trim()
            .to_string();
        if !command.is_empty() {
            return Some(command);
        }
    }

    let comm_path = format!("/proc/{pid}/comm");
    std::fs::read_to_string(comm_path)
        .ok()
        .map(|comm| comm.trim().to_string())
}

fn pid_command_name(pid: u32) -> Option<String> {
    if pid <= 1 {
        return None;
    }

    let cmdline_path = format!("/proc/{pid}/cmdline");
    if let Ok(cmdline) = std::fs::read(&cmdline_path) {
        let command = String::from_utf8_lossy(&cmdline)
            .replace('\0', " ")
            .trim()
            .to_string();
        if !command.is_empty() {
            return Some(command);
        }
    }

    let comm_path = format!("/proc/{pid}/comm");
    std::fs::read_to_string(comm_path)
        .ok()
        .map(|comm| comm.trim().to_string())
}

fn pid_command_looks_like_nirvana(pid: u32) -> bool {
    pid_command_name(pid)
        .as_deref()
        .is_some_and(command_looks_like_nirvana)
}

fn is_nirvana_app_id(app_id: &str) -> bool {
    app_id.trim().eq_ignore_ascii_case("nirvana")
}

fn command_looks_like_nirvana(command: &str) -> bool {
    let normalized = command.trim().to_ascii_lowercase();
    normalized.contains("/nirvana/")
        || normalized.contains(" etyos/nirvana/")
        || normalized.contains("nirvana/src/main/main.js")
        || normalized.contains("nirvana/scripts/run-electron.sh")
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
