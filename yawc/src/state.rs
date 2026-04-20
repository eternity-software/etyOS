use std::{ffi::OsString, sync::Arc, time::Instant};

use smithay::{
    desktop::{PopupManager, Space, Window, WindowSurfaceType},
    input::{Seat, SeatState},
    output::Output,
    reexports::{
        calloop::{generic::Generic, EventLoop, Interest, LoopSignal, Mode, PostAction},
        wayland_protocols::xdg::shell::server::xdg_toplevel,
        wayland_server::{
            backend::{ClientData, ClientId, DisconnectReason},
            protocol::{wl_output::WlOutput, wl_surface::WlSurface},
            Display, DisplayHandle,
        },
    },
    utils::{Logical, Point, Rectangle, SERIAL_COUNTER},
    wayland::{
        compositor::{CompositorClientState, CompositorState},
        output::OutputManagerState,
        selection::data_device::DataDeviceState,
        shell::xdg::{decoration::XdgDecorationState, XdgShellState},
        shm::ShmState,
        socket::ListeningSocketSource,
    },
};
use tracing::info;

use crate::{
    config::Config,
    cursor::CursorShape,
    window::{SnapSide, WindowStore},
    CalloopData,
};

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
    pub seat_state: SeatState<Self>,
    pub data_device_state: DataDeviceState,
    pub popups: PopupManager,
    pub seat: Seat<Self>,
    pub pending_cursor: CursorShape,
    pub config: Config,
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
        let mut seat_state = SeatState::new();
        let data_device_state = DataDeviceState::new::<Self>(&display_handle);
        let popups = PopupManager::default();
        let windows = WindowStore::default();

        let mut seat: Seat<Self> = seat_state.new_wl_seat(&display_handle, "seat0");
        seat.add_keyboard(Default::default(), 200, 25).unwrap();
        seat.add_pointer();

        let space = Space::default();
        let socket_name = Self::init_wayland_listener(display, event_loop);
        let loop_signal = event_loop.get_signal();
        let config = Config::load_or_create();

        Self {
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
            seat_state,
            data_device_state,
            popups,
            seat,
            pending_cursor: CursorShape::Default,
            config,
        }
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

    pub fn send_pending_configures(&self) {
        self.space.elements().for_each(|window| {
            if let Some(toplevel) = window.toplevel() {
                toplevel.send_pending_configure();
            }
        });
    }

    pub fn prune_windows(&mut self) {
        self.windows.prune_dead();
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
        let x = match side {
            SnapSide::Left => output_geometry.loc.x,
            SnapSide::Right => output_geometry.loc.x + output_geometry.size.w - content_width,
        };
        let y = output_geometry.loc.y + titlebar_height;
        let window_location = Point::from((x, y));
        let window_size = (content_width, content_height).into();

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
            let restore_rect = self
                .windows
                .restore_rect(&surface)
                .or_else(|| {
                    self.space
                        .element_location(window)
                        .map(|location| Rectangle::new(location, window.geometry().size))
                })
                .unwrap_or_else(|| Rectangle::from_size(window.geometry().size));

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

#[derive(Default)]
pub struct ClientState {
    pub compositor_state: CompositorClientState,
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}

    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}
