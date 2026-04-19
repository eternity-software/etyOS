use smithay::{
    desktop::{Space, Window},
    reexports::wayland_protocols::xdg::shell::server::xdg_toplevel,
    reexports::wayland_server::{protocol::wl_surface::WlSurface, Resource},
    utils::{Logical, Point, Rectangle},
    wayland::{compositor, shell::xdg::SurfaceCachedState},
};

use crate::shell::xdg::WindowMetadata;

pub const TITLEBAR_HEIGHT: i32 = 40;
pub const RESIZE_HITBOX: i32 = 16;
pub const CSD_RESIZE_HITBOX: i32 = 8;
pub const BUTTON_SIZE: i32 = 18;
pub const BUTTON_PADDING: i32 = 12;
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
}

#[derive(Clone)]
pub struct WindowFrame {
    pub window: Window,
    pub frame: Rectangle<i32, Logical>,
    pub header: Rectangle<i32, Logical>,
    pub close_button: Rectangle<i32, Logical>,
    pub active: bool,
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
    Close,
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
            if !uses_server_decoration(tracked) {
                continue;
            }
            let Some(content_geometry) = space.element_geometry(window) else {
                continue;
            };
            frames.push(WindowFrame {
                window: window.clone(),
                frame: frame_geometry(content_geometry),
                header: header_geometry(content_geometry),
                close_button: close_button_geometry(content_geometry),
                active: tracked.active,
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

            let uses_server = uses_server_decoration(tracked);
            let visual_rect = if uses_server {
                let Some(content_geometry) = space.element_geometry(window) else {
                    continue;
                };
                frame_geometry(content_geometry)
            } else {
                let Some(location) = space.element_location(window) else {
                    continue;
                };
                Rectangle::new(location, window.geometry().size)
            };

            let resize_hitbox = if uses_server {
                RESIZE_HITBOX
            } else {
                CSD_RESIZE_HITBOX
            };

            if let Some(edges) = resize_edge_for(visual_rect, position, resize_hitbox)
                .and_then(|edges| allowed_resize_edges(&tracked.window, edges))
            {
                return Some(DecorationHit {
                    window: tracked.window.clone(),
                    action: DecorationAction::Resize(edges),
                });
            }

            if !uses_server {
                if contains(visual_rect, position) {
                    return None;
                }
                continue;
            }

            let Some(content_geometry) = space.element_geometry(window) else {
                continue;
            };
            let frame = WindowFrame {
                window: window.clone(),
                frame: frame_geometry(content_geometry),
                header: header_geometry(content_geometry),
                close_button: close_button_geometry(content_geometry),
                active: tracked.active,
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

            if contains(frame.header, position) {
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
    let Some(surface) = window.toplevel().map(|toplevel| toplevel.wl_surface().clone()) else {
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
) -> Option<ResizeEdge> {
    let hitbox = Rectangle::new(
        (frame.loc.x - hitbox_size, frame.loc.y - hitbox_size).into(),
        (frame.size.w + hitbox_size * 2, frame.size.h + hitbox_size * 2).into(),
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
