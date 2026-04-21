use std::{ffi::OsString, process::Command, sync::Arc, time::Instant};

use smithay::{
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
            Display, DisplayHandle, Resource,
        },
    },
    utils::{Logical, Point, Rectangle, Size, SERIAL_COUNTER},
    wayland::{
        compositor::{CompositorClientState, CompositorState},
        cursor_shape::CursorShapeManagerState,
        output::OutputManagerState,
        selection::data_device::DataDeviceState,
        shell::xdg::{decoration::XdgDecorationState, XdgShellState},
        shm::ShmState,
        socket::ListeningSocketSource,
    },
};
use tracing::{info, warn};

use crate::{
    config::{Config, KeyboardConfig},
    cursor::CursorShape,
    screencopy::ScreencopyState,
    window::{SnapSide, WindowStore},
    CalloopData,
};

#[derive(Clone)]
pub struct TitlebarClick {
    pub surface: WlSurface,
    pub time_msec: u32,
    pub location: Point<f64, Logical>,
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
    pub xdg_decoration_state: XdgDecorationState,
    pub shm_state: ShmState,
    pub output_manager_state: OutputManagerState,
    pub cursor_shape_state: CursorShapeManagerState,
    pub seat_state: SeatState<Self>,
    pub data_device_state: DataDeviceState,
    pub screencopy_state: ScreencopyState,
    pub popups: PopupManager,
    pub seat: Seat<Self>,
    pub pending_cursor: CursorShape,
    pub compositor_cursor: Option<CursorShape>,
    pub cursor_image: CursorImageStatus,
    pub dnd_icon: Option<WlSurface>,
    pub config: Config,
    pub titlebar_right_press: Option<WlSurface>,
    pub last_titlebar_click: Option<TitlebarClick>,
    applied_keyboard_config: Option<KeyboardConfig>,
}

impl Yawc {
    pub fn new(event_loop: &mut EventLoop<CalloopData>, display: Display<Self>) -> Self {
        let start_time = Instant::now();
        let display_handle = display.handle();
        let compositor_state = CompositorState::new::<Self>(&display_handle);
        let xdg_shell_state = XdgShellState::new::<Self>(&display_handle);
        let xdg_decoration_state = XdgDecorationState::new::<Self>(&display_handle);
        let shm_state = ShmState::new::<Self>(&display_handle, vec![]);
        let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(&display_handle);
        let cursor_shape_state = CursorShapeManagerState::new::<Self>(&display_handle);
        let mut seat_state = SeatState::new();
        let data_device_state = DataDeviceState::new::<Self>(&display_handle);
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
            xdg_decoration_state,
            shm_state,
            output_manager_state,
            cursor_shape_state,
            seat_state,
            data_device_state,
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
            applied_keyboard_config: None,
        };
        state.apply_keyboard_config();
        state
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
    ) -> Option<(WlSurface, Point<f64, Logical>)> {
        self.space
            .element_under(position)
            .and_then(|(window, location)| {
                window
                    .surface_under(position - location.to_f64(), WindowSurfaceType::ALL)
                    .map(|(surface, point)| (surface, (point + location).to_f64()))
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

    pub(crate) fn initial_window_location(&self, window: &Window) -> Point<i32, Logical> {
        let Some(surface) = window
            .toplevel()
            .map(|toplevel| toplevel.wl_surface().clone())
        else {
            return Point::from((48, 48));
        };
        let uses_server_decoration = self.windows.uses_server_decoration(&surface);
        let content_size = non_empty_size(window.geometry().size, default_initial_window_size());

        self.centered_content_location(content_size, uses_server_decoration)
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
        let location = self
            .centered_content_location(content_size, uses_server_decoration)
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

    fn centered_content_location(
        &self,
        content_size: Size<i32, Logical>,
        uses_server_decoration: bool,
    ) -> Option<Point<i32, Logical>> {
        let output_geometry = self.output_geometry_for(None)?;
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
        let surface = window
            .toplevel()
            .map(|toplevel| toplevel.wl_surface().clone())?;

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
        Some(Rectangle::new(location + bbox.loc, bbox.size))
    }

    fn visual_rect_from_content(
        content_rect: Rectangle<i32, Logical>,
        uses_server_decoration: bool,
    ) -> Rectangle<i32, Logical> {
        if !uses_server_decoration {
            return content_rect;
        }

        Rectangle::new(
            (
                content_rect.loc.x,
                content_rect.loc.y - crate::window::TITLEBAR_HEIGHT,
            )
                .into(),
            (
                content_rect.size.w,
                content_rect.size.h + crate::window::TITLEBAR_HEIGHT,
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

    pub fn prune_windows(&mut self) {
        self.windows.prune_dead(self.config.animations());
    }

    pub fn finish_close_animations(&mut self) {
        let windows = self.windows.close_requests_ready(self.config.animations());
        for window in windows {
            if let Some(toplevel) = window.toplevel() {
                toplevel.send_close();
            }
        }
    }

    pub fn close_window(&mut self, window: &Window) {
        let Some(toplevel) = window.toplevel() else {
            return;
        };

        if self
            .windows
            .request_close(toplevel.wl_surface(), self.config.animations())
        {
            toplevel.send_close();
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

        let pid = credentials.pid;
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

    pub fn toggle_window_fullscreen(&mut self, window: &Window) {
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
                        .map(|location| Rectangle::new(location, window.geometry().size))
                })
                .unwrap_or_else(|| Rectangle::from_size(window.geometry().size));

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

        let Some(output_geometry) = self.output_geometry_for(None) else {
            return;
        };

        let restore_rect = self.windows.snap_restore_rect(&surface).or_else(|| {
            self.space
                .element_location(window)
                .map(|location| Rectangle::new(location, window.geometry().size))
        });
        let uses_server_decoration = self.windows.uses_server_decoration(&surface);
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
        let target_visual_rect = Rectangle::new(
            (visual_x, output_geometry.loc.y).into(),
            (content_width, output_geometry.size.h).into(),
        );
        let window_location =
            Self::content_location_from_visual(target_visual_rect, uses_server_decoration);
        let window_size = (content_width, content_height).into();

        if let Some(from_rect) = self.window_visual_rect(window) {
            self.windows
                .start_geometry_animation(&surface, from_rect, target_visual_rect);
        }

        self.windows.set_snap(&surface, side, restore_rect);
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
        let Some(toplevel) = window.toplevel() else {
            return;
        };
        let surface = toplevel.wl_surface().clone();

        if maximized {
            let Some(output_geometry) = self.output_geometry_for(None) else {
                return;
            };
            let restore_rect = if self.windows.is_maximized(&surface) {
                self.windows.restore_rect(&surface)
            } else {
                self.space
                    .element_location(window)
                    .map(|location| Rectangle::new(location, window.geometry().size))
            };

            let uses_server_decoration = self.windows.uses_server_decoration(&surface);
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
            if !self.windows.is_maximized(&surface) && self.windows.restore_rect(&surface).is_none()
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
                .or_else(|| {
                    self.space
                        .element_location(window)
                        .map(|location| Rectangle::new(location, window.geometry().size))
                })
                .unwrap_or_else(|| Rectangle::from_size(window.geometry().size));

            if let Some(from_rect) = self.window_visual_rect(window) {
                let uses_server_decoration = self.windows.uses_server_decoration(&surface);
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

    pub fn set_window_minimized(&mut self, window: &Window) {
        let Some(toplevel) = window.toplevel() else {
            return;
        };
        let surface = toplevel.wl_surface().clone();
        let restore_rect = self
            .space
            .element_location(window)
            .map(|location| Rectangle::new(location, window.geometry().size));

        self.windows.set_minimized(&surface, true, restore_rect);
        self.space.unmap_elem(window);
        self.seat.get_keyboard().unwrap().set_focus(
            self,
            Option::<WlSurface>::None,
            SERIAL_COUNTER.next_serial(),
        );
        info!("window minimized");
    }

    pub fn restore_last_minimized_window(&mut self) {
        let Some(window) = self.windows.last_minimized_window() else {
            return;
        };
        let Some(toplevel) = window.toplevel() else {
            return;
        };
        let surface = toplevel.wl_surface().clone();
        let restore_rect = self
            .windows
            .minimized_rect(&surface)
            .or_else(|| self.windows.restore_rect(&surface))
            .unwrap_or_else(|| Rectangle::from_size(window.geometry().size));

        self.windows.set_minimized(&surface, false, None);
        self.windows.activate(&surface);
        self.space
            .map_element(window.clone(), restore_rect.loc, true);
        self.seat.get_keyboard().unwrap().set_focus(
            self,
            Some(surface),
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
        let Some(toplevel) = window.toplevel() else {
            return;
        };
        let surface = toplevel.wl_surface().clone();

        if fullscreen {
            let Some(output_geometry) = self.output_geometry_for(requested_output.as_ref()) else {
                return;
            };
            let restore_rect = if self.windows.is_fullscreen(&surface) {
                self.windows.restore_rect(&surface)
            } else {
                self.space
                    .element_location(window)
                    .map(|location| Rectangle::new(location, window.geometry().size))
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
                        .map(|location| Rectangle::new(location, window.geometry().size))
                })
                .unwrap_or_else(|| Rectangle::from_size(window.geometry().size));

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
}

fn default_initial_window_size() -> Size<i32, Logical> {
    (900, 640).into()
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

#[derive(Default)]
pub struct ClientState {
    pub compositor_state: CompositorClientState,
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}

    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}
