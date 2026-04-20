use smithay::{
    desktop::{Space, Window},
    reexports::wayland_protocols::xdg::shell::server::xdg_toplevel,
    reexports::wayland_server::{protocol::wl_surface::WlSurface, Resource},
    utils::{Logical, Point, Rectangle},
    wayland::{compositor, shell::xdg::SurfaceCachedState},
};

use crate::shell::xdg::WindowMetadata;

pub const TITLEBAR_HEIGHT: i32 = 40;
pub const RESIZE_HITBOX: i32 = 6;
pub const TOP_RESIZE_HITBOX: i32 = 10;
pub const CSD_RESIZE_HITBOX: i32 = 4;
pub const CSD_TOP_RESIZE_HITBOX: i32 = 8;
pub const BUTTON_SIZE: i32 = 18;
pub const BUTTON_PADDING: i32 = 12;
const BUTTON_STEP: i32 = BUTTON_SIZE + BUTTON_PADDING;
#[cfg_attr(not(feature = "winit-backend"), allow(dead_code))]
pub const FRAME_RADIUS: i32 = 18;

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub struct ResizeEdge: u32 {
        const TOP = 0b0001;
        const BOTTOM = 0b0010;
        const LEFT = 0b0100;
        const RIGHT = 0b1000;
        const TOP_LEFT = Self::TOP.bits() | Self::LEFT.bits();
        const TOP_RIGHT = Self::TOP.bits() | Self::RIGHT.bits();
        const BOTTOM_LEFT = Self::BOTTOM.bits() | Self::LEFT.bits();
        const BOTTOM_RIGHT = Self::BOTTOM.bits() | Self::RIGHT.bits();
    }
}

impl From<xdg_toplevel::ResizeEdge> for ResizeEdge {
    fn from(value: xdg_toplevel::ResizeEdge) -> Self {
        Self::from_bits(value as u32).unwrap()
    }
}

#[derive(Clone)]
pub struct TrackedWindow {
    pub window: Window,
    pub active: bool,
    pub title: String,
    pub app_id: Option<String>,
    pub server_decoration: bool,
    pub decoration_negotiated: bool,
    pub maximized: bool,
    pub maximized_server_decoration: Option<bool>,
    pub minimized: bool,
    pub fullscreen: bool,
    pub resizing: bool,
    pub restore_rect: Option<Rectangle<i32, Logical>>,
    pub minimized_rect: Option<Rectangle<i32, Logical>>,
    pub snap_side: Option<SnapSide>,
    pub snap_restore_rect: Option<Rectangle<i32, Logical>>,
}

#[derive(Clone)]
pub struct WindowFrame {
    pub window: Window,
    pub frame: Rectangle<i32, Logical>,
    pub header: Rectangle<i32, Logical>,
    pub minimize_button: Rectangle<i32, Logical>,
    pub maximize_button: Rectangle<i32, Logical>,
    pub close_button: Rectangle<i32, Logical>,
    pub active: bool,
    pub maximized: bool,
    pub fullscreen: bool,
    pub resizing: bool,
    pub title: String,
    pub app_id: Option<String>,
}

#[derive(Clone)]
pub struct DecorationHit {
    pub window: Window,
    pub action: DecorationAction,
}

#[derive(Clone, Copy)]
pub enum DecorationAction {
    Move,
    Resize(ResizeEdge),
    Minimize,
    ToggleMaximize,
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapSide {
    Left,
    Right,
}

#[derive(Default)]
pub struct WindowStore {
    windows: Vec<TrackedWindow>,
}

impl WindowStore {
    pub fn insert(&mut self, window: Window) -> Point<i32, Logical> {
        let index = self.windows.len() as i32;
        let location = Point::from((48 + 48 * index, 48 + 48 * index));

        self.windows.push(TrackedWindow {
            window,
            active: false,
            title: "Untitled".to_string(),
            app_id: None,
            server_decoration: true,
            decoration_negotiated: false,
            maximized: false,
            maximized_server_decoration: None,
            minimized: false,
            fullscreen: false,
            resizing: false,
            restore_rect: None,
            minimized_rect: None,
            snap_side: None,
            snap_restore_rect: None,
        });

        location
    }

    pub fn len(&self) -> usize {
        self.windows.len()
    }

    pub fn activate(&mut self, surface: &WlSurface) {
        for tracked in &mut self.windows {
            let is_active = tracked
                .window
                .toplevel()
                .map(|toplevel| toplevel.wl_surface() == surface)
                .unwrap_or(false);

            tracked.active = is_active;
            tracked.window.set_activated(is_active);
        }
    }

    pub fn clear_focus(&mut self) {
        for tracked in &mut self.windows {
            tracked.active = false;
            tracked.window.set_activated(false);
        }
    }

    pub fn active_window(&self) -> Option<Window> {
        self.windows
            .iter()
            .find(|tracked| tracked.active)
            .map(|tracked| tracked.window.clone())
    }

    pub fn last_minimized_window(&self) -> Option<Window> {
        self.windows
            .iter()
            .rev()
            .find(|tracked| tracked.minimized)
            .map(|tracked| tracked.window.clone())
    }

    pub fn prune_dead(&mut self) {
        self.windows.retain(|tracked| {
            tracked
                .window
                .toplevel()
                .map(|toplevel| toplevel.wl_surface().is_alive())
                .unwrap_or(false)
        });
    }

    pub fn set_metadata(&mut self, surface: &WlSurface, metadata: WindowMetadata) {
        if let Some(tracked) = self.find_mut(surface) {
            tracked.title = metadata.title;
            tracked.app_id = metadata.app_id;
        }
    }

    pub fn remove(&mut self, surface: &WlSurface) {
        self.windows
            .retain(|tracked| !same_surface(tracked, surface));
    }

    pub fn set_server_decoration(&mut self, surface: &WlSurface, enabled: bool) {
        if let Some(tracked) = self.find_mut(surface) {
            tracked.server_decoration = enabled;
            tracked.decoration_negotiated = true;
        }
    }

    pub fn set_fullscreen(
        &mut self,
        surface: &WlSurface,
        fullscreen: bool,
        restore_rect: Option<Rectangle<i32, Logical>>,
    ) {
        if let Some(tracked) = self.find_mut(surface) {
            tracked.fullscreen = fullscreen;
            if fullscreen {
                tracked.maximized = false;
                tracked.snap_side = None;
                tracked.snap_restore_rect = None;
            }
            tracked.restore_rect = fullscreen.then(|| restore_rect).flatten();
        }
    }

    pub fn set_maximized(
        &mut self,
        surface: &WlSurface,
        maximized: bool,
        restore_rect: Option<Rectangle<i32, Logical>>,
        server_decoration: Option<bool>,
    ) {
        if let Some(tracked) = self.find_mut(surface) {
            tracked.maximized = maximized;
            tracked.maximized_server_decoration = maximized.then_some(server_decoration).flatten();
            if maximized {
                tracked.fullscreen = false;
                tracked.snap_side = None;
                tracked.snap_restore_rect = None;
            }
            tracked.restore_rect = maximized.then(|| restore_rect).flatten();
        }
    }

    pub fn is_maximized(&self, surface: &WlSurface) -> bool {
        self.find(surface)
            .map(|tracked| tracked.maximized)
            .unwrap_or(false)
    }

    pub fn set_minimized(
        &mut self,
        surface: &WlSurface,
        minimized: bool,
        restore_rect: Option<Rectangle<i32, Logical>>,
    ) {
        if let Some(tracked) = self.find_mut(surface) {
            tracked.minimized = minimized;
            if minimized {
                tracked.active = false;
                tracked.window.set_activated(false);
            }
            if let Some(restore_rect) = restore_rect {
                tracked.minimized_rect = Some(restore_rect);
            }
            if !minimized {
                tracked.minimized_rect = None;
            }
        }
    }

    pub fn set_resizing(&mut self, surface: &WlSurface, resizing: bool) {
        if let Some(tracked) = self.find_mut(surface) {
            tracked.resizing = resizing;
        }
    }

    pub fn is_fullscreen(&self, surface: &WlSurface) -> bool {
        self.find(surface)
            .map(|tracked| tracked.fullscreen)
            .unwrap_or(false)
    }

    pub fn restore_rect(&self, surface: &WlSurface) -> Option<Rectangle<i32, Logical>> {
        self.find(surface).and_then(|tracked| tracked.restore_rect)
    }

    pub fn snap_side(&self, surface: &WlSurface) -> Option<SnapSide> {
        self.find(surface).and_then(|tracked| tracked.snap_side)
    }

    pub fn snap_restore_rect(&self, surface: &WlSurface) -> Option<Rectangle<i32, Logical>> {
        self.find(surface)
            .and_then(|tracked| tracked.snap_restore_rect)
    }

    pub fn set_snap(
        &mut self,
        surface: &WlSurface,
        side: SnapSide,
        restore_rect: Option<Rectangle<i32, Logical>>,
    ) {
        if let Some(tracked) = self.find_mut(surface) {
            tracked.snap_side = Some(side);
            if tracked.snap_restore_rect.is_none() {
                tracked.snap_restore_rect = restore_rect;
            }
            tracked.maximized = false;
            tracked.maximized_server_decoration = None;
            tracked.fullscreen = false;
            tracked.restore_rect = None;
        }
    }

    pub fn clear_snap(&mut self, surface: &WlSurface) {
        if let Some(tracked) = self.find_mut(surface) {
            tracked.snap_side = None;
            tracked.snap_restore_rect = None;
        }
    }

    pub fn minimized_rect(&self, surface: &WlSurface) -> Option<Rectangle<i32, Logical>> {
        self.find(surface)
            .and_then(|tracked| tracked.minimized_rect)
    }

    pub fn uses_server_decoration(&self, surface: &WlSurface) -> bool {
        self.find(surface)
            .map(uses_server_decoration)
            .unwrap_or(true)
    }

    pub fn frames(&self, space: &Space<Window>) -> Vec<WindowFrame> {
        let mut frames = Vec::new();

        for window in space.elements() {
            let Some(surface) = window
                .toplevel()
                .map(|toplevel| toplevel.wl_surface().clone())
            else {
                continue;
            };
            let Some(tracked) = self.find(&surface) else {
                continue;
            };
            if tracked.fullscreen || tracked.minimized {
                continue;
            }
            if !uses_server_decoration(tracked) {
                continue;
            }
            let Some(content_geometry) = server_content_geometry(space, window) else {
                continue;
            };
            frames.push(WindowFrame {
                window: window.clone(),
                frame: frame_geometry(content_geometry),
                header: header_geometry(content_geometry),
                minimize_button: minimize_button_geometry(content_geometry),
                maximize_button: maximize_button_geometry(content_geometry),
                close_button: close_button_geometry(content_geometry),
                active: tracked.active,
                maximized: tracked.maximized,
                fullscreen: tracked.fullscreen,
                resizing: tracked.resizing,
                title: tracked.title.clone(),
                app_id: tracked.app_id.clone(),
            });
        }

        frames
    }

    pub fn decoration_hit_at(
        &self,
        space: &Space<Window>,
        position: Point<f64, Logical>,
    ) -> Option<DecorationHit> {
        for window in space.elements().rev() {
            let Some(surface) = window
                .toplevel()
                .map(|toplevel| toplevel.wl_surface().clone())
            else {
                continue;
            };
            let Some(tracked) = self.find(&surface) else {
                continue;
            };
            if tracked.minimized {
                continue;
            }

            if tracked.fullscreen {
                let Some(rect) = window_rect(space, window) else {
                    continue;
                };
                if contains(rect, position) {
                    return None;
                }
                continue;
            }

            let uses_server = uses_server_decoration(tracked);
            let visual_rect = if uses_server {
                let Some(content_geometry) = server_content_geometry(space, window) else {
                    continue;
                };
                frame_geometry(content_geometry)
            } else {
                let Some(rect) = window_rect(space, window) else {
                    continue;
                };
                rect
            };

            let (resize_hitbox, top_resize_hitbox) = if uses_server {
                (RESIZE_HITBOX, TOP_RESIZE_HITBOX)
            } else {
                (CSD_RESIZE_HITBOX, CSD_TOP_RESIZE_HITBOX)
            };

            if !uses_server {
                if contains(visual_rect, position) {
                    // Let smart client-side-decorated apps, such as Firefox,
                    // receive their own border press and issue xdg_toplevel.resize.
                    return None;
                }

                if !tracked.maximized {
                    if let Some(edges) =
                        resize_edge_for(visual_rect, position, resize_hitbox, top_resize_hitbox)
                            .and_then(|edges| allowed_resize_edges(&tracked.window, edges))
                    {
                        return Some(DecorationHit {
                            window: tracked.window.clone(),
                            action: DecorationAction::Resize(edges),
                        });
                    }
                }

                continue;
            }

            if !tracked.maximized {
                if let Some(edges) =
                    resize_edge_for(visual_rect, position, resize_hitbox, top_resize_hitbox)
                        .and_then(|edges| allowed_resize_edges(&tracked.window, edges))
                {
                    return Some(DecorationHit {
                        window: tracked.window.clone(),
                        action: DecorationAction::Resize(edges),
                    });
                }
            }

            let Some(content_geometry) = server_content_geometry(space, window) else {
                continue;
            };
            let frame = WindowFrame {
                window: window.clone(),
                frame: frame_geometry(content_geometry),
                header: header_geometry(content_geometry),
                minimize_button: minimize_button_geometry(content_geometry),
                maximize_button: maximize_button_geometry(content_geometry),
                close_button: close_button_geometry(content_geometry),
                active: tracked.active,
                maximized: tracked.maximized,
                fullscreen: tracked.fullscreen,
                resizing: tracked.resizing,
                title: tracked.title.clone(),
                app_id: tracked.app_id.clone(),
            };

            if !contains(frame.frame, position) {
                continue;
            }

            if contains(frame.close_button, position) {
                return Some(DecorationHit {
                    window: tracked.window.clone(),
                    action: DecorationAction::Close,
                });
            }

            if contains(frame.maximize_button, position) {
                return Some(DecorationHit {
                    window: tracked.window.clone(),
                    action: DecorationAction::ToggleMaximize,
                });
            }

            if contains(frame.minimize_button, position) {
                return Some(DecorationHit {
                    window: tracked.window.clone(),
                    action: DecorationAction::Minimize,
                });
            }

            if !tracked.maximized && contains(frame.header, position) {
                return Some(DecorationHit {
                    window: tracked.window.clone(),
                    action: DecorationAction::Move,
                });
            }

            if contains(frame.frame, position) {
                return None;
            }
        }

        None
    }

    fn find_mut(&mut self, surface: &WlSurface) -> Option<&mut TrackedWindow> {
        self.windows
            .iter_mut()
            .find(|tracked| same_surface(tracked, surface))
    }

    fn find(&self, surface: &WlSurface) -> Option<&TrackedWindow> {
        self.windows
            .iter()
            .find(|tracked| same_surface(tracked, surface))
    }
}

fn same_surface(tracked: &TrackedWindow, surface: &WlSurface) -> bool {
    tracked
        .window
        .toplevel()
        .map(|toplevel| toplevel.wl_surface() == surface)
        .unwrap_or(false)
}

fn uses_server_decoration(tracked: &TrackedWindow) -> bool {
    if tracked.maximized {
        if let Some(server_decoration) = tracked.maximized_server_decoration {
            return server_decoration;
        }
    }

    if tracked.decoration_negotiated {
        return tracked.server_decoration;
    }

    !client_draws_own_decorations(&tracked.window)
}

fn client_draws_own_decorations(window: &Window) -> bool {
    let geometry = window.geometry();
    let bbox = window.bbox();

    geometry.loc != bbox.loc || geometry.size != bbox.size
}

fn allowed_resize_edges(window: &Window, edges: ResizeEdge) -> Option<ResizeEdge> {
    let Some(surface) = window
        .toplevel()
        .map(|toplevel| toplevel.wl_surface().clone())
    else {
        return None;
    };

    let (min_size, max_size) = compositor::with_states(&surface, |states| {
        let mut guard = states.cached_state.get::<SurfaceCachedState>();
        let data = guard.current();
        (data.min_size, data.max_size)
    });

    let width_resizable = max_size.w == 0 || min_size.w != max_size.w;
    let height_resizable = max_size.h == 0 || min_size.h != max_size.h;

    let mut filtered = edges;
    if !width_resizable {
        filtered.remove(ResizeEdge::LEFT | ResizeEdge::RIGHT);
    }
    if !height_resizable {
        filtered.remove(ResizeEdge::TOP | ResizeEdge::BOTTOM);
    }

    (!filtered.is_empty()).then_some(filtered)
}

fn frame_geometry(content_geometry: Rectangle<i32, Logical>) -> Rectangle<i32, Logical> {
    Rectangle::new(
        (
            content_geometry.loc.x,
            content_geometry.loc.y - TITLEBAR_HEIGHT,
        )
            .into(),
        (
            content_geometry.size.w,
            content_geometry.size.h + TITLEBAR_HEIGHT,
        )
            .into(),
    )
}

fn header_geometry(content_geometry: Rectangle<i32, Logical>) -> Rectangle<i32, Logical> {
    let frame = frame_geometry(content_geometry);
    Rectangle::new(frame.loc, (frame.size.w, TITLEBAR_HEIGHT).into())
}

fn close_button_geometry(content_geometry: Rectangle<i32, Logical>) -> Rectangle<i32, Logical> {
    let header = header_geometry(content_geometry);
    Rectangle::new(
        (
            header.loc.x + header.size.w - BUTTON_PADDING - BUTTON_SIZE,
            header.loc.y + (TITLEBAR_HEIGHT - BUTTON_SIZE) / 2,
        )
            .into(),
        (BUTTON_SIZE, BUTTON_SIZE).into(),
    )
}

fn button_left_of(rect: Rectangle<i32, Logical>, steps: i32) -> Rectangle<i32, Logical> {
    Rectangle::new(
        (rect.loc.x - BUTTON_STEP * steps, rect.loc.y).into(),
        rect.size,
    )
}

fn minimize_button_geometry(content_geometry: Rectangle<i32, Logical>) -> Rectangle<i32, Logical> {
    button_left_of(close_button_geometry(content_geometry), 2)
}

fn maximize_button_geometry(content_geometry: Rectangle<i32, Logical>) -> Rectangle<i32, Logical> {
    button_left_of(close_button_geometry(content_geometry), 1)
}

fn server_content_geometry(
    space: &Space<Window>,
    window: &Window,
) -> Option<Rectangle<i32, Logical>> {
    let location = space.element_location(window)?;
    Some(Rectangle::new(location, window.geometry().size))
}

fn window_rect(space: &Space<Window>, window: &Window) -> Option<Rectangle<i32, Logical>> {
    let location = space.element_location(window)?;
    let bbox = window.bbox();
    Some(Rectangle::new(location + bbox.loc, bbox.size))
}

fn contains(rect: Rectangle<i32, Logical>, position: Point<f64, Logical>) -> bool {
    position.x >= rect.loc.x as f64
        && position.x < (rect.loc.x + rect.size.w) as f64
        && position.y >= rect.loc.y as f64
        && position.y < (rect.loc.y + rect.size.h) as f64
}

fn resize_edge_for(
    frame: Rectangle<i32, Logical>,
    position: Point<f64, Logical>,
    hitbox_size: i32,
    top_hitbox_size: i32,
) -> Option<ResizeEdge> {
    let hitbox = Rectangle::new(
        (frame.loc.x - hitbox_size, frame.loc.y - top_hitbox_size).into(),
        (
            frame.size.w + hitbox_size * 2,
            frame.size.h + top_hitbox_size + hitbox_size,
        )
            .into(),
    );
    if !contains(hitbox, position) {
        return None;
    }

    let left = position.x < (frame.loc.x + hitbox_size) as f64;
    let right = position.x >= (frame.loc.x + frame.size.w - hitbox_size) as f64;
    let top = position.y < (frame.loc.y + hitbox_size) as f64;
    let bottom = position.y >= (frame.loc.y + frame.size.h - hitbox_size) as f64;

    let mut edges = ResizeEdge::empty();
    if left {
        edges |= ResizeEdge::LEFT;
    }
    if right {
        edges |= ResizeEdge::RIGHT;
    }
    if top {
        edges |= ResizeEdge::TOP;
    }
    if bottom {
        edges |= ResizeEdge::BOTTOM;
    }

    (!edges.is_empty()).then_some(edges)
}
