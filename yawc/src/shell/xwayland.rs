use std::{process::Stdio, time::Duration};

use smithay::{
    delegate_xwayland_keyboard_grab, delegate_xwayland_shell,
    desktop::Window,
    reexports::{calloop::EventLoop, wayland_server::protocol::wl_surface::WlSurface},
    utils::{Logical, Point, Rectangle, Size, SERIAL_COUNTER},
    wayland::{
        xwayland_keyboard_grab::XWaylandKeyboardGrabHandler,
        xwayland_shell::{XWaylandShellHandler, XWaylandShellState},
    },
    xwayland::{
        xwm::{Reorder, ResizeEdge as X11ResizeEdge, XwmId},
        X11Surface, X11Wm, XWayland, XWaylandEvent, XwmHandler,
    },
};
use tracing::{debug, info, warn};

use crate::{
    focus::FocusTarget,
    state::{ClientState, Yawc},
    window::ResizeEdge,
    CalloopData,
};

pub fn init(event_loop: &mut EventLoop<'static, CalloopData>, data: &mut CalloopData) {
    let (xwayland, client) = match XWayland::spawn(
        &data.display_handle,
        None,
        std::iter::empty::<(&str, &str)>(),
        true,
        Stdio::null(),
        Stdio::null(),
        |user_data| {
            user_data.insert_if_missing(ClientState::default);
        },
    ) {
        Ok(handles) => handles,
        Err(error) => {
            data.state.xwayland_failed = true;
            warn!(?error, "failed to spawn Xwayland");
            return;
        }
    };

    let loop_handle = event_loop.handle();
    let xwm_loop_handle = loop_handle.clone();
    loop_handle
        .insert_source(xwayland, move |event, _, data| match event {
            XWaylandEvent::Ready {
                x11_socket,
                display_number,
            } => {
                std::env::set_var("DISPLAY", format!(":{display_number}"));
                data.state.xwayland_display = Some(display_number);
                match X11Wm::start_wm(xwm_loop_handle.clone(), x11_socket, client.clone()) {
                    Ok(xwm) => {
                        info!(display = display_number, "Xwayland is ready");
                        data.state.xwm = Some(xwm);
                    }
                    Err(error) => {
                        data.state.xwayland_failed = true;
                        warn!(?error, "failed to start Xwayland window manager");
                    }
                }
            }
            XWaylandEvent::Error => {
                data.state.xwayland_failed = true;
                warn!("Xwayland exited during startup");
            }
        })
        .expect("failed to insert Xwayland source");
}

pub fn wait_until_ready(event_loop: &mut EventLoop<'static, CalloopData>, data: &mut CalloopData) {
    if data.state.xwayland_failed || data.state.xwayland_display.is_some() {
        return;
    }

    let deadline = std::time::Instant::now() + Duration::from_millis(1500);
    while data.state.xwayland_display.is_none()
        && !data.state.xwayland_failed
        && std::time::Instant::now() < deadline
    {
        if let Err(error) = event_loop.dispatch(Some(Duration::from_millis(20)), data) {
            warn!(?error, "failed while waiting for Xwayland readiness");
            break;
        }
    }
}

impl XWaylandShellHandler for Yawc {
    fn xwayland_shell_state(&mut self) -> &mut XWaylandShellState {
        &mut self.xwayland_shell_state
    }

    fn surface_associated(&mut self, _xwm: XwmId, _wl_surface: WlSurface, surface: X11Surface) {
        self.windows.set_x11_metadata(&surface);
        self.request_render();
    }
}

impl XWaylandKeyboardGrabHandler for Yawc {
    fn keyboard_focus_for_xsurface(&self, surface: &WlSurface) -> Option<Self::KeyboardFocus> {
        self.space
            .elements()
            .find_map(|window| {
                let x11 = window.x11_surface()?;
                (x11.wl_surface().as_ref() == Some(surface)).then(|| FocusTarget::X11(x11.clone()))
            })
            .or_else(|| Some(FocusTarget::Wayland(surface.clone())))
    }
}

delegate_xwayland_shell!(Yawc);
delegate_xwayland_keyboard_grab!(Yawc);

impl XwmHandler for Yawc {
    fn xwm_state(&mut self, _xwm: XwmId) -> &mut X11Wm {
        self.xwm
            .as_mut()
            .expect("X11Wm state requested before initialization")
    }

    fn new_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    fn new_override_redirect_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    fn map_window_request(&mut self, _xwm: XwmId, _window: X11Surface) {}

    fn mapped_override_redirect_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    fn unmapped_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    fn destroyed_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    fn configure_request(
        &mut self,
        _xwm: XwmId,
        _window: X11Surface,
        _x: Option<i32>,
        _y: Option<i32>,
        _w: Option<u32>,
        _h: Option<u32>,
        _reorder: Option<Reorder>,
    ) {
    }

    fn configure_notify(
        &mut self,
        _xwm: XwmId,
        _window: X11Surface,
        _geometry: Rectangle<i32, Logical>,
        _above: Option<smithay::xwayland::xwm::X11Window>,
    ) {
    }

    fn resize_request(
        &mut self,
        _xwm: XwmId,
        _window: X11Surface,
        _button: u32,
        _resize_edge: X11ResizeEdge,
    ) {
    }

    fn move_request(&mut self, _xwm: XwmId, _window: X11Surface, _button: u32) {}
}

impl XWaylandShellHandler for CalloopData {
    fn xwayland_shell_state(&mut self) -> &mut XWaylandShellState {
        &mut self.state.xwayland_shell_state
    }

    fn surface_associated(&mut self, xwm: XwmId, wl_surface: WlSurface, surface: X11Surface) {
        <Yawc as XWaylandShellHandler>::surface_associated(
            &mut self.state,
            xwm,
            wl_surface,
            surface,
        );
    }
}

impl XwmHandler for CalloopData {
    fn xwm_state(&mut self, _xwm: XwmId) -> &mut X11Wm {
        self.state
            .xwm
            .as_mut()
            .expect("X11Wm event received before XWM state was stored")
    }

    fn new_window(&mut self, _xwm: XwmId, window: X11Surface) {
        debug!(window = window.window_id(), "new X11 window");
    }

    fn new_override_redirect_window(&mut self, _xwm: XwmId, window: X11Surface) {
        debug!(
            window = window.window_id(),
            "new override-redirect X11 window"
        );
    }

    fn map_window_request(&mut self, _xwm: XwmId, surface: X11Surface) {
        if let Err(error) = surface.set_mapped(true) {
            warn!(
                ?error,
                window = surface.window_id(),
                "failed to map X11 window"
            );
            return;
        }
        map_x11_window(&mut self.state, surface, false);
    }

    fn mapped_override_redirect_window(&mut self, _xwm: XwmId, surface: X11Surface) {
        map_x11_window(&mut self.state, surface, true);
    }

    fn unmapped_window(&mut self, _xwm: XwmId, surface: X11Surface) {
        unmap_x11_window(&mut self.state, &surface);
    }

    fn destroyed_window(&mut self, _xwm: XwmId, surface: X11Surface) {
        unmap_x11_window(&mut self.state, &surface);
        self.state.windows.remove_x11(&surface);
    }

    fn configure_request(
        &mut self,
        _xwm: XwmId,
        surface: X11Surface,
        x: Option<i32>,
        y: Option<i32>,
        w: Option<u32>,
        h: Option<u32>,
        _reorder: Option<Reorder>,
    ) {
        let mut geometry = surface.geometry();
        if let Some(x) = x {
            geometry.loc.x = x;
        }
        if let Some(y) = y {
            geometry.loc.y = y;
        }
        if let Some(w) = w {
            geometry.size.w = w as i32;
        }
        if let Some(h) = h {
            geometry.size.h = h as i32;
        }
        if let Err(error) = surface.configure(geometry) {
            warn!(
                ?error,
                window = surface.window_id(),
                "failed to configure X11 window"
            );
        }
        if let Some(window) = self.state.windows.x11_window(&surface) {
            self.state.space.map_element(window, geometry.loc, false);
        }
    }

    fn configure_notify(
        &mut self,
        _xwm: XwmId,
        surface: X11Surface,
        geometry: Rectangle<i32, Logical>,
        _above: Option<smithay::xwayland::xwm::X11Window>,
    ) {
        if let Some(window) = self.state.windows.x11_window(&surface) {
            self.state.space.map_element(window, geometry.loc, false);
            self.state.request_render();
        }
    }

    fn property_notify(
        &mut self,
        _xwm: XwmId,
        surface: X11Surface,
        _property: smithay::xwayland::xwm::WmWindowProperty,
    ) {
        self.state.windows.set_x11_metadata(&surface);
        self.state.request_render();
    }

    fn maximize_request(&mut self, _xwm: XwmId, surface: X11Surface) {
        if let Some(window) = self.state.windows.x11_window(&surface) {
            self.state.set_window_maximized(&window, true);
        }
    }

    fn unmaximize_request(&mut self, _xwm: XwmId, surface: X11Surface) {
        if let Some(window) = self.state.windows.x11_window(&surface) {
            self.state.set_window_maximized(&window, false);
        }
    }

    fn fullscreen_request(&mut self, _xwm: XwmId, surface: X11Surface) {
        if let Some(window) = self.state.windows.x11_window(&surface) {
            self.state.set_window_fullscreen(&window, true, None);
        }
    }

    fn unfullscreen_request(&mut self, _xwm: XwmId, surface: X11Surface) {
        if let Some(window) = self.state.windows.x11_window(&surface) {
            self.state.set_window_fullscreen(&window, false, None);
        }
    }

    fn minimize_request(&mut self, _xwm: XwmId, surface: X11Surface) {
        if let Some(window) = self.state.windows.x11_window(&surface) {
            self.state.set_window_minimized(&window);
        }
    }

    fn unminimize_request(&mut self, _xwm: XwmId, surface: X11Surface) {
        if self.state.windows.x11_window(&surface).is_some() {
            self.state.restore_last_minimized_window();
        }
    }

    fn resize_request(
        &mut self,
        _xwm: XwmId,
        surface: X11Surface,
        button: u32,
        edge: X11ResizeEdge,
    ) {
        if let Some(window) = self.state.windows.x11_window(&surface) {
            let edges = x11_resize_edge(edge);
            let Some(pointer) = self.state.seat.get_pointer() else {
                return;
            };
            let start_data = smithay::input::pointer::GrabStartData {
                focus: None,
                button: x11_button_to_evdev(button),
                location: pointer.current_location(),
            };
            let initial_location = self
                .state
                .space
                .element_location(&window)
                .unwrap_or_else(|| surface.geometry().loc);
            let initial_size = non_empty_size(surface.geometry().size);
            self.state.windows.set_x11_metadata(&surface);
            pointer.set_grab(
                &mut self.state,
                crate::grabs::X11ResizeSurfaceGrab::start(
                    start_data,
                    window,
                    surface.clone(),
                    edges,
                    Rectangle::new(initial_location, initial_size),
                ),
                SERIAL_COUNTER.next_serial(),
                smithay::input::pointer::Focus::Clear,
            );
            debug!(
                ?edges,
                button,
                width = initial_size.w,
                height = initial_size.h,
                "X11 resize request accepted"
            );
        }
    }

    fn move_request(&mut self, _xwm: XwmId, surface: X11Surface, button: u32) {
        if let Some(window) = self.state.windows.x11_window(&surface) {
            let Some(pointer) = self.state.seat.get_pointer() else {
                return;
            };
            let start_data = smithay::input::pointer::GrabStartData {
                focus: None,
                button: x11_button_to_evdev(button),
                location: pointer.current_location(),
            };
            let initial_window_location = self
                .state
                .space
                .element_location(&window)
                .unwrap_or_else(|| surface.geometry().loc);
            pointer.set_grab(
                &mut self.state,
                crate::grabs::MoveSurfaceGrab {
                    start_data,
                    window,
                    initial_window_location,
                },
                SERIAL_COUNTER.next_serial(),
                smithay::input::pointer::Focus::Clear,
            );
        }
    }

    fn disconnected(&mut self, _xwm: XwmId) {
        self.state.xwm = None;
        self.state.xwayland_failed = true;
        warn!("Xwayland window manager disconnected");
    }
}

fn map_x11_window(state: &mut Yawc, surface: X11Surface, override_redirect: bool) {
    let window = state.windows.x11_window(&surface).unwrap_or_else(|| {
        let window = Window::new_x11_window(surface.clone());
        state.windows.insert_x11(window.clone());
        window
    });

    state.windows.set_x11_metadata(&surface);
    let geometry = surface.geometry();
    let size = non_empty_size(geometry.size);
    let location = if override_redirect {
        geometry.loc
    } else {
        centered_x11_location(state, &window, size).unwrap_or(geometry.loc)
    };
    let target = Rectangle::new(location, size);
    if !override_redirect {
        if let Err(error) = surface.configure(target) {
            warn!(
                ?error,
                window = surface.window_id(),
                "failed to configure mapped X11 window"
            );
        }
    }

    state.space.map_element(window.clone(), location, true);
    state.windows.activate_x11(&surface);
    if let Some(keyboard) = state.seat.get_keyboard() {
        keyboard.set_focus(
            state,
            Some(FocusTarget::X11(surface.clone())),
            SERIAL_COUNTER.next_serial(),
        );
    }
    state.request_render();
    info!(
        window = surface.window_id(),
        x = location.x,
        y = location.y,
        width = size.w,
        height = size.h,
        override_redirect,
        "mapped X11 window"
    );
}

fn unmap_x11_window(state: &mut Yawc, surface: &X11Surface) {
    if let Some(window) = state.windows.x11_window(surface) {
        state.space.unmap_elem(&window);
        state.request_render();
    }
}

fn centered_x11_location(
    state: &Yawc,
    window: &Window,
    size: Size<i32, Logical>,
) -> Option<Point<i32, Logical>> {
    let output_geometry = state
        .output_for_window(window)
        .and_then(|output| state.space.output_geometry(&output))
        .or_else(|| state.virtual_output_geometry())?;
    Some(Point::from((
        output_geometry.loc.x + ((output_geometry.size.w - size.w).max(0) / 2),
        output_geometry.loc.y + ((output_geometry.size.h - size.h).max(0) / 2),
    )))
}

fn non_empty_size(size: Size<i32, Logical>) -> Size<i32, Logical> {
    if size.w > 0 && size.h > 0 {
        size
    } else {
        (900, 640).into()
    }
}

fn x11_resize_edge(edge: X11ResizeEdge) -> ResizeEdge {
    match edge {
        X11ResizeEdge::Top => ResizeEdge::TOP,
        X11ResizeEdge::Bottom => ResizeEdge::BOTTOM,
        X11ResizeEdge::Left => ResizeEdge::LEFT,
        X11ResizeEdge::Right => ResizeEdge::RIGHT,
        X11ResizeEdge::TopLeft => ResizeEdge::TOP_LEFT,
        X11ResizeEdge::TopRight => ResizeEdge::TOP_RIGHT,
        X11ResizeEdge::BottomLeft => ResizeEdge::BOTTOM_LEFT,
        X11ResizeEdge::BottomRight => ResizeEdge::BOTTOM_RIGHT,
    }
}

fn x11_button_to_evdev(button: u32) -> u32 {
    match button {
        1 => 0x110,
        2 => 0x112,
        3 => 0x111,
        other => other,
    }
}
