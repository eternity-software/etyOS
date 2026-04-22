use std::{borrow::Cow, fmt};

use smithay::{
    backend::input::KeyState,
    input::{
        keyboard::{KeyboardTarget, KeysymHandle, ModifiersState},
        pointer::{
            AxisFrame, ButtonEvent, GestureHoldBeginEvent, GestureHoldEndEvent,
            GesturePinchBeginEvent, GesturePinchEndEvent, GesturePinchUpdateEvent,
            GestureSwipeBeginEvent, GestureSwipeEndEvent, GestureSwipeUpdateEvent, MotionEvent,
            PointerTarget, RelativeMotionEvent,
        },
        Seat, SeatHandler,
    },
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{IsAlive, Serial},
    wayland::seat::WaylandFocus,
};

#[cfg(feature = "xwayland")]
use smithay::xwayland::X11Surface;

#[derive(Clone)]
pub enum FocusTarget {
    Wayland(WlSurface),
    #[cfg(feature = "xwayland")]
    X11(X11Surface),
}

impl FocusTarget {
    pub fn wl_surface(&self) -> Option<WlSurface> {
        match self {
            Self::Wayland(surface) => Some(surface.clone()),
            #[cfg(feature = "xwayland")]
            Self::X11(surface) => surface.wl_surface(),
        }
    }

    #[cfg(feature = "xwayland")]
    pub fn x11_surface(&self) -> Option<&X11Surface> {
        match self {
            Self::Wayland(_) => None,
            Self::X11(surface) => Some(surface),
        }
    }
}

impl fmt::Debug for FocusTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Wayland(surface) => f.debug_tuple("Wayland").field(surface).finish(),
            #[cfg(feature = "xwayland")]
            Self::X11(surface) => f
                .debug_struct("X11")
                .field("window_id", &surface.window_id())
                .finish(),
        }
    }
}

impl PartialEq for FocusTarget {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Wayland(lhs), Self::Wayland(rhs)) => lhs == rhs,
            #[cfg(feature = "xwayland")]
            (Self::X11(lhs), Self::X11(rhs)) => lhs.window_id() == rhs.window_id(),
            #[cfg(feature = "xwayland")]
            _ => false,
        }
    }
}

impl Eq for FocusTarget {}

impl IsAlive for FocusTarget {
    fn alive(&self) -> bool {
        match self {
            Self::Wayland(surface) => surface.alive(),
            #[cfg(feature = "xwayland")]
            Self::X11(surface) => surface.alive(),
        }
    }
}

impl WaylandFocus for FocusTarget {
    fn wl_surface(&self) -> Option<Cow<'_, WlSurface>> {
        match self {
            Self::Wayland(surface) => Some(Cow::Borrowed(surface)),
            #[cfg(feature = "xwayland")]
            Self::X11(surface) => surface.wl_surface().map(Cow::Owned),
        }
    }
}

impl<D> KeyboardTarget<D> for FocusTarget
where
    D: SeatHandler<KeyboardFocus = FocusTarget> + 'static,
{
    fn enter(&self, seat: &Seat<D>, data: &mut D, keys: Vec<KeysymHandle<'_>>, serial: Serial) {
        match self {
            Self::Wayland(surface) => KeyboardTarget::enter(surface, seat, data, keys, serial),
            #[cfg(feature = "xwayland")]
            Self::X11(surface) => KeyboardTarget::enter(surface, seat, data, keys, serial),
        }
    }

    fn leave(&self, seat: &Seat<D>, data: &mut D, serial: Serial) {
        match self {
            Self::Wayland(surface) => KeyboardTarget::leave(surface, seat, data, serial),
            #[cfg(feature = "xwayland")]
            Self::X11(surface) => KeyboardTarget::leave(surface, seat, data, serial),
        }
    }

    fn key(
        &self,
        seat: &Seat<D>,
        data: &mut D,
        key: KeysymHandle<'_>,
        state: KeyState,
        serial: Serial,
        time: u32,
    ) {
        match self {
            Self::Wayland(surface) => {
                KeyboardTarget::key(surface, seat, data, key, state, serial, time)
            }
            #[cfg(feature = "xwayland")]
            Self::X11(surface) => {
                KeyboardTarget::key(surface, seat, data, key, state, serial, time)
            }
        }
    }

    fn modifiers(&self, seat: &Seat<D>, data: &mut D, modifiers: ModifiersState, serial: Serial) {
        match self {
            Self::Wayland(surface) => {
                KeyboardTarget::modifiers(surface, seat, data, modifiers, serial)
            }
            #[cfg(feature = "xwayland")]
            Self::X11(surface) => KeyboardTarget::modifiers(surface, seat, data, modifiers, serial),
        }
    }
}

impl<D> PointerTarget<D> for FocusTarget
where
    D: SeatHandler<PointerFocus = FocusTarget> + 'static,
{
    fn enter(&self, seat: &Seat<D>, data: &mut D, event: &MotionEvent) {
        match self {
            Self::Wayland(surface) => PointerTarget::enter(surface, seat, data, event),
            #[cfg(feature = "xwayland")]
            Self::X11(surface) => PointerTarget::enter(surface, seat, data, event),
        }
    }

    fn motion(&self, seat: &Seat<D>, data: &mut D, event: &MotionEvent) {
        match self {
            Self::Wayland(surface) => PointerTarget::motion(surface, seat, data, event),
            #[cfg(feature = "xwayland")]
            Self::X11(surface) => PointerTarget::motion(surface, seat, data, event),
        }
    }

    fn relative_motion(&self, seat: &Seat<D>, data: &mut D, event: &RelativeMotionEvent) {
        match self {
            Self::Wayland(surface) => PointerTarget::relative_motion(surface, seat, data, event),
            #[cfg(feature = "xwayland")]
            Self::X11(surface) => PointerTarget::relative_motion(surface, seat, data, event),
        }
    }

    fn button(&self, seat: &Seat<D>, data: &mut D, event: &ButtonEvent) {
        match self {
            Self::Wayland(surface) => PointerTarget::button(surface, seat, data, event),
            #[cfg(feature = "xwayland")]
            Self::X11(surface) => PointerTarget::button(surface, seat, data, event),
        }
    }

    fn axis(&self, seat: &Seat<D>, data: &mut D, frame: AxisFrame) {
        match self {
            Self::Wayland(surface) => PointerTarget::axis(surface, seat, data, frame),
            #[cfg(feature = "xwayland")]
            Self::X11(surface) => PointerTarget::axis(surface, seat, data, frame),
        }
    }

    fn frame(&self, seat: &Seat<D>, data: &mut D) {
        match self {
            Self::Wayland(surface) => PointerTarget::frame(surface, seat, data),
            #[cfg(feature = "xwayland")]
            Self::X11(surface) => PointerTarget::frame(surface, seat, data),
        }
    }

    fn gesture_swipe_begin(&self, seat: &Seat<D>, data: &mut D, event: &GestureSwipeBeginEvent) {
        match self {
            Self::Wayland(surface) => {
                PointerTarget::gesture_swipe_begin(surface, seat, data, event)
            }
            #[cfg(feature = "xwayland")]
            Self::X11(surface) => PointerTarget::gesture_swipe_begin(surface, seat, data, event),
        }
    }

    fn gesture_swipe_update(&self, seat: &Seat<D>, data: &mut D, event: &GestureSwipeUpdateEvent) {
        match self {
            Self::Wayland(surface) => {
                PointerTarget::gesture_swipe_update(surface, seat, data, event)
            }
            #[cfg(feature = "xwayland")]
            Self::X11(surface) => PointerTarget::gesture_swipe_update(surface, seat, data, event),
        }
    }

    fn gesture_swipe_end(&self, seat: &Seat<D>, data: &mut D, event: &GestureSwipeEndEvent) {
        match self {
            Self::Wayland(surface) => PointerTarget::gesture_swipe_end(surface, seat, data, event),
            #[cfg(feature = "xwayland")]
            Self::X11(surface) => PointerTarget::gesture_swipe_end(surface, seat, data, event),
        }
    }

    fn gesture_pinch_begin(&self, seat: &Seat<D>, data: &mut D, event: &GesturePinchBeginEvent) {
        match self {
            Self::Wayland(surface) => {
                PointerTarget::gesture_pinch_begin(surface, seat, data, event)
            }
            #[cfg(feature = "xwayland")]
            Self::X11(surface) => PointerTarget::gesture_pinch_begin(surface, seat, data, event),
        }
    }

    fn gesture_pinch_update(&self, seat: &Seat<D>, data: &mut D, event: &GesturePinchUpdateEvent) {
        match self {
            Self::Wayland(surface) => {
                PointerTarget::gesture_pinch_update(surface, seat, data, event)
            }
            #[cfg(feature = "xwayland")]
            Self::X11(surface) => PointerTarget::gesture_pinch_update(surface, seat, data, event),
        }
    }

    fn gesture_pinch_end(&self, seat: &Seat<D>, data: &mut D, event: &GesturePinchEndEvent) {
        match self {
            Self::Wayland(surface) => PointerTarget::gesture_pinch_end(surface, seat, data, event),
            #[cfg(feature = "xwayland")]
            Self::X11(surface) => PointerTarget::gesture_pinch_end(surface, seat, data, event),
        }
    }

    fn gesture_hold_begin(&self, seat: &Seat<D>, data: &mut D, event: &GestureHoldBeginEvent) {
        match self {
            Self::Wayland(surface) => PointerTarget::gesture_hold_begin(surface, seat, data, event),
            #[cfg(feature = "xwayland")]
            Self::X11(surface) => PointerTarget::gesture_hold_begin(surface, seat, data, event),
        }
    }

    fn gesture_hold_end(&self, seat: &Seat<D>, data: &mut D, event: &GestureHoldEndEvent) {
        match self {
            Self::Wayland(surface) => PointerTarget::gesture_hold_end(surface, seat, data, event),
            #[cfg(feature = "xwayland")]
            Self::X11(surface) => PointerTarget::gesture_hold_end(surface, seat, data, event),
        }
    }

    fn leave(&self, seat: &Seat<D>, data: &mut D, serial: Serial, time: u32) {
        match self {
            Self::Wayland(surface) => PointerTarget::leave(surface, seat, data, serial, time),
            #[cfg(feature = "xwayland")]
            Self::X11(surface) => PointerTarget::leave(surface, seat, data, serial, time),
        }
    }
}
