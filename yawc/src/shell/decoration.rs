use smithay::{
    delegate_xdg_decoration,
    reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode,
    wayland::{
        compositor::with_states,
        shell::xdg::{decoration::XdgDecorationHandler, ToplevelSurface, XdgToplevelSurfaceData},
    },
};
use tracing::info;

use crate::state::Yawc;

impl XdgDecorationHandler for Yawc {
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        self.windows
            .set_server_decoration(toplevel.wl_surface(), true);
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(Mode::ServerSide);
        });
        if initial_configure_sent(&toplevel) {
            let configure_serial = toplevel.send_pending_configure();
            if configure_serial.is_some() {
                self.flush_wayland_clients();
            }
        }
    }

    fn request_mode(&mut self, toplevel: ToplevelSurface, mode: Mode) {
        info!(?mode, "client requested xdg-decoration mode");
        let server_side = matches!(mode, Mode::ServerSide);
        self.windows
            .set_server_decoration(toplevel.wl_surface(), server_side);
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(mode);
        });
        if !initial_configure_sent(&toplevel) {
            return;
        }
        let configure_serial = toplevel.send_pending_configure();
        if configure_serial.is_some() {
            self.flush_wayland_clients();
        }
    }

    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        self.windows
            .set_server_decoration(toplevel.wl_surface(), true);
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(Mode::ServerSide);
        });
        if !initial_configure_sent(&toplevel) {
            return;
        }
        let configure_serial = toplevel.send_pending_configure();
        if configure_serial.is_some() {
            self.flush_wayland_clients();
        }
    }
}

delegate_xdg_decoration!(Yawc);

fn initial_configure_sent(toplevel: &ToplevelSurface) -> bool {
    with_states(toplevel.wl_surface(), |states| {
        states
            .data_map
            .get::<XdgToplevelSurfaceData>()
            .map(|data| data.lock().unwrap().initial_configure_sent)
            .unwrap_or(false)
    })
}
