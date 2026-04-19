use smithay::{
    backend::input::{
        AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputBackend, InputEvent,
        KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent,
    },
    input::{
        keyboard::FilterResult,
        pointer::{
            AxisFrame, ButtonEvent, Focus, GrabStartData as PointerGrabStartData, MotionEvent,
        },
    },
    reexports::{
        wayland_server::protocol::wl_surface::WlSurface,
        winit::window::CursorIcon,
    },
    utils::{Rectangle, SERIAL_COUNTER},
};

use crate::{
    grabs::{MoveSurfaceGrab, ResizeSurfaceGrab},
    state::Yawc,
    window::{DecorationAction, ResizeEdge},
};

fn edges_to_cursor(edges: ResizeEdge) -> CursorIcon {
    let top = edges.contains(ResizeEdge::TOP);
    let bottom = edges.contains(ResizeEdge::BOTTOM);
    let left = edges.contains(ResizeEdge::LEFT);
    let right = edges.contains(ResizeEdge::RIGHT);
    match (top, bottom, left, right) {
        (true, false, true, false) => CursorIcon::NwseResize,
        (false, true, false, true) => CursorIcon::NwseResize,
        (true, false, false, true) => CursorIcon::NeswResize,
        (false, true, true, false) => CursorIcon::NeswResize,
        (true, false, false, false) => CursorIcon::RowResize,
        (false, true, false, false) => CursorIcon::RowResize,
        (false, false, true, false) => CursorIcon::ColResize,
        (false, false, false, true) => CursorIcon::ColResize,
        _ => CursorIcon::Default,
    }
}

fn cursor_for_decoration_hit(hit: Option<crate::window::DecorationHit>) -> CursorIcon {
    match hit.map(|hit| hit.action) {
        Some(DecorationAction::Resize(edges)) => edges_to_cursor(edges),
        Some(DecorationAction::Move) => CursorIcon::Move,
        Some(DecorationAction::Close) | None => CursorIcon::Default,
    }
}

impl Yawc {
    pub fn process_input_event<I: InputBackend>(&mut self, event: InputEvent<I>) {
        match event {
            InputEvent::Keyboard { event, .. } => {
                let serial = SERIAL_COUNTER.next_serial();
                let time = Event::time_msec(&event);

                self.seat.get_keyboard().unwrap().input::<(), _>(
                    self,
                    event.key_code(),
                    event.state(),
                    serial,
                    time,
                    |_, _, _| FilterResult::Forward,
                );
            }
            InputEvent::PointerMotion { .. } => {}
            InputEvent::PointerMotionAbsolute { event, .. } => {
                let Some(output) = self.space.outputs().next() else {
                    return;
                };
                let Some(output_geometry) = self.space.output_geometry(output) else {
                    return;
                };

                let location =
                    event.position_transformed(output_geometry.size) + output_geometry.loc.to_f64();
                let serial = SERIAL_COUNTER.next_serial();
                let pointer = self.seat.get_pointer().unwrap();
                let decoration_hit = self.windows.decoration_hit_at(&self.space, location);
                let under = if decoration_hit.is_some() {
                    None
                } else {
                    self.surface_under(location)
                };

                pointer.motion(
                    self,
                    under,
                    &MotionEvent {
                        location,
                        serial,
                        time: event.time_msec(),
                    },
                );
                pointer.frame(self);

                if !pointer.is_grabbed() {
                    self.pending_cursor = cursor_for_decoration_hit(decoration_hit);
                }
            }
            InputEvent::PointerButton { event, .. } => {
                let pointer = self.seat.get_pointer().unwrap();
                let keyboard = self.seat.get_keyboard().unwrap();
                let serial = SERIAL_COUNTER.next_serial();
                let button = event.button_code();
                let button_state = event.state();

                if ButtonState::Pressed == button_state && !pointer.is_grabbed() {
                    if let Some(hit) = self
                        .windows
                        .decoration_hit_at(&self.space, pointer.current_location())
                    {
                        let surface = hit.window.toplevel().unwrap().wl_surface().clone();
                        self.space.raise_element(&hit.window, true);
                        self.windows.activate(&surface);
                        keyboard.set_focus(self, Some(surface), serial);
                        self.send_pending_configures();

                        let button_event = ButtonEvent {
                            button,
                            state: button_state,
                            serial,
                            time: event.time_msec(),
                        };
                        pointer.button(self, &button_event);

                        match hit.action {
                            crate::window::DecorationAction::Move => {
                                let start_data = PointerGrabStartData {
                                    focus: None,
                                    button,
                                    location: pointer.current_location(),
                                };
                                let initial_window_location =
                                    self.space.element_location(&hit.window).unwrap();
                                pointer.set_grab(
                                    self,
                                    MoveSurfaceGrab {
                                        start_data,
                                        window: hit.window,
                                        initial_window_location,
                                    },
                                    serial,
                                    Focus::Clear,
                                );
                            }
                            crate::window::DecorationAction::Resize(edges) => {
                                self.pending_cursor = edges_to_cursor(edges);
                                let start_data = PointerGrabStartData {
                                    focus: None,
                                    button,
                                    location: pointer.current_location(),
                                };
                                let initial_window_location =
                                    self.space.element_location(&hit.window).unwrap();
                                let initial_window_size = hit.window.geometry().size;
                                let grab = ResizeSurfaceGrab::start(
                                    start_data,
                                    hit.window.clone(),
                                    edges,
                                    Rectangle::new(initial_window_location, initial_window_size),
                                );
                                pointer.set_grab(self, grab, serial, Focus::Clear);
                            }
                            crate::window::DecorationAction::Close => {
                                hit.window.toplevel().unwrap().send_close();
                            }
                        }

                        pointer.frame(self);
                        return;
                    } else if let Some((window, _location)) = self
                        .space
                        .element_under(pointer.current_location())
                        .map(|(window, location)| (window.clone(), location))
                    {
                        self.space.raise_element(&window, true);

                        let surface = window.toplevel().unwrap().wl_surface().clone();
                        self.windows.activate(&surface);
                        keyboard.set_focus(self, Some(surface), serial);
                        self.send_pending_configures();
                    } else {
                        self.windows.clear_focus();
                        keyboard.set_focus(self, Option::<WlSurface>::None, serial);
                        self.send_pending_configures();
                    }
                }

                pointer.button(
                    self,
                    &ButtonEvent {
                        button,
                        state: button_state,
                        serial,
                        time: event.time_msec(),
                    },
                );
                pointer.frame(self);

                if !pointer.is_grabbed() {
                    self.pending_cursor = cursor_for_decoration_hit(
                        self.windows
                            .decoration_hit_at(&self.space, pointer.current_location()),
                    );
                }
            }
            InputEvent::PointerAxis { event, .. } => {
                let source = event.source();
                let horizontal_amount = event.amount(Axis::Horizontal).unwrap_or_else(|| {
                    event.amount_v120(Axis::Horizontal).unwrap_or(0.0) * 15.0 / 120.0
                });
                let vertical_amount = event.amount(Axis::Vertical).unwrap_or_else(|| {
                    event.amount_v120(Axis::Vertical).unwrap_or(0.0) * 15.0 / 120.0
                });
                let horizontal_amount_discrete = event.amount_v120(Axis::Horizontal);
                let vertical_amount_discrete = event.amount_v120(Axis::Vertical);

                let mut frame = AxisFrame::new(event.time_msec()).source(source);

                if horizontal_amount != 0.0 {
                    frame = frame.value(Axis::Horizontal, horizontal_amount);
                    if let Some(discrete) = horizontal_amount_discrete {
                        frame = frame.v120(Axis::Horizontal, discrete as i32);
                    }
                }

                if vertical_amount != 0.0 {
                    frame = frame.value(Axis::Vertical, vertical_amount);
                    if let Some(discrete) = vertical_amount_discrete {
                        frame = frame.v120(Axis::Vertical, discrete as i32);
                    }
                }

                if source == AxisSource::Finger {
                    if event.amount(Axis::Horizontal) == Some(0.0) {
                        frame = frame.stop(Axis::Horizontal);
                    }
                    if event.amount(Axis::Vertical) == Some(0.0) {
                        frame = frame.stop(Axis::Vertical);
                    }
                }

                let pointer = self.seat.get_pointer().unwrap();
                pointer.axis(self, frame);
                pointer.frame(self);
            }
            _ => {}
        }
    }
}
