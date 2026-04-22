use smithay::{
    desktop::Window,
    input::pointer::{
        AxisFrame, ButtonEvent, GestureHoldBeginEvent, GestureHoldEndEvent, GesturePinchBeginEvent,
        GesturePinchEndEvent, GesturePinchUpdateEvent, GestureSwipeBeginEvent,
        GestureSwipeEndEvent, GestureSwipeUpdateEvent, GrabStartData as PointerGrabStartData,
        MotionEvent, PointerGrab, PointerInnerHandle, RelativeMotionEvent,
    },
    utils::{Logical, Point, Rectangle},
    xwayland::X11Surface,
};
use tracing::warn;

use crate::{focus::FocusTarget, state::Yawc, window::ResizeEdge};

pub struct X11ResizeSurfaceGrab {
    start_data: PointerGrabStartData<Yawc>,
    window: Window,
    surface: X11Surface,
    edges: ResizeEdge,
    initial_rect: Rectangle<i32, Logical>,
}

impl X11ResizeSurfaceGrab {
    pub fn start(
        start_data: PointerGrabStartData<Yawc>,
        window: Window,
        surface: X11Surface,
        edges: ResizeEdge,
        initial_rect: Rectangle<i32, Logical>,
    ) -> Self {
        Self {
            start_data,
            window,
            surface,
            edges,
            initial_rect,
        }
    }
}

impl PointerGrab<Yawc> for X11ResizeSurfaceGrab {
    fn motion(
        &mut self,
        data: &mut Yawc,
        handle: &mut PointerInnerHandle<'_, Yawc>,
        _focus: Option<(FocusTarget, Point<f64, Logical>)>,
        event: &MotionEvent,
    ) {
        handle.motion(data, None, event);

        let delta = event.location - self.start_data.location;
        let mut rect = self.initial_rect;
        if self.edges.intersects(ResizeEdge::LEFT) {
            let new_x = (self.initial_rect.loc.x as f64 + delta.x) as i32;
            let right = self.initial_rect.loc.x + self.initial_rect.size.w;
            rect.loc.x = new_x.min(right - 1);
            rect.size.w = (right - rect.loc.x).max(1);
        }
        if self.edges.intersects(ResizeEdge::RIGHT) {
            rect.size.w = (self.initial_rect.size.w as f64 + delta.x) as i32;
        }
        if self.edges.intersects(ResizeEdge::TOP) {
            let new_y = (self.initial_rect.loc.y as f64 + delta.y) as i32;
            let bottom = self.initial_rect.loc.y + self.initial_rect.size.h;
            rect.loc.y = new_y.min(bottom - 1);
            rect.size.h = (bottom - rect.loc.y).max(1);
        }
        if self.edges.intersects(ResizeEdge::BOTTOM) {
            rect.size.h = (self.initial_rect.size.h as f64 + delta.y) as i32;
        }
        rect.size.w = rect.size.w.max(1);
        rect.size.h = rect.size.h.max(1);

        if let Err(error) = self.surface.configure(rect) {
            warn!(?error, "failed to resize X11 window");
            return;
        }
        data.space.map_element(self.window.clone(), rect.loc, true);
    }

    fn relative_motion(
        &mut self,
        data: &mut Yawc,
        handle: &mut PointerInnerHandle<'_, Yawc>,
        focus: Option<(FocusTarget, Point<f64, Logical>)>,
        event: &RelativeMotionEvent,
    ) {
        handle.relative_motion(data, focus, event);
    }

    fn button(
        &mut self,
        data: &mut Yawc,
        handle: &mut PointerInnerHandle<'_, Yawc>,
        event: &ButtonEvent,
    ) {
        handle.button(data, event);
        if !handle.current_pressed().contains(&self.start_data.button) {
            handle.unset_grab(self, data, event.serial, event.time, true);
        }
    }

    fn axis(
        &mut self,
        data: &mut Yawc,
        handle: &mut PointerInnerHandle<'_, Yawc>,
        details: AxisFrame,
    ) {
        handle.axis(data, details);
    }

    fn frame(&mut self, data: &mut Yawc, handle: &mut PointerInnerHandle<'_, Yawc>) {
        handle.frame(data);
    }

    fn gesture_swipe_begin(
        &mut self,
        data: &mut Yawc,
        handle: &mut PointerInnerHandle<'_, Yawc>,
        event: &GestureSwipeBeginEvent,
    ) {
        handle.gesture_swipe_begin(data, event);
    }

    fn gesture_swipe_update(
        &mut self,
        data: &mut Yawc,
        handle: &mut PointerInnerHandle<'_, Yawc>,
        event: &GestureSwipeUpdateEvent,
    ) {
        handle.gesture_swipe_update(data, event);
    }

    fn gesture_swipe_end(
        &mut self,
        data: &mut Yawc,
        handle: &mut PointerInnerHandle<'_, Yawc>,
        event: &GestureSwipeEndEvent,
    ) {
        handle.gesture_swipe_end(data, event);
    }

    fn gesture_pinch_begin(
        &mut self,
        data: &mut Yawc,
        handle: &mut PointerInnerHandle<'_, Yawc>,
        event: &GesturePinchBeginEvent,
    ) {
        handle.gesture_pinch_begin(data, event);
    }

    fn gesture_pinch_update(
        &mut self,
        data: &mut Yawc,
        handle: &mut PointerInnerHandle<'_, Yawc>,
        event: &GesturePinchUpdateEvent,
    ) {
        handle.gesture_pinch_update(data, event);
    }

    fn gesture_pinch_end(
        &mut self,
        data: &mut Yawc,
        handle: &mut PointerInnerHandle<'_, Yawc>,
        event: &GesturePinchEndEvent,
    ) {
        handle.gesture_pinch_end(data, event);
    }

    fn gesture_hold_begin(
        &mut self,
        data: &mut Yawc,
        handle: &mut PointerInnerHandle<'_, Yawc>,
        event: &GestureHoldBeginEvent,
    ) {
        handle.gesture_hold_begin(data, event);
    }

    fn gesture_hold_end(
        &mut self,
        data: &mut Yawc,
        handle: &mut PointerInnerHandle<'_, Yawc>,
        event: &GestureHoldEndEvent,
    ) {
        handle.gesture_hold_end(data, event);
    }

    fn start_data(&self) -> &PointerGrabStartData<Yawc> {
        &self.start_data
    }

    fn unset(&mut self, _data: &mut Yawc) {}
}
