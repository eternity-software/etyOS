use smithay::{
    delegate_xdg_decoration,
    reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode,
    wayland::shell::xdg::{decoration::XdgDecorationHandler, ToplevelSurface},
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
        toplevel.send_configure();
    }

    fn request_mode(&mut self, toplevel: ToplevelSurface, mode: Mode) {
        info!(?mode, "client requested xdg-decoration mode");
        let server_side = matches!(mode, Mode::ServerSide);
        self.windows
            .set_server_decoration(toplevel.wl_surface(), server_side);
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(mode);
        });
        toplevel.send_pending_configure();
    }

    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        self.windows
            .set_server_decoration(toplevel.wl_surface(), true);
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(Mode::ServerSide);
        });
        toplevel.send_pending_configure();
    }
}

delegate_xdg_decoration!(Yawc);
