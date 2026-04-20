use smithay::{
    delegate_xdg_shell,
    desktop::{
        find_popup_root_surface, get_popup_toplevel_coords, PopupKind, PopupManager, Space, Window,
    },
    input::{
        pointer::{Focus, GrabStartData as PointerGrabStartData},
        Seat,
    },
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel,
        wayland_server::{
            protocol::{wl_seat, wl_surface::WlSurface},
            Resource,
        },
    },
    utils::{Rectangle, Serial, SERIAL_COUNTER},
    wayland::{
        compositor::{self, with_states},
        shell::xdg::{
            PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
            XdgToplevelSurfaceData,
        },
    },
};
use tracing::info;

use crate::{
    grabs::{MoveSurfaceGrab, ResizeSurfaceGrab},
    state::Yawc,
    window::ResizeEdge,
};

impl XdgShellHandler for Yawc {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        let metadata = read_toplevel_metadata(&surface);
        let window = Window::new_wayland_window(surface);
        let wl_surface = window.toplevel().unwrap().wl_surface().clone();
        let location = self.windows.insert(window.clone());
        self.windows.set_metadata(&wl_surface, metadata.clone());
        self.windows.activate(&wl_surface);

        self.space.map_element(window, location, true);
        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_focus(self, Some(wl_surface), SERIAL_COUNTER.next_serial());
        }
        self.send_pending_configures();
        info!(
            windows = self.windows.len(),
            x = location.x,
            y = location.y,
            title = metadata.title,
            "mapped new xdg toplevel"
        );
    }

    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        self.unconstrain_popup(&surface);
        let _ = self.popups.track_popup(PopupKind::Xdg(surface));
    }

    fn reposition_request(
        &mut self,
        surface: PopupSurface,
        positioner: PositionerState,
        token: u32,
    ) {
        surface.with_pending_state(|state| {
            state.geometry = positioner.get_geometry();
            state.positioner = positioner;
        });

        self.unconstrain_popup(&surface);
        surface.send_repositioned(token);
    }

    fn move_request(&mut self, surface: ToplevelSurface, seat: wl_seat::WlSeat, serial: Serial) {
        let seat = Seat::from_resource(&seat).unwrap();
        let wl_surface = surface.wl_surface();

        if self.windows.is_fullscreen(wl_surface) || self.windows.is_maximized(wl_surface) {
            return;
        }

        if let Some(start_data) = check_grab(&seat, wl_surface, serial) {
            let pointer = seat.get_pointer().unwrap();
            let window = self
                .space
                .elements()
                .find(|window| window.toplevel().unwrap().wl_surface() == wl_surface)
                .unwrap()
                .clone();
            let initial_window_location = self.space.element_location(&window).unwrap();

            pointer.set_grab(
                self,
                MoveSurfaceGrab {
                    start_data,
                    window,
                    initial_window_location,
                },
                serial,
                Focus::Clear,
            );
        }
    }

    fn resize_request(
        &mut self,
        surface: ToplevelSurface,
        seat: wl_seat::WlSeat,
        serial: Serial,
        edges: xdg_toplevel::ResizeEdge,
    ) {
        let seat = Seat::from_resource(&seat).unwrap();
        let wl_surface = surface.wl_surface();

        if self.windows.is_fullscreen(wl_surface) || self.windows.is_maximized(wl_surface) {
            return;
        }

        if let Some(start_data) = check_grab(&seat, wl_surface, serial) {
            let pointer = seat.get_pointer().unwrap();
            let window = self
                .space
                .elements()
                .find(|window| window.toplevel().unwrap().wl_surface() == wl_surface)
                .unwrap()
                .clone();
            let initial_window_location = self.space.element_location(&window).unwrap();
            let initial_window_size = window.geometry().size;

            surface.with_pending_state(|state| {
                state.states.set(xdg_toplevel::State::Resizing);
            });
            surface.send_pending_configure();
            self.windows.set_resizing(wl_surface, true);

            let grab = ResizeSurfaceGrab::start(
                start_data,
                window,
                ResizeEdge::from(edges),
                Rectangle::new(initial_window_location, initial_window_size),
            );
            pointer.set_grab(self, grab, serial, Focus::Clear);
        }
    }

    fn maximize_request(&mut self, surface: ToplevelSurface) {
        let wl_surface = surface.wl_surface();
        let Some(window) = self
            .space
            .elements()
            .find(|window| window.toplevel().unwrap().wl_surface() == wl_surface)
            .cloned()
        else {
            surface.send_configure();
            return;
        };

        self.set_window_maximized(&window, true);
    }

    fn unmaximize_request(&mut self, surface: ToplevelSurface) {
        let wl_surface = surface.wl_surface();
        let Some(window) = self
            .space
            .elements()
            .find(|window| window.toplevel().unwrap().wl_surface() == wl_surface)
            .cloned()
        else {
            surface.send_configure();
            return;
        };

        self.set_window_maximized(&window, false);
    }

    fn fullscreen_request(
        &mut self,
        surface: ToplevelSurface,
        output: Option<smithay::reexports::wayland_server::protocol::wl_output::WlOutput>,
    ) {
        let wl_surface = surface.wl_surface();
        let Some(window) = self
            .space
            .elements()
            .find(|window| window.toplevel().unwrap().wl_surface() == wl_surface)
            .cloned()
        else {
            surface.send_configure();
            return;
        };

        self.set_window_fullscreen(&window, true, output);
    }

    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        let wl_surface = surface.wl_surface();
        let Some(window) = self
            .space
            .elements()
            .find(|window| window.toplevel().unwrap().wl_surface() == wl_surface)
            .cloned()
        else {
            surface.send_configure();
            return;
        };

        self.set_window_fullscreen(&window, false, None);
    }

    fn minimize_request(&mut self, surface: ToplevelSurface) {
        let wl_surface = surface.wl_surface();
        let Some(window) = self
            .space
            .elements()
            .find(|window| window.toplevel().unwrap().wl_surface() == wl_surface)
            .cloned()
        else {
            surface.send_configure();
            return;
        };

        self.set_window_minimized(&window);
    }

    fn title_changed(&mut self, surface: ToplevelSurface) {
        let metadata = read_toplevel_metadata(&surface);
        self.windows
            .set_metadata(surface.wl_surface(), metadata.clone());
        info!(title = metadata.title, "xdg title updated");
    }

    fn app_id_changed(&mut self, surface: ToplevelSurface) {
        let metadata = read_toplevel_metadata(&surface);
        self.windows
            .set_metadata(surface.wl_surface(), metadata.clone());
        info!(app_id = metadata.app_id, "xdg app_id updated");
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        self.windows.remove(surface.wl_surface());
    }

    fn grab(
        &mut self,
        _surface: PopupSurface,
        _seat: wl_seat::WlSeat,
        _serial: smithay::utils::Serial,
    ) {
        // Popup grabs are intentionally left for a later milestone.
    }
}

delegate_xdg_shell!(Yawc);

pub fn handle_commit(popups: &mut PopupManager, space: &Space<Window>, surface: &WlSurface) {
    if let Some(window) = space
        .elements()
        .find(|window| window.toplevel().unwrap().wl_surface() == surface)
        .cloned()
    {
        let initial_configure_sent = with_states(surface, |states| {
            states
                .data_map
                .get::<XdgToplevelSurfaceData>()
                .unwrap()
                .lock()
                .unwrap()
                .initial_configure_sent
        });

        if !initial_configure_sent {
            window.toplevel().unwrap().send_configure();
        }
    }

    popups.commit(surface);

    if let Some(popup) = popups.find_popup(surface) {
        match popup {
            PopupKind::Xdg(ref xdg) => {
                if !xdg.is_initial_configure_sent() {
                    xdg.send_configure()
                        .expect("initial popup configure failed");
                }
            }
            PopupKind::InputMethod(_) => {}
        }
    }
}

impl Yawc {
    fn unconstrain_popup(&self, popup: &PopupSurface) {
        let Ok(root) = find_popup_root_surface(&PopupKind::Xdg(popup.clone())) else {
            return;
        };

        let Some(window) = self
            .space
            .elements()
            .find(|window| window.toplevel().unwrap().wl_surface() == &root)
        else {
            return;
        };

        let Some(output) = self.space.outputs().next() else {
            return;
        };
        let Some(output_geometry) = self.space.output_geometry(output) else {
            return;
        };
        let Some(window_geometry) = self.space.element_geometry(window) else {
            return;
        };

        let mut target = output_geometry;
        target.loc -= get_popup_toplevel_coords(&PopupKind::Xdg(popup.clone()));
        target.loc -= window_geometry.loc;

        popup.with_pending_state(|state| {
            state.geometry = state.positioner.get_unconstrained_geometry(target);
        });
    }
}

fn check_grab(
    seat: &Seat<Yawc>,
    surface: &WlSurface,
    serial: Serial,
) -> Option<PointerGrabStartData<Yawc>> {
    let pointer = seat.get_pointer()?;

    if !pointer.has_grab(serial) {
        return None;
    }

    let start_data = pointer.grab_start_data()?;
    let (focus, _) = start_data.focus.as_ref()?;

    if !focus.id().same_client_as(&surface.id()) {
        return None;
    }

    Some(start_data)
}

#[derive(Clone, Debug, Default)]
pub struct WindowMetadata {
    pub title: String,
    pub app_id: Option<String>,
}

fn read_toplevel_metadata(surface: &ToplevelSurface) -> WindowMetadata {
    compositor::with_states(surface.wl_surface(), |states| {
        let role = states
            .data_map
            .get::<XdgToplevelSurfaceData>()
            .unwrap()
            .lock()
            .unwrap();

        WindowMetadata {
            title: role.title.clone().unwrap_or_else(|| "Untitled".to_string()),
            app_id: role.app_id.clone(),
        }
    })
}
