use std::time::{Duration, Instant};

#[cfg(feature = "xwayland")]
use smithay::xwayland::X11Surface;
use smithay::{
    desktop::{Space, Window},
    reexports::wayland_protocols::xdg::shell::server::xdg_toplevel,
    reexports::wayland_server::{protocol::wl_surface::WlSurface, Resource},
    utils::{Logical, Point, Rectangle},
    wayland::{compositor, shell::xdg::SurfaceCachedState},
};

use crate::{
    config::{AnimationConfig, WindowControlsMode},
    shell::xdg::WindowMetadata,
};

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
const MAP_ANIMATION_START_SCALE: f64 = 0.94;
const DECORATION_ACTIVE_OPACITY: f32 = 1.0;
const DECORATION_INACTIVE_OPACITY: f32 = 0.64;
const DECORATION_PRESSED_OPACITY: f32 = 0.82;
const TITLEBAR_CLOSE_TINT_MS: u64 = 180;
const TITLEBAR_CLOSE_TINT_TARGET: f32 = 0.24;
const CLOSE_RESTORE_GRACE_MS: u64 = 180;

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
    pub surface: Option<WlSurface>,
    #[cfg(feature = "xwayland")]
    pub x11_window_id: Option<u32>,
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
    pub mapped_at: Instant,
    pub map_animation_started: bool,
    pub geometry_animation: Option<GeometryAnimation>,
    pub initial_positioned: bool,
    pub decoration_pressed: bool,
    pub decoration_opacity_from: f32,
    pub decoration_opacity_to: f32,
    pub decoration_opacity_started_at: Instant,
    pub titlebar_close_pressed: bool,
    pub titlebar_close_tint_from: f32,
    pub titlebar_close_tint_to: f32,
    pub titlebar_close_tint_started_at: Instant,
    pub close_animating: bool,
    pub close_sent: bool,
    pub close_destroyed: bool,
    pub close_restoring: bool,
    pub close_started_at: Instant,
    pub close_restore_started_at: Instant,
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
    pub legacy_x11: bool,
    pub animation: WindowAnimation,
    pub decoration_opacity: f32,
    pub close_tint: f32,
    pub controls_mode: WindowControlsMode,
}

#[derive(Clone)]
pub struct DecorationHit {
    pub window: Window,
    pub action: DecorationAction,
}

#[derive(Clone, Copy)]
pub enum DecorationAction {
    Titlebar,
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

#[derive(Clone, Copy, Debug)]
pub struct WindowAnimation {
    pub alpha: f32,
    pub scale: f64,
    pub geometry: Option<GeometryAnimationFrame>,
}

impl Default for WindowAnimation {
    fn default() -> Self {
        Self {
            alpha: 1.0,
            scale: 1.0,
            geometry: None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct GeometryAnimationFrame {
    pub from: Rectangle<i32, Logical>,
    pub to: Rectangle<i32, Logical>,
    pub progress: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct GeometryAnimation {
    pub from: Rectangle<i32, Logical>,
    pub to: Rectangle<i32, Logical>,
    pub started_at: Instant,
}

#[derive(Default)]
pub struct WindowStore {
    windows: Vec<TrackedWindow>,
}

impl WindowStore {
    pub fn insert(&mut self, window: Window) -> Point<i32, Logical> {
        let index = self.windows.len() as i32;
        let location = Point::from((48 + 48 * index, 48 + 48 * index));
        let surface = window
            .toplevel()
            .expect("tracked windows must have an xdg toplevel")
            .wl_surface()
            .clone();

        let now = Instant::now();
        self.windows.push(TrackedWindow {
            window,
            surface: Some(surface),
            #[cfg(feature = "xwayland")]
            x11_window_id: None,
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
            mapped_at: now,
            map_animation_started: false,
            geometry_animation: None,
            initial_positioned: false,
            decoration_pressed: false,
            decoration_opacity_from: DECORATION_INACTIVE_OPACITY,
            decoration_opacity_to: DECORATION_INACTIVE_OPACITY,
            decoration_opacity_started_at: now,
            titlebar_close_pressed: false,
            titlebar_close_tint_from: 0.0,
            titlebar_close_tint_to: 0.0,
            titlebar_close_tint_started_at: now,
            close_animating: false,
            close_sent: false,
            close_destroyed: false,
            close_restoring: false,
            close_started_at: now,
            close_restore_started_at: now,
        });

        location
    }

    #[cfg(feature = "xwayland")]
    pub fn insert_x11(&mut self, window: Window) -> Point<i32, Logical> {
        let index = self.windows.len() as i32;
        let location = Point::from((48 + 48 * index, 48 + 48 * index));
        let surface = window
            .x11_surface()
            .expect("tracked X11 windows need an X11 surface")
            .clone();
        let server_decoration = !surface.is_decorated();
        let now = Instant::now();
        self.windows.push(TrackedWindow {
            window,
            surface: surface.wl_surface(),
            x11_window_id: Some(surface.window_id()),
            active: false,
            title: title_for_x11(&surface),
            app_id: app_id_for_x11(&surface),
            server_decoration,
            decoration_negotiated: true,
            maximized: false,
            maximized_server_decoration: None,
            minimized: false,
            fullscreen: false,
            resizing: false,
            restore_rect: None,
            minimized_rect: None,
            snap_side: None,
            snap_restore_rect: None,
            mapped_at: now,
            map_animation_started: false,
            geometry_animation: None,
            initial_positioned: false,
            decoration_pressed: false,
            decoration_opacity_from: DECORATION_INACTIVE_OPACITY,
            decoration_opacity_to: DECORATION_INACTIVE_OPACITY,
            decoration_opacity_started_at: now,
            titlebar_close_pressed: false,
            titlebar_close_tint_from: 0.0,
            titlebar_close_tint_to: 0.0,
            titlebar_close_tint_started_at: now,
            close_animating: false,
            close_sent: false,
            close_destroyed: false,
            close_restoring: false,
            close_started_at: now,
            close_restore_started_at: now,
        });

        location
    }

    pub fn len(&self) -> usize {
        self.windows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }

    pub fn managed_windows(&self) -> Vec<Window> {
        self.windows
            .iter()
            .map(|tracked| tracked.window.clone())
            .collect()
    }

    pub fn activate(&mut self, surface: &WlSurface) {
        for tracked in &mut self.windows {
            let is_active = same_surface(tracked, surface);

            if tracked.active != is_active {
                tracked.active = is_active;
                tracked.decoration_pressed &= is_active;
                tracked.titlebar_close_pressed &= is_active;
                set_decoration_opacity_target(tracked, decoration_opacity_target(tracked));
                set_titlebar_close_tint_target(tracked, titlebar_close_tint_target(tracked));
                tracked.window.set_activated(is_active);
            } else if !is_active && tracked.decoration_pressed {
                tracked.decoration_pressed = false;
                set_decoration_opacity_target(tracked, decoration_opacity_target(tracked));
            } else if !is_active && tracked.titlebar_close_pressed {
                tracked.titlebar_close_pressed = false;
                set_titlebar_close_tint_target(tracked, titlebar_close_tint_target(tracked));
            }
        }
    }

    pub fn activate_window(&mut self, window: &Window) {
        for tracked in &mut self.windows {
            let is_active = tracked.window == *window;
            if tracked.active != is_active {
                tracked.active = is_active;
                tracked.decoration_pressed &= is_active;
                tracked.titlebar_close_pressed &= is_active;
                set_decoration_opacity_target(tracked, decoration_opacity_target(tracked));
                set_titlebar_close_tint_target(tracked, titlebar_close_tint_target(tracked));
                tracked.window.set_activated(is_active);
            }
        }
    }

    #[cfg(feature = "xwayland")]
    pub fn activate_x11(&mut self, surface: &X11Surface) {
        if let Some(tracked) = self.find_x11_mut(surface) {
            tracked.surface = surface.wl_surface();
        }
        for tracked in &mut self.windows {
            let is_active = tracked.x11_window_id == Some(surface.window_id());
            if tracked.active != is_active {
                tracked.active = is_active;
                tracked.decoration_pressed &= is_active;
                tracked.titlebar_close_pressed &= is_active;
                set_decoration_opacity_target(tracked, decoration_opacity_target(tracked));
                set_titlebar_close_tint_target(tracked, titlebar_close_tint_target(tracked));
                tracked.window.set_activated(is_active);
            }
        }
    }

    pub fn clear_focus(&mut self) {
        for tracked in &mut self.windows {
            if tracked.active {
                tracked.active = false;
                tracked.decoration_pressed = false;
                tracked.titlebar_close_pressed = false;
                set_decoration_opacity_target(tracked, decoration_opacity_target(tracked));
                set_titlebar_close_tint_target(tracked, titlebar_close_tint_target(tracked));
                tracked.window.set_activated(false);
            } else if tracked.decoration_pressed {
                tracked.decoration_pressed = false;
                set_decoration_opacity_target(tracked, decoration_opacity_target(tracked));
            } else if tracked.titlebar_close_pressed {
                tracked.titlebar_close_pressed = false;
                set_titlebar_close_tint_target(tracked, titlebar_close_tint_target(tracked));
            }
        }
    }

    pub fn active_window(&self) -> Option<Window> {
        self.windows
            .iter()
            .find(|tracked| tracked.active)
            .map(|tracked| tracked.window.clone())
    }

    #[cfg(feature = "xwayland")]
    pub fn x11_window(&self, surface: &X11Surface) -> Option<Window> {
        self.windows
            .iter()
            .find(|tracked| tracked.x11_window_id == Some(surface.window_id()))
            .map(|tracked| tracked.window.clone())
    }

    pub fn last_minimized_window(&self) -> Option<Window> {
        self.windows
            .iter()
            .rev()
            .find(|tracked| tracked.minimized)
            .map(|tracked| tracked.window.clone())
    }

    pub fn prune_dead(&mut self, config: AnimationConfig) {
        let close_duration = Duration::from_millis(config.close_ms);
        self.windows.retain(|tracked| {
            if tracked.close_animating {
                if !tracked.close_destroyed && tracked_window_alive(tracked) {
                    return true;
                }

                return config.enabled
                    && config.close_ms > 0
                    && tracked.close_started_at.elapsed() < close_duration
                    && (tracked.close_destroyed || !tracked.close_sent);
            }

            tracked_window_alive(tracked)
        });
    }

    pub fn set_metadata(&mut self, surface: &WlSurface, metadata: WindowMetadata) {
        if let Some(tracked) = self.find_mut(surface) {
            tracked.title = metadata.title;
            tracked.app_id = metadata.app_id;
        }
    }

    #[cfg(feature = "xwayland")]
    pub fn set_x11_metadata(&mut self, surface: &X11Surface) {
        if let Some(tracked) = self.find_x11_mut(surface) {
            tracked.surface = surface.wl_surface();
            tracked.title = title_for_x11(surface);
            tracked.app_id = app_id_for_x11(surface);
            tracked.server_decoration = !surface.is_decorated();
            tracked.decoration_negotiated = true;
        }
    }

    pub fn remove(&mut self, surface: &WlSurface) {
        self.windows
            .retain(|tracked| !same_surface(tracked, surface));
    }

    #[cfg(feature = "xwayland")]
    pub fn remove_x11(&mut self, surface: &X11Surface) {
        self.windows
            .retain(|tracked| tracked.x11_window_id != Some(surface.window_id()));
    }

    pub fn request_destroy_close(&mut self, surface: &WlSurface, config: AnimationConfig) -> bool {
        if !config.enabled || config.close_ms == 0 {
            self.remove(surface);
            return false;
        }

        let now = Instant::now();
        let Some(index) = self
            .windows
            .iter()
            .position(|tracked| same_surface(tracked, surface))
        else {
            return false;
        };

        let tracked = &mut self.windows[index];
        if tracked.close_animating && tracked.close_sent && !tracked.close_destroyed {
            self.windows.remove(index);
            return false;
        }

        {
            tracked.close_animating = true;
            tracked.close_sent = true;
            tracked.close_destroyed = true;
            tracked.close_restoring = false;
            tracked.active = false;
            tracked.decoration_pressed = false;
            tracked.titlebar_close_pressed = false;
            tracked.close_started_at = now;
            set_decoration_opacity_target(tracked, DECORATION_INACTIVE_OPACITY);
            set_titlebar_close_tint_target(tracked, 0.0);
        }

        true
    }

    pub fn request_close(&mut self, surface: &WlSurface, config: AnimationConfig) -> bool {
        if !config.enabled || config.close_ms == 0 {
            return true;
        }

        let now = Instant::now();
        if let Some(tracked) = self.find_mut(surface) {
            tracked.close_animating = true;
            tracked.close_sent = false;
            tracked.close_destroyed = false;
            tracked.close_restoring = false;
            tracked.active = false;
            tracked.decoration_pressed = false;
            tracked.titlebar_close_pressed = false;
            tracked.close_started_at = now;
            set_decoration_opacity_target(tracked, DECORATION_INACTIVE_OPACITY);
            return false;
        }

        true
    }

    pub fn close_requests_ready(&mut self, config: AnimationConfig) -> Vec<Window> {
        if !config.enabled || config.close_ms == 0 {
            for tracked in &mut self.windows {
                if tracked.close_animating || tracked.close_restoring {
                    tracked.close_animating = false;
                    tracked.close_sent = false;
                    tracked.close_destroyed = false;
                    tracked.close_restoring = false;
                    set_decoration_opacity_target(tracked, decoration_opacity_target(tracked));
                    set_titlebar_close_tint_target(tracked, 0.0);
                }
            }
            return Vec::new();
        }

        let close_duration = Duration::from_millis(config.close_ms);
        let restore_duration = close_duration + Duration::from_millis(CLOSE_RESTORE_GRACE_MS);
        let mut windows = Vec::new();
        for tracked in &mut self.windows {
            if tracked.close_animating
                && !tracked.close_destroyed
                && !tracked.close_sent
                && tracked.close_started_at.elapsed() >= close_duration
            {
                tracked.close_sent = true;
                set_titlebar_close_tint_target(tracked, 0.0);
                windows.push(tracked.window.clone());
            } else if tracked.close_animating
                && tracked.close_sent
                && !tracked.close_destroyed
                && tracked.close_started_at.elapsed() >= restore_duration
            {
                tracked.close_animating = false;
                tracked.close_sent = false;
                tracked.close_destroyed = false;
                tracked.close_restoring = true;
                tracked.close_restore_started_at = Instant::now();
                set_decoration_opacity_target(tracked, decoration_opacity_target(tracked));
                set_titlebar_close_tint_target(tracked, 0.0);
            } else if tracked.close_restoring
                && tracked.close_restore_started_at.elapsed() >= close_duration
            {
                tracked.close_restoring = false;
            }
        }
        windows
    }

    pub fn destroyed_close_animations(
        &self,
        config: AnimationConfig,
    ) -> Vec<(WlSurface, WindowAnimation)> {
        if !config.enabled || config.close_ms == 0 {
            return Vec::new();
        }

        let duration = Duration::from_millis(config.close_ms);
        self.windows
            .iter()
            .filter(|tracked| {
                tracked.close_animating
                    && tracked.close_destroyed
                    && tracked.close_started_at.elapsed() < duration
            })
            .filter_map(|tracked| {
                Some((
                    tracked.surface.clone()?,
                    close_animation_for_elapsed(tracked.close_started_at.elapsed(), config),
                ))
            })
            .collect()
    }

    pub fn request_close_window(&mut self, window: &Window, config: AnimationConfig) -> bool {
        if !config.enabled || config.close_ms == 0 {
            return true;
        }

        let now = Instant::now();
        if let Some(tracked) = self.find_window_mut(window) {
            tracked.close_animating = true;
            tracked.close_sent = false;
            tracked.close_destroyed = false;
            tracked.close_restoring = false;
            tracked.active = false;
            tracked.decoration_pressed = false;
            tracked.titlebar_close_pressed = false;
            tracked.close_started_at = now;
            set_decoration_opacity_target(tracked, DECORATION_INACTIVE_OPACITY);
            return false;
        }

        true
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
                tracked.initial_positioned = true;
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
                tracked.initial_positioned = true;
            }
            tracked.restore_rect = maximized.then(|| restore_rect).flatten();
        }
    }

    pub fn is_maximized(&self, surface: &WlSurface) -> bool {
        self.find(surface)
            .map(|tracked| tracked.maximized)
            .unwrap_or(false)
    }

    pub fn is_window_maximized(&self, window: &Window) -> bool {
        self.find_window(window)
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
                tracked.decoration_pressed = false;
                set_decoration_opacity_target(tracked, decoration_opacity_target(tracked));
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

    pub fn set_window_minimized(
        &mut self,
        window: &Window,
        minimized: bool,
        restore_rect: Option<Rectangle<i32, Logical>>,
    ) {
        if let Some(tracked) = self.find_window_mut(window) {
            tracked.minimized = minimized;
            if minimized {
                tracked.active = false;
                tracked.decoration_pressed = false;
                tracked.titlebar_close_pressed = false;
                set_decoration_opacity_target(tracked, decoration_opacity_target(tracked));
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

    pub fn is_resizing(&self, surface: &WlSurface) -> bool {
        self.find(surface)
            .map(|tracked| tracked.resizing)
            .unwrap_or(false)
    }

    pub fn set_decoration_pressed(&mut self, surface: &WlSurface, pressed: bool) {
        if let Some(tracked) = self.find_mut(surface) {
            tracked.decoration_pressed = pressed && tracked.active;
            set_decoration_opacity_target(tracked, decoration_opacity_target(tracked));
        }
    }

    pub fn set_decoration_pressed_window(&mut self, window: &Window, pressed: bool) {
        if let Some(tracked) = self.find_window_mut(window) {
            tracked.decoration_pressed = pressed && tracked.active;
            set_decoration_opacity_target(tracked, decoration_opacity_target(tracked));
        }
    }

    pub fn clear_decoration_pressed(&mut self) {
        for tracked in &mut self.windows {
            if tracked.decoration_pressed {
                tracked.decoration_pressed = false;
                set_decoration_opacity_target(tracked, decoration_opacity_target(tracked));
            }
        }
    }

    pub fn set_titlebar_close_pressed(&mut self, surface: &WlSurface, pressed: bool) {
        if let Some(tracked) = self.find_mut(surface) {
            tracked.titlebar_close_pressed = pressed && tracked.active;
            set_titlebar_close_tint_target(tracked, titlebar_close_tint_target(tracked));
        }
    }

    pub fn set_titlebar_close_pressed_window(&mut self, window: &Window, pressed: bool) {
        if let Some(tracked) = self.find_window_mut(window) {
            tracked.titlebar_close_pressed = pressed && tracked.active;
            set_titlebar_close_tint_target(tracked, titlebar_close_tint_target(tracked));
        }
    }

    pub fn clear_titlebar_close_pressed(&mut self) {
        for tracked in &mut self.windows {
            if tracked.titlebar_close_pressed {
                tracked.titlebar_close_pressed = false;
                set_titlebar_close_tint_target(tracked, titlebar_close_tint_target(tracked));
            }
        }
    }

    pub fn is_fullscreen(&self, surface: &WlSurface) -> bool {
        self.find(surface)
            .map(|tracked| tracked.fullscreen)
            .unwrap_or(false)
    }

    pub fn is_window_fullscreen(&self, window: &Window) -> bool {
        self.find_window(window)
            .map(|tracked| tracked.fullscreen)
            .unwrap_or(false)
    }

    pub fn restore_rect(&self, surface: &WlSurface) -> Option<Rectangle<i32, Logical>> {
        self.find(surface).and_then(|tracked| tracked.restore_rect)
    }

    pub fn window_restore_rect(&self, window: &Window) -> Option<Rectangle<i32, Logical>> {
        self.find_window(window)
            .and_then(|tracked| tracked.restore_rect)
    }

    pub fn set_window_maximized(
        &mut self,
        window: &Window,
        maximized: bool,
        restore_rect: Option<Rectangle<i32, Logical>>,
    ) {
        if let Some(tracked) = self.find_window_mut(window) {
            tracked.maximized = maximized;
            if maximized {
                tracked.fullscreen = false;
                tracked.snap_side = None;
                tracked.snap_restore_rect = None;
                tracked.initial_positioned = true;
            }
            tracked.restore_rect = maximized.then_some(restore_rect).flatten();
        }
    }

    pub fn set_window_fullscreen(
        &mut self,
        window: &Window,
        fullscreen: bool,
        restore_rect: Option<Rectangle<i32, Logical>>,
    ) {
        if let Some(tracked) = self.find_window_mut(window) {
            tracked.fullscreen = fullscreen;
            if fullscreen {
                tracked.maximized = false;
                tracked.snap_side = None;
                tracked.snap_restore_rect = None;
                tracked.initial_positioned = true;
            }
            tracked.restore_rect = fullscreen.then_some(restore_rect).flatten();
        }
    }

    pub fn snap_side(&self, surface: &WlSurface) -> Option<SnapSide> {
        self.find(surface).and_then(|tracked| tracked.snap_side)
    }

    pub fn app_id(&self, surface: &WlSurface) -> Option<String> {
        self.find(surface)
            .and_then(|tracked| tracked.app_id.clone())
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
            tracked.initial_positioned = true;
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

    pub fn window_minimized_rect(&self, window: &Window) -> Option<Rectangle<i32, Logical>> {
        self.find_window(window)
            .and_then(|tracked| tracked.minimized_rect)
    }

    pub fn is_window_minimized(&self, window: &Window) -> bool {
        self.find_window(window)
            .map(|tracked| tracked.minimized)
            .unwrap_or(false)
    }

    pub fn uses_server_decoration(&self, surface: &WlSurface) -> bool {
        self.find(surface)
            .map(uses_server_decoration)
            .unwrap_or(true)
    }

    pub fn animation(&self, surface: &WlSurface, config: AnimationConfig) -> WindowAnimation {
        self.find(surface)
            .map(|tracked| animation_for_tracked(tracked, config))
            .unwrap_or_default()
    }

    pub fn needs_animation_frame(&self, config: AnimationConfig) -> bool {
        if !config.enabled {
            return false;
        }

        self.windows
            .iter()
            .any(|tracked| tracked_needs_animation_frame(tracked, config))
    }

    pub fn start_map_animation_if_needed(&mut self, surface: &WlSurface) {
        if let Some(tracked) = self.find_mut(surface) {
            if !tracked.map_animation_started {
                tracked.mapped_at = Instant::now();
                tracked.map_animation_started = true;
            }
        }
    }

    pub fn start_geometry_animation(
        &mut self,
        surface: &WlSurface,
        from: Rectangle<i32, Logical>,
        to: Rectangle<i32, Logical>,
    ) {
        if let Some(tracked) = self.find_mut(surface) {
            if from == to || !valid_animation_rect(from) || !valid_animation_rect(to) {
                tracked.geometry_animation = None;
                return;
            }

            tracked.geometry_animation = Some(GeometryAnimation {
                from,
                to,
                started_at: Instant::now(),
            });
        }
    }

    pub fn needs_initial_position(&self, surface: &WlSurface) -> bool {
        self.find(surface)
            .map(|tracked| {
                !tracked.initial_positioned
                    && !tracked.maximized
                    && !tracked.fullscreen
                    && tracked.snap_side.is_none()
                    && !tracked.minimized
            })
            .unwrap_or(false)
    }

    pub fn mark_initial_positioned(&mut self, surface: &WlSurface) {
        if let Some(tracked) = self.find_mut(surface) {
            tracked.initial_positioned = true;
        }
    }

    pub fn frames(
        &self,
        space: &Space<Window>,
        config: AnimationConfig,
        controls_mode: WindowControlsMode,
    ) -> Vec<WindowFrame> {
        let mut frames = Vec::new();

        for window in space.elements() {
            let Some(tracked) = self.find_window(window) else {
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
                legacy_x11: is_x11_tracked(tracked),
                animation: animation_for_tracked(tracked, config),
                decoration_opacity: decoration_opacity_for_tracked(tracked, config),
                close_tint: titlebar_close_tint_for_tracked(tracked),
                controls_mode,
            });
        }

        frames
    }

    pub fn decoration_hit_at(
        &self,
        space: &Space<Window>,
        position: Point<f64, Logical>,
        controls_mode: WindowControlsMode,
    ) -> Option<DecorationHit> {
        for window in space.elements().rev() {
            let Some(tracked) = self.find_window(window) else {
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

            let x11_csd = !uses_server && is_x11_tracked(tracked);
            let (resize_hitbox, top_resize_hitbox) = if uses_server || x11_csd {
                (RESIZE_HITBOX, TOP_RESIZE_HITBOX)
            } else {
                (CSD_RESIZE_HITBOX, CSD_TOP_RESIZE_HITBOX)
            };

            if !uses_server {
                if !tracked.maximized {
                    if let Some(edges) =
                        resize_edge_for(visual_rect, position, resize_hitbox, top_resize_hitbox)
                            .and_then(|edges| allowed_resize_edges(&tracked.window, edges))
                    {
                        if x11_csd || !contains(visual_rect, position) {
                            return Some(DecorationHit {
                                window: tracked.window.clone(),
                                action: DecorationAction::Resize(edges),
                            });
                        }
                    }
                }

                if contains(visual_rect, position) {
                    // Let smart Wayland client-side-decorated apps, such as Firefox,
                    // receive their own border press and issue xdg_toplevel.resize.
                    if !x11_csd {
                        return None;
                    }
                    return None;
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
                legacy_x11: is_x11_tracked(tracked),
                animation: WindowAnimation::default(),
                decoration_opacity: decoration_opacity_target(tracked),
                close_tint: titlebar_close_tint_target(tracked),
                controls_mode,
            };

            if !contains(frame.frame, position) {
                continue;
            }

            if controls_mode == WindowControlsMode::Buttons {
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
            }

            if contains(frame.header, position) {
                return Some(DecorationHit {
                    window: tracked.window.clone(),
                    action: DecorationAction::Titlebar,
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

    fn find_window_mut(&mut self, window: &Window) -> Option<&mut TrackedWindow> {
        self.windows
            .iter_mut()
            .find(|tracked| tracked.window == *window)
    }

    fn find_window(&self, window: &Window) -> Option<&TrackedWindow> {
        self.windows
            .iter()
            .find(|tracked| tracked.window == *window)
    }

    #[cfg(feature = "xwayland")]
    fn find_x11_mut(&mut self, surface: &X11Surface) -> Option<&mut TrackedWindow> {
        self.windows
            .iter_mut()
            .find(|tracked| tracked.x11_window_id == Some(surface.window_id()))
    }

    fn find(&self, surface: &WlSurface) -> Option<&TrackedWindow> {
        self.windows
            .iter()
            .find(|tracked| same_surface(tracked, surface))
    }
}

fn valid_animation_rect(rect: Rectangle<i32, Logical>) -> bool {
    rect.size.w > 1 && rect.size.h > 1
}

fn same_surface(tracked: &TrackedWindow, surface: &WlSurface) -> bool {
    tracked.surface.as_ref() == Some(surface)
}

fn tracked_window_alive(tracked: &TrackedWindow) -> bool {
    if let Some(toplevel) = tracked.window.toplevel() {
        return toplevel.wl_surface().is_alive();
    }

    #[cfg(feature = "xwayland")]
    if let Some(surface) = tracked.window.x11_surface() {
        return surface.alive();
    }

    false
}

#[cfg(feature = "xwayland")]
fn title_for_x11(surface: &X11Surface) -> String {
    let title = surface.title();
    if title.is_empty() {
        "Untitled".to_string()
    } else {
        title
    }
}

#[cfg(feature = "xwayland")]
fn app_id_for_x11(surface: &X11Surface) -> Option<String> {
    let class = surface.class();
    if !class.is_empty() {
        return Some(class);
    }

    let instance = surface.instance();
    (!instance.is_empty()).then_some(instance)
}

fn animation_for_tracked(tracked: &TrackedWindow, config: AnimationConfig) -> WindowAnimation {
    if !config.enabled {
        return WindowAnimation::default();
    }

    if tracked.close_animating {
        return close_animation_for_elapsed(tracked.close_started_at.elapsed(), config);
    }
    if tracked.close_restoring {
        return close_restore_animation_for_elapsed(
            tracked.close_restore_started_at.elapsed(),
            config,
        );
    }
    if tracked.resizing {
        return WindowAnimation::default();
    }

    let mut animation = if tracked.map_animation_started {
        animation_for_elapsed(tracked.mapped_at.elapsed(), config)
    } else {
        WindowAnimation::default()
    };

    animation.geometry = tracked.geometry_animation.and_then(|geometry| {
        if config.geometry_ms == 0 {
            return None;
        }

        let duration = Duration::from_millis(config.geometry_ms);
        let t = geometry.started_at.elapsed().as_secs_f64() / duration.as_secs_f64();
        (t < 1.0).then_some(GeometryAnimationFrame {
            from: geometry.from,
            to: geometry.to,
            progress: ease_out_cubic(t.clamp(0.0, 1.0)),
        })
    });

    animation
}

fn tracked_needs_animation_frame(tracked: &TrackedWindow, config: AnimationConfig) -> bool {
    if tracked.resizing {
        return false;
    }
    if tracked.close_animating {
        return config.close_ms > 0
            && tracked.close_started_at.elapsed()
                < Duration::from_millis(config.close_ms + CLOSE_RESTORE_GRACE_MS);
    }
    if tracked.close_restoring {
        return config.close_ms > 0
            && tracked.close_restore_started_at.elapsed() < Duration::from_millis(config.close_ms);
    }
    if tracked.map_animation_started
        && config.popup_ms > 0
        && tracked.mapped_at.elapsed() < Duration::from_millis(config.popup_ms)
    {
        return true;
    }
    if tracked.geometry_animation.is_some_and(|geometry| {
        config.geometry_ms > 0
            && geometry.started_at.elapsed() < Duration::from_millis(config.geometry_ms)
    }) {
        return true;
    }
    if config.decoration_ms > 0
        && tracked.decoration_opacity_started_at.elapsed()
            < Duration::from_millis(config.decoration_ms)
    {
        return true;
    }

    tracked.titlebar_close_tint_started_at.elapsed() < Duration::from_millis(TITLEBAR_CLOSE_TINT_MS)
}

fn close_animation_for_elapsed(elapsed: Duration, config: AnimationConfig) -> WindowAnimation {
    if config.close_ms == 0 {
        return WindowAnimation::default();
    }

    let duration = Duration::from_millis(config.close_ms);
    let t = elapsed.as_secs_f64() / duration.as_secs_f64();
    let eased = ease_out_cubic(t.clamp(0.0, 1.0));
    WindowAnimation {
        alpha: (1.0 - eased) as f32,
        scale: 1.0 - 0.04 * eased,
        geometry: None,
    }
}

fn close_restore_animation_for_elapsed(
    elapsed: Duration,
    config: AnimationConfig,
) -> WindowAnimation {
    if config.close_ms == 0 {
        return WindowAnimation::default();
    }

    let duration = Duration::from_millis(config.close_ms);
    let t = elapsed.as_secs_f64() / duration.as_secs_f64();
    let eased = ease_out_cubic(t.clamp(0.0, 1.0));
    WindowAnimation {
        alpha: eased as f32,
        scale: 0.96 + 0.04 * eased,
        geometry: None,
    }
}

fn animation_for_elapsed(elapsed: Duration, config: AnimationConfig) -> WindowAnimation {
    if config.popup_ms == 0 {
        return WindowAnimation::default();
    }

    let duration = Duration::from_millis(config.popup_ms);
    let t = (elapsed.as_secs_f64() / duration.as_secs_f64()).clamp(0.0, 1.0);
    let eased = ease_out_cubic(t);

    WindowAnimation {
        alpha: eased as f32,
        scale: MAP_ANIMATION_START_SCALE + (1.0 - MAP_ANIMATION_START_SCALE) * eased,
        geometry: None,
    }
}

fn decoration_opacity_target(tracked: &TrackedWindow) -> f32 {
    if tracked.decoration_pressed {
        DECORATION_PRESSED_OPACITY
    } else if tracked.active {
        DECORATION_ACTIVE_OPACITY
    } else {
        DECORATION_INACTIVE_OPACITY
    }
}

fn set_decoration_opacity_target(tracked: &mut TrackedWindow, target: f32) {
    if (tracked.decoration_opacity_to - target).abs() < f32::EPSILON {
        return;
    }

    tracked.decoration_opacity_from =
        current_decoration_opacity(tracked, Duration::from_millis(140));
    tracked.decoration_opacity_to = target;
    tracked.decoration_opacity_started_at = Instant::now();
}

fn decoration_opacity_for_tracked(tracked: &TrackedWindow, config: AnimationConfig) -> f32 {
    if !config.enabled || config.decoration_ms == 0 {
        return tracked.decoration_opacity_to;
    }

    current_decoration_opacity(tracked, Duration::from_millis(config.decoration_ms))
}

fn titlebar_close_tint_target(tracked: &TrackedWindow) -> f32 {
    if tracked.titlebar_close_pressed {
        TITLEBAR_CLOSE_TINT_TARGET
    } else {
        0.0
    }
}

fn set_titlebar_close_tint_target(tracked: &mut TrackedWindow, target: f32) {
    if (tracked.titlebar_close_tint_to - target).abs() < f32::EPSILON {
        return;
    }

    tracked.titlebar_close_tint_from = titlebar_close_tint_for_tracked(tracked);
    tracked.titlebar_close_tint_to = target;
    tracked.titlebar_close_tint_started_at = Instant::now();
}

fn titlebar_close_tint_for_tracked(tracked: &TrackedWindow) -> f32 {
    let duration = Duration::from_millis(TITLEBAR_CLOSE_TINT_MS);
    let t = tracked
        .titlebar_close_tint_started_at
        .elapsed()
        .as_secs_f64()
        / duration.as_secs_f64();
    let progress = ease_out_cubic(t.clamp(0.0, 1.0)) as f32;
    tracked.titlebar_close_tint_from
        + (tracked.titlebar_close_tint_to - tracked.titlebar_close_tint_from) * progress
}

fn current_decoration_opacity(tracked: &TrackedWindow, duration: Duration) -> f32 {
    if duration.is_zero() {
        return tracked.decoration_opacity_to;
    }

    let t = (tracked
        .decoration_opacity_started_at
        .elapsed()
        .as_secs_f64()
        / duration.as_secs_f64())
    .clamp(0.0, 1.0);
    let eased = ease_out_cubic(t) as f32;
    tracked.decoration_opacity_from
        + (tracked.decoration_opacity_to - tracked.decoration_opacity_from) * eased
}

fn ease_out_cubic(t: f64) -> f64 {
    1.0 - (1.0 - t).powi(3)
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

fn is_x11_tracked(tracked: &TrackedWindow) -> bool {
    #[cfg(feature = "xwayland")]
    {
        tracked.x11_window_id.is_some()
    }
    #[cfg(not(feature = "xwayland"))]
    {
        let _ = tracked;
        false
    }
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
        return Some(edges);
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
    let size = window.geometry().size;
    (size.w > 0 && size.h > 0).then_some(Rectangle::new(location, size))
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
