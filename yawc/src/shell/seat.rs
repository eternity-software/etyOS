use smithay::{
    backend::input::TabletToolDescriptor,
    delegate_cursor_shape, delegate_data_device, delegate_output, delegate_seat,
    input::{pointer::CursorImageStatus, Seat, SeatHandler, SeatState},
    reexports::wayland_server::{
        protocol::{wl_data_source::WlDataSource, wl_surface::WlSurface},
        Resource,
    },
    wayland::{
        output::OutputHandler,
        selection::{
            data_device::{
                set_data_device_focus, ClientDndGrabHandler, DataDeviceHandler, DataDeviceState,
                ServerDndGrabHandler,
            },
            SelectionHandler,
        },
        tablet_manager::TabletSeatHandler,
    },
};

use crate::focus::FocusTarget;
use crate::state::Yawc;

impl SeatHandler for Yawc {
    type KeyboardFocus = FocusTarget;
    type PointerFocus = FocusTarget;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seat_state
    }

    fn cursor_image(&mut self, _seat: &Seat<Self>, image: CursorImageStatus) {
        self.cursor_image = image;
    }

    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&FocusTarget>) {
        let surface = focused.and_then(FocusTarget::wl_surface);
        let client = surface
            .as_ref()
            .and_then(|surface| self.display_handle.get_client(surface.id()).ok());
        set_data_device_focus(&self.display_handle, seat, client);
    }
}

impl SelectionHandler for Yawc {
    type SelectionUserData = ();
}

impl DataDeviceHandler for Yawc {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device_state
    }
}

impl ClientDndGrabHandler for Yawc {
    fn started(
        &mut self,
        _source: Option<WlDataSource>,
        icon: Option<WlSurface>,
        _seat: Seat<Self>,
    ) {
        self.dnd_icon = icon;
    }

    fn dropped(&mut self, _target: Option<WlSurface>, _validated: bool, _seat: Seat<Self>) {
        self.dnd_icon = None;
    }
}
impl ServerDndGrabHandler for Yawc {}
impl OutputHandler for Yawc {}
impl TabletSeatHandler for Yawc {
    fn tablet_tool_image(&mut self, _tool: &TabletToolDescriptor, image: CursorImageStatus) {
        self.cursor_image = image;
    }
}

delegate_seat!(Yawc);
delegate_cursor_shape!(Yawc);
delegate_data_device!(Yawc);
delegate_output!(Yawc);
