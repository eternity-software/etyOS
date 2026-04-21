use smithay::{
    backend::input::{
        AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputBackend, InputEvent,
        KeyState, KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent, PointerMotionEvent,
    },
    input::{
        keyboard::FilterResult,
        pointer::{
            AxisFrame, ButtonEvent, Focus, GrabStartData as PointerGrabStartData, MotionEvent,
        },
    },
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Rectangle, SERIAL_COUNTER},
};

use crate::{
    config::{HotkeyAction, WindowControlsMode},
    cursor::CursorShape,
    grabs::{MoveSurfaceGrab, ResizeSurfaceGrab},
    state::{TitlebarClick, Yawc},
    window::{DecorationAction, ResizeEdge, SnapSide},
};

const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const TITLEBAR_DOUBLE_CLICK_MS: u32 = 450;
const TITLEBAR_DOUBLE_CLICK_DISTANCE: f64 = 8.0;

fn edges_to_cursor(edges: ResizeEdge) -> CursorShape {
    let top = edges.contains(ResizeEdge::TOP);
    let bottom = edges.contains(ResizeEdge::BOTTOM);
    let left = edges.contains(ResizeEdge::LEFT);
    let right = edges.contains(ResizeEdge::RIGHT);
    match (top, bottom, left, right) {
        (true, false, true, false) => CursorShape::NwseResize,
        (false, true, false, true) => CursorShape::NwseResize,
        (true, false, false, true) => CursorShape::NeswResize,
        (false, true, true, false) => CursorShape::NeswResize,
        (true, false, false, false) => CursorShape::RowResize,
        (false, true, false, false) => CursorShape::RowResize,
        (false, false, true, false) => CursorShape::ColResize,
        (false, false, false, true) => CursorShape::ColResize,
        _ => CursorShape::Default,
    }
}

fn cursor_for_decoration_hit(hit: Option<crate::window::DecorationHit>) -> Option<CursorShape> {
    match hit.map(|hit| hit.action) {
        Some(DecorationAction::Resize(edges)) => Some(edges_to_cursor(edges)),
        Some(DecorationAction::Move) | Some(DecorationAction::Titlebar) => Some(CursorShape::Move),
        Some(DecorationAction::Minimize)
        | Some(DecorationAction::ToggleMaximize)
        | Some(DecorationAction::Close) => Some(CursorShape::Default),
        None => None,
    }
}

fn set_cursor_override(state: &mut Yawc, cursor: Option<CursorShape>) {
    state.compositor_cursor = cursor;
    state.pending_cursor = cursor.unwrap_or(CursorShape::Default);
}

fn titlebar_double_click(
    previous: Option<&TitlebarClick>,
    surface: &WlSurface,
    location: smithay::utils::Point<f64, smithay::utils::Logical>,
    time_msec: u32,
) -> bool {
    let Some(previous) = previous else {
        return false;
    };
    if &previous.surface != surface {
        return false;
    }
    let elapsed = time_msec.saturating_sub(previous.time_msec);
    if elapsed > TITLEBAR_DOUBLE_CLICK_MS {
        return false;
    }
    let dx = previous.location.x - location.x;
    let dy = previous.location.y - location.y;
    (dx * dx + dy * dy).sqrt() <= TITLEBAR_DOUBLE_CLICK_DISTANCE
}

impl Yawc {
    pub fn process_input_event<I: InputBackend>(&mut self, event: InputEvent<I>) {
        match event {
            InputEvent::Keyboard { event, .. } => {
                let serial = SERIAL_COUNTER.next_serial();
                let time = Event::time_msec(&event);
                let pressed = event.state() == KeyState::Pressed;

                self.seat.get_keyboard().unwrap().input::<(), _>(
                    self,
                    event.key_code(),
                    event.state(),
                    serial,
                    time,
                    |state, modifiers, keysym| {
                        if !pressed {
                            return FilterResult::Forward;
                        }

                        let Some(key) = keysym
                            .raw_latin_sym_or_raw_current_sym()
                            .map(|sym| sym.raw())
                        else {
                            return FilterResult::Forward;
                        };

                        state.reload_config_if_changed();
                        if state
                            .config
                            .modifier_hotkey_action(key, *modifiers)
                            .is_some_and(|action| action == HotkeyAction::SwitchKeyboardLayout)
                        {
                            state.cycle_keyboard_layout();
                            return FilterResult::Forward;
                        }

                        match state.config.hotkey_action(key, *modifiers) {
                            Some(HotkeyAction::ToggleMaximize) => {
                                state.toggle_active_window_maximized();
                                FilterResult::Intercept(())
                            }
                            Some(HotkeyAction::SnapLeft) => {
                                state.snap_active_window(SnapSide::Left);
                                FilterResult::Intercept(())
                            }
                            Some(HotkeyAction::SnapRight) => {
                                state.snap_active_window(SnapSide::Right);
                                FilterResult::Intercept(())
                            }
                            Some(HotkeyAction::ToggleFullscreen) => {
                                state.toggle_active_window_fullscreen();
                                FilterResult::Intercept(())
                            }
                            Some(HotkeyAction::ToggleMinimize) => {
                                state.toggle_active_window_minimized();
                                FilterResult::Intercept(())
                            }
                            Some(HotkeyAction::CloseWindow) => {
                                state.close_active_window();
                                FilterResult::Intercept(())
                            }
                            Some(HotkeyAction::KillWindow) => {
                                state.kill_active_window();
                                FilterResult::Intercept(())
                            }
                            Some(HotkeyAction::SwitchKeyboardLayout) => FilterResult::Forward,
                            None => FilterResult::Forward,
                        }
                    },
                );
            }
            InputEvent::PointerMotion { event, .. } => {
                let Some(output) = self.space.outputs().next() else {
                    return;
                };
                let Some(output_geometry) = self.space.output_geometry(output) else {
                    return;
                };

                let pointer = self.seat.get_pointer().unwrap();
                let mut location = pointer.current_location() + event.delta();
                location.x = location.x.clamp(
                    output_geometry.loc.x as f64,
                    (output_geometry.loc.x + output_geometry.size.w - 1) as f64,
                );
                location.y = location.y.clamp(
                    output_geometry.loc.y as f64,
                    (output_geometry.loc.y + output_geometry.size.h - 1) as f64,
                );

                let serial = SERIAL_COUNTER.next_serial();
                let controls_mode = self.config.window_controls();
                let decoration_hit =
                    self.windows
                        .decoration_hit_at(&self.space, location, controls_mode);
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
                    set_cursor_override(self, cursor_for_decoration_hit(decoration_hit));
                }
            }
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
                let controls_mode = self.config.window_controls();
                let decoration_hit =
                    self.windows
                        .decoration_hit_at(&self.space, location, controls_mode);
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
                    set_cursor_override(self, cursor_for_decoration_hit(decoration_hit));
                }
            }
            InputEvent::PointerButton { event, .. } => {
                let pointer = self.seat.get_pointer().unwrap();
                let keyboard = self.seat.get_keyboard().unwrap();
                let serial = SERIAL_COUNTER.next_serial();
                let button = event.button_code();
                let button_state = event.state();
                let controls_mode = self.config.window_controls();
                let pointer_location = pointer.current_location();

                if ButtonState::Released == button_state {
                    if button == BTN_RIGHT {
                        if let Some(pressed_surface) = self.titlebar_right_press.take() {
                            let close_on_release = self
                                .windows
                                .decoration_hit_at(&self.space, pointer_location, controls_mode)
                                .filter(|hit| {
                                    matches!(hit.action, DecorationAction::Titlebar)
                                        && hit
                                            .window
                                            .toplevel()
                                            .map(|toplevel| {
                                                toplevel.wl_surface() == &pressed_surface
                                            })
                                            .unwrap_or(false)
                                })
                                .map(|hit| hit.window)
                                .filter(|_| controls_mode == WindowControlsMode::Gestures);

                            if let Some(window) = close_on_release {
                                self.windows.clear_decoration_pressed();
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
                                self.close_window(&window);
                                return;
                            }

                            self.windows.clear_titlebar_close_pressed();
                            self.windows.clear_decoration_pressed();
                        }
                    }
                    self.titlebar_right_press = None;
                    self.windows.clear_titlebar_close_pressed();
                    self.windows.clear_decoration_pressed();
                }

                if ButtonState::Pressed == button_state && !pointer.is_grabbed() {
                    if let Some(hit) =
                        self.windows
                            .decoration_hit_at(&self.space, pointer_location, controls_mode)
                    {
                        let surface = hit.window.toplevel().unwrap().wl_surface().clone();
                        self.space.raise_element(&hit.window, true);
                        self.windows.activate(&surface);
                        if matches!(hit.action, DecorationAction::Titlebar)
                            && controls_mode == WindowControlsMode::Gestures
                            && button == BTN_RIGHT
                        {
                            self.windows.set_titlebar_close_pressed(&surface, true);
                            self.titlebar_right_press = Some(surface.clone());
                        } else {
                            self.titlebar_right_press = None;
                            self.windows.clear_titlebar_close_pressed();
                            self.windows.set_decoration_pressed(&surface, true);
                        }
                        keyboard.set_focus(self, Some(surface.clone()), serial);
                        self.send_pending_configures();

                        let button_event = ButtonEvent {
                            button,
                            state: button_state,
                            serial,
                            time: event.time_msec(),
                        };
                        pointer.button(self, &button_event);

                        match hit.action {
                            crate::window::DecorationAction::Titlebar => {
                                if button == BTN_RIGHT
                                    && controls_mode == WindowControlsMode::Gestures
                                {
                                    pointer.frame(self);
                                    return;
                                }

                                if button == BTN_LEFT {
                                    if titlebar_double_click(
                                        self.last_titlebar_click.as_ref(),
                                        hit.window.toplevel().unwrap().wl_surface(),
                                        pointer_location,
                                        event.time_msec(),
                                    ) {
                                        self.last_titlebar_click = None;
                                        self.windows.clear_decoration_pressed();
                                        self.toggle_window_maximized(&hit.window);
                                        pointer.frame(self);
                                        return;
                                    }

                                    self.last_titlebar_click = Some(TitlebarClick {
                                        surface: hit
                                            .window
                                            .toplevel()
                                            .unwrap()
                                            .wl_surface()
                                            .clone(),
                                        time_msec: event.time_msec(),
                                        location: pointer_location,
                                    });

                                    if self.windows.is_maximized(&surface) {
                                        pointer.frame(self);
                                        return;
                                    }

                                    set_cursor_override(self, Some(CursorShape::Move));
                                    if let Some(toplevel) = hit.window.toplevel() {
                                        self.windows.clear_snap(toplevel.wl_surface());
                                    }
                                    let start_data = PointerGrabStartData {
                                        focus: None,
                                        button,
                                        location: pointer_location,
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
                            }
                            crate::window::DecorationAction::Move => {
                                if button != BTN_LEFT {
                                    pointer.frame(self);
                                    return;
                                }
                                set_cursor_override(self, Some(CursorShape::Move));
                                if let Some(toplevel) = hit.window.toplevel() {
                                    self.windows.clear_snap(toplevel.wl_surface());
                                }
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
                                if button != BTN_LEFT {
                                    pointer.frame(self);
                                    return;
                                }
                                set_cursor_override(self, Some(edges_to_cursor(edges)));
                                let start_data = PointerGrabStartData {
                                    focus: None,
                                    button,
                                    location: pointer.current_location(),
                                };
                                if let Some(toplevel) = hit.window.toplevel() {
                                    self.windows.clear_snap(toplevel.wl_surface());
                                    self.windows.set_resizing(toplevel.wl_surface(), true);
                                }
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
                                if button == BTN_LEFT {
                                    self.close_window(&hit.window);
                                }
                            }
                            crate::window::DecorationAction::Minimize => {
                                if button == BTN_LEFT {
                                    self.set_window_minimized(&hit.window);
                                }
                            }
                            crate::window::DecorationAction::ToggleMaximize => {
                                if button == BTN_LEFT {
                                    self.toggle_window_maximized(&hit.window);
                                }
                            }
                        }

                        pointer.frame(self);
                        return;
                    } else if let Some((window, _location)) = self
                        .space
                        .element_under(pointer.current_location())
                        .map(|(window, location)| (window.clone(), location))
                    {
                        self.windows.clear_decoration_pressed();
                        self.windows.clear_titlebar_close_pressed();
                        self.titlebar_right_press = None;
                        self.space.raise_element(&window, true);

                        let surface = window.toplevel().unwrap().wl_surface().clone();
                        self.windows.activate(&surface);
                        keyboard.set_focus(self, Some(surface), serial);
                        self.send_pending_configures();
                    } else {
                        self.windows.clear_decoration_pressed();
                        self.windows.clear_titlebar_close_pressed();
                        self.titlebar_right_press = None;
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
                    set_cursor_override(
                        self,
                        cursor_for_decoration_hit(self.windows.decoration_hit_at(
                            &self.space,
                            pointer.current_location(),
                            controls_mode,
                        )),
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
