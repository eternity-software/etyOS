#![cfg_attr(not(feature = "winit-backend"), allow(dead_code, unused_imports))]

use std::{
    collections::{HashMap, HashSet},
    fs,
    path::Path,
};

use image::{imageops::FilterType, DynamicImage, Rgba, RgbaImage};
use rusttype::{point, Font, Scale};
use smithay::{
    backend::renderer::element::AsRenderElements,
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{
                memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
                surface::{
                    render_elements_from_surface_tree, WaylandSurfaceRenderElement,
                    WaylandSurfaceTexture,
                },
                texture::{TextureBuffer, TextureRenderElement},
                Element, Id, Kind, RenderElement,
            },
            gles::{
                element::TextureShaderElement, ffi, GlesError, GlesRenderer, GlesTarget,
                GlesTexProgram, GlesTexture, Uniform, UniformName, UniformType,
            },
            utils::{CommitCounter, DamageSet, OpaqueRegions},
            ExportMem, Frame, Renderer, Texture,
        },
    },
    desktop::{
        space::SpaceRenderElements, utils::send_frames_surface_tree, PopupManager, Space, Window,
    },
    output::{Mode, Output, PhysicalProperties, Scale as OutputScale, Subpixel},
    reexports::wayland_server::{protocol::wl_surface::WlSurface, DisplayHandle},
    utils::{Buffer, Logical, Physical, Point, Rectangle, Scale as RendererScale, Size, Transform},
};
use tracing::{info, warn};

use crate::screencopy::CapturedFrame;
#[cfg(feature = "tty-udev")]
use crate::window::WindowStore;
use crate::{
    config::WindowControlsMode,
    state::Yawc,
    window::{WindowAnimation, WindowFrame, BUTTON_PADDING, FRAME_RADIUS, TITLEBAR_HEIGHT},
};
use smithay::backend::renderer::{Bind, Offscreen};

smithay::backend::renderer::element::render_elements! {
    pub YawcRenderElements<=GlesRenderer>;
    Space=SpaceRenderElements<GlesRenderer, WaylandSurfaceRenderElement<GlesRenderer>>,
    Memory=MemoryRenderBufferRenderElement<GlesRenderer>,
    Texture=TextureRenderElement<GlesTexture>,
    TextureShader=TextureShaderElement,
    Shape=ShapeElement,
    TitlebarBlur=TitlebarBlurElement,
    RoundedSurface=RoundedSurfaceElement,
    CsdRoundedSurface=CsdRoundedSurfaceElement,
    AnimatedSurface=AnimatedSurfaceElement,
}

#[derive(Clone)]
#[cfg(feature = "tty-udev")]
pub struct CaptureCursor {
    pub buffer: MemoryRenderBuffer,
    pub hotspot: Point<i32, Physical>,
    pub location: Point<f64, Logical>,
}

const FRAME_FILL_RGBA: [u8; 4] = [92, 96, 102, 150];
const TITLE_COLOR: Rgba<u8> = Rgba([244, 246, 248, 238]);
const CLOSE_COLOR: Rgba<u8> = Rgba([244, 246, 248, 230]);
const TITLE_PADDING: i32 = 18;
const TITLE_FONT_SIZE: f32 = 17.5;
const ICON_SIZE: u32 = 20;
const ICON_GAP: i32 = 10;
const LEGACY_BADGE_SIZE: i32 = 18;
const LEGACY_BADGE_GAP: i32 = 10;
const LEGACY_BADGE_COLOR: Rgba<u8> = Rgba([245, 145, 35, 235]);
const LEGACY_BADGE_MARK_COLOR: Rgba<u8> = Rgba([52, 32, 16, 245]);
const LEGACY_TOOLTIP_TEXT: &str = "Legacy X11 application running through XWayland";
const TOOLTIP_FONT_SIZE: f32 = 14.0;
const TOOLTIP_PADDING_X: i32 = 10;
const TOOLTIP_HEIGHT: i32 = 28;
const BLUR_PAD_X: i32 = 48;
const BLUR_PAD_Y: i32 = 96;
const CSD_RADIUS: i32 = 10;
const SHADOW_PAD: i32 = 28;
const SHADOW_OFFSET_Y: i32 = 8;
const SHADOW_OPACITY: f32 = 0.16;
const SHADOW_SPREAD: f32 = 18.0;
const TITLEBAR_SHAPE_MODE: f32 = 0.0;
const SHADOW_SHAPE_MODE: f32 = 1.0;
const TITLEBAR_BLUR_SHADER: &str = r#"
//_DEFINES_

#if defined(EXTERNAL)
#extension GL_OES_EGL_image_external : require
#endif

precision highp float;
#if defined(EXTERNAL)
uniform samplerExternalOES tex;
#else
uniform sampler2D tex;
#endif

uniform float alpha;
uniform vec2 area_size;
uniform vec2 texel_size;
uniform float radius;
uniform vec2 src_origin;
uniform vec2 src_size;
uniform float flip_y;
varying vec2 v_coords;

float top_round_alpha(vec2 p, vec2 size, float r) {
    if (p.x < 0.0 || p.y < 0.0 || p.x >= size.x || p.y >= size.y) {
        return 0.0;
    }
    r = min(r, min(size.x, size.y) * 0.5);
    if (r <= 0.0) {
        return 1.0;
    }
    if (p.y >= r || (p.x >= r && p.x < size.x - r)) {
        return 1.0;
    }

    vec2 center = vec2(p.x < r ? r : size.x - r, r);
    float dist = length(p - center) - r;
    return 1.0 - smoothstep(-1.25, 1.25, dist);
}

void main() {
    vec2 local_coords = (v_coords - src_origin) / src_size;
    vec2 pos = local_coords * area_size;
    float corner_alpha = top_round_alpha(pos, area_size, radius);
    if (corner_alpha <= 0.0) {
        gl_FragColor = vec4(0.0);
        return;
    }

    vec2 sample_coords = vec2(v_coords.x, mix(v_coords.y, 1.0 - v_coords.y, flip_y));
    vec4 color = vec4(0.0);
    float total = 0.0;
    const float sigma = 3.5;
    const float blur_step = 1.75;

    for (int ix = -6; ix <= 6; ++ix) {
        for (int iy = -6; iy <= 6; ++iy) {
            vec2 offset = vec2(float(ix), float(iy)) * texel_size * blur_step;
            float dist2 = float(ix * ix + iy * iy);
            float weight = exp(-dist2 / (2.0 * sigma * sigma));
            color += texture2D(tex, sample_coords + offset) * weight;
            total += weight;
        }
    }

    color /= total;
    color.rgb = mix(color.rgb, vec3(0.42, 0.44, 0.47), 0.045);

#if defined(NO_ALPHA)
    color = vec4(color.rgb, 1.0) * alpha;
#else
    color = color * alpha;
#endif

    gl_FragColor = color * corner_alpha;
}"#;
const CLIENT_CLIP_SHADER: &str = r#"
//_DEFINES_

#if defined(EXTERNAL)
#extension GL_OES_EGL_image_external : require
#endif

precision highp float;
#if defined(EXTERNAL)
uniform samplerExternalOES tex;
#else
uniform sampler2D tex;
#endif

uniform float alpha;
uniform vec2 client_size;
uniform vec2 element_offset;
uniform vec2 element_size;
uniform float radius;
varying vec2 v_coords;

float bottom_round_alpha(vec2 p, vec2 size, float r) {
    if (p.x < 0.0 || p.y < 0.0 || p.x >= size.x || p.y >= size.y) {
        return 0.0;
    }
    r = min(r, min(size.x, size.y) * 0.5);
    if (r <= 0.0) {
        return 1.0;
    }
    if (p.y < size.y - r || (p.x >= r && p.x < size.x - r)) {
        return 1.0;
    }

    vec2 center = vec2(p.x < r ? r : size.x - r, size.y - r);
    float dist = length(p - center) - r;
    return 1.0 - smoothstep(-1.25, 1.25, dist);
}

void main() {
    vec2 pos = element_offset + v_coords * element_size;
    float corner_alpha = bottom_round_alpha(pos, client_size, radius);
    if (corner_alpha <= 0.0) {
        gl_FragColor = vec4(0.0);
        return;
    }

    vec4 color = texture2D(tex, v_coords);

#if defined(NO_ALPHA)
    color = vec4(color.rgb, 1.0) * alpha;
#else
    color = color * alpha;
#endif

    gl_FragColor = color * corner_alpha;
}"#;
const CSD_CLIP_SHADER: &str = r#"
//_DEFINES_

#if defined(EXTERNAL)
#extension GL_OES_EGL_image_external : require
#endif

precision highp float;
#if defined(EXTERNAL)
uniform samplerExternalOES tex;
#else
uniform sampler2D tex;
#endif

uniform float alpha;
uniform vec2 client_size;
uniform vec2 element_offset;
uniform vec2 element_size;
uniform float radius;
varying vec2 v_coords;

float rounded_distance(vec2 p, vec2 origin, vec2 size, float r) {
    r = min(r, min(size.x, size.y) * 0.5);
    vec2 center = origin + size * 0.5;
    vec2 half_size = size * 0.5 - vec2(r);
    vec2 q = abs(p - center) - half_size;
    return length(max(q, vec2(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

float round_alpha(vec2 p, vec2 size, float r) {
    if (p.x < 0.0 || p.y < 0.0 || p.x >= size.x || p.y >= size.y) {
        return 0.0;
    }
    r = min(r, min(size.x, size.y) * 0.5);
    if (r <= 0.0) {
        return 1.0;
    }

    float dist = rounded_distance(p, vec2(0.0), size, r);
    return 1.0 - smoothstep(-1.25, 1.25, dist);
}

void main() {
    vec2 pos = element_offset + v_coords * element_size;
    float corner_alpha = round_alpha(pos, client_size, radius);
    if (corner_alpha <= 0.0) {
        gl_FragColor = vec4(0.0);
        return;
    }

    vec4 color = texture2D(tex, v_coords);

#if defined(NO_ALPHA)
    color = vec4(color.rgb, 1.0) * alpha;
#else
    color = color * alpha;
#endif

    gl_FragColor = color * corner_alpha;
}"#;
const SHAPE_SHADER: &str = r#"
//_DEFINES_

#if defined(EXTERNAL)
#extension GL_OES_EGL_image_external : require
#endif

precision highp float;
#if defined(EXTERNAL)
uniform samplerExternalOES tex;
#else
uniform sampler2D tex;
#endif

uniform float alpha;
uniform vec2 area_size;
uniform vec4 color;
uniform vec4 inner_rect;
uniform float radius;
uniform float mode;
uniform float spread;
varying vec2 v_coords;

float rounded_distance(vec2 p, vec2 origin, vec2 size, float r) {
    r = min(r, min(size.x, size.y) * 0.5);
    vec2 center = origin + size * 0.5;
    vec2 half_size = size * 0.5 - vec2(r);
    vec2 q = abs(p - center) - half_size;
    return length(max(q, vec2(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

float top_round_alpha(vec2 p, vec2 size, float r) {
    if (p.x < 0.0 || p.y < 0.0 || p.x >= size.x || p.y >= size.y) {
        return 0.0;
    }
    r = min(r, min(size.x, size.y) * 0.5);
    if (r <= 0.0) {
        return 1.0;
    }
    if (p.y >= r || (p.x >= r && p.x < size.x - r)) {
        return 1.0;
    }

    vec2 center = vec2(p.x < r ? r : size.x - r, r);
    float dist = length(p - center) - r;
    return 1.0 - smoothstep(-1.25, 1.25, dist);
}

void main() {
    vec2 p = v_coords * area_size;
    float coverage = 1.0;
    if (mode < 0.5) {
        coverage = top_round_alpha(p, area_size, radius);
    } else {
        vec2 origin = inner_rect.xy;
        vec2 size = inner_rect.zw;
        float dist = rounded_distance(p, origin, size, radius);
        if (dist < 0.0) {
            coverage = 0.0;
        } else {
            float glow = clamp(1.0 - dist / max(spread, 1.0), 0.0, 1.0);
            coverage = glow * glow;
        }
    }

    if (coverage <= 0.0) {
        gl_FragColor = vec4(0.0);
        return;
    }

    gl_FragColor = color * alpha * coverage;
}"#;

pub struct RenderState {
    output: Output,
    #[cfg_attr(not(feature = "tty-udev"), allow(dead_code))]
    output_location: Point<i32, Logical>,
    #[cfg_attr(not(feature = "tty-udev"), allow(dead_code))]
    output_scale: f64,
    titlebar_shader: Option<GlesTexProgram>,
    titlebar_shader_failed: bool,
    client_clip_shader: Option<GlesTexProgram>,
    client_clip_shader_failed: bool,
    csd_clip_shader: Option<GlesTexProgram>,
    csd_clip_shader_failed: bool,
    shape_shader: Option<GlesTexProgram>,
    shape_shader_failed: bool,
    shape_texture: Option<GlesTexture>,
    title_font: Option<Font<'static>>,
    icon_cache: HashMap<String, Option<RgbaImage>>,
    overlay_cache: HashMap<DecorationCacheKey, MemoryRenderBuffer>,
    blur_texture_cache: HashMap<WlSurface, GlesTexture>,
    csd_snapshot_cache: HashMap<WlSurface, CsdWindowSnapshot>,
    wallpaper_source: Option<RgbaImage>,
    wallpaper_image: Option<RgbaImage>,
    wallpaper_buffer: Option<MemoryRenderBuffer>,
}

struct CsdWindowSnapshot {
    texture: GlesTexture,
    rect: Rectangle<i32, Logical>,
    output_location: Point<i32, Logical>,
}

impl RenderState {
    pub fn new(
        display_handle: &DisplayHandle,
        space: &mut Space<Window>,
        size: Size<i32, Physical>,
    ) -> Self {
        Self::new_with_output(
            display_handle,
            space,
            size,
            60_000,
            Transform::Flipped180,
            "Nested Compositor",
            "yawc".to_string(),
            (0, 0).into(),
            1.0,
        )
    }

    #[cfg(feature = "tty-udev")]
    pub fn new_standalone(
        display_handle: &DisplayHandle,
        space: &mut Space<Window>,
        size: Size<i32, Physical>,
        refresh: i32,
    ) -> Self {
        Self::new_with_output(
            display_handle,
            space,
            size,
            refresh,
            Transform::Normal,
            "Standalone Session",
            "yawc".to_string(),
            (0, 0).into(),
            1.0,
        )
    }

    #[cfg(feature = "tty-udev")]
    pub fn new_standalone_at(
        display_handle: &DisplayHandle,
        space: &mut Space<Window>,
        size: Size<i32, Physical>,
        refresh: i32,
        name: String,
        location: Point<i32, Logical>,
        scale: f64,
    ) -> Self {
        Self::new_with_output(
            display_handle,
            space,
            size,
            refresh,
            Transform::Normal,
            "Standalone Session",
            name,
            location,
            scale,
        )
    }

    fn new_with_output(
        display_handle: &DisplayHandle,
        space: &mut Space<Window>,
        size: Size<i32, Physical>,
        refresh: i32,
        transform: Transform,
        model: &str,
        name: String,
        location: Point<i32, Logical>,
        scale: f64,
    ) -> Self {
        let mode = Mode {
            size,
            refresh: refresh.max(1),
        };

        let output = Output::new(
            name,
            PhysicalProperties {
                size: (0, 0).into(),
                subpixel: Subpixel::Unknown,
                make: "YAWC".into(),
                model: model.into(),
            },
        );
        let _ = output.create_global::<Yawc>(display_handle);

        output.change_current_state(
            Some(mode),
            Some(transform),
            Some(output_scale(scale)),
            Some(location),
        );
        output.set_preferred(mode);
        space.map_output(&output, location);

        let title_font = load_title_font();
        let wallpaper_source = load_png(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("desktop.png")
                .as_path(),
        );

        let mut state = Self {
            output,
            output_location: location,
            output_scale: scale,
            titlebar_shader: None,
            titlebar_shader_failed: false,
            client_clip_shader: None,
            client_clip_shader_failed: false,
            csd_clip_shader: None,
            csd_clip_shader_failed: false,
            shape_shader: None,
            shape_shader_failed: false,
            shape_texture: None,
            title_font,
            icon_cache: HashMap::new(),
            overlay_cache: HashMap::new(),
            blur_texture_cache: HashMap::new(),
            csd_snapshot_cache: HashMap::new(),
            wallpaper_source,
            wallpaper_image: None,
            wallpaper_buffer: None,
        };
        state.rebuild_desktop_buffers(size);
        state
    }

    #[cfg(feature = "tty-udev")]
    pub fn output(&self) -> &Output {
        &self.output
    }

    #[cfg(feature = "tty-udev")]
    pub fn output_geometry(&self) -> Rectangle<i32, Logical> {
        let size = self
            .output
            .current_mode()
            .map(|mode| {
                mode.size
                    .to_f64()
                    .to_logical(self.output.current_scale().fractional_scale())
                    .to_i32_ceil()
            })
            .unwrap_or_else(|| Size::from((1, 1)));
        Rectangle::new(self.output_location, size)
    }

    #[cfg(feature = "tty-udev")]
    pub fn reconfigure_output(
        &mut self,
        size: Size<i32, Physical>,
        refresh: i32,
        location: Point<i32, Logical>,
        scale: f64,
    ) {
        let mode = Mode {
            size,
            refresh: refresh.max(1),
        };

        self.output.change_current_state(
            Some(mode),
            Some(Transform::Normal),
            Some(output_scale(scale)),
            Some(location),
        );
        self.output.set_preferred(mode);
        if self.output_location != location || (self.output_scale - scale).abs() > f64::EPSILON {
            self.overlay_cache.clear();
            self.blur_texture_cache.clear();
            self.csd_snapshot_cache.clear();
        }
        self.output_location = location;
        self.output_scale = scale;
        self.rebuild_desktop_buffers(size);
    }

    #[cfg(feature = "tty-udev")]
    pub fn wallpaper_buffer(&self) -> Option<&MemoryRenderBuffer> {
        self.wallpaper_buffer.as_ref()
    }

    #[cfg(feature = "tty-udev")]
    pub fn capture_scene_xrgb8888(
        &mut self,
        renderer: &mut GlesRenderer,
        space: &Space<Window>,
        frames: &[WindowFrame],
        windows: &WindowStore,
        animation_config: crate::config::AnimationConfig,
        region: Rectangle<i32, Logical>,
        cursor: Option<CaptureCursor>,
    ) -> Result<CapturedFrame, GlesError> {
        let output_size = self
            .output
            .current_mode()
            .map(|mode| mode.size)
            .unwrap_or_else(|| Size::from((1, 1)));
        let buffer_size = Size::<i32, Buffer>::from((output_size.w, output_size.h));
        let mut target: GlesTexture = renderer.create_buffer(Fourcc::Xrgb8888, buffer_size)?;
        let mut framebuffer = renderer.bind(&mut target)?;
        let mut scene =
            self.tty_scene_elements(renderer, space, frames, windows, animation_config, None)?;
        if let Some(cursor) = cursor {
            if let Some(cursor) = capture_cursor_element(renderer, cursor)? {
                scene.splice(0..0, [cursor]);
            }
        }

        {
            let mut frame = renderer.render(&mut framebuffer, output_size, Transform::Normal)?;
            frame.clear(
                [0.06, 0.09, 0.11, 1.0].into(),
                &[Rectangle::from_size(output_size)],
            )?;
            draw_elements_back_to_front_with_offset(
                &mut frame,
                &scene,
                self.output_location.to_physical(1),
            )?;
            let _ = frame.finish()?;
        }

        let local_region = Rectangle::new(
            (
                region.loc.x - self.output_location.x,
                region.loc.y - self.output_location.y,
            )
                .into(),
            region.size,
        );
        read_xrgb_framebuffer(renderer, &framebuffer, output_size, local_region)
    }

    #[cfg(feature = "tty-udev")]
    pub fn capture_scene_into_dmabuf(
        &mut self,
        renderer: &mut GlesRenderer,
        space: &Space<Window>,
        frames: &[WindowFrame],
        windows: &WindowStore,
        animation_config: crate::config::AnimationConfig,
        region: Rectangle<i32, Logical>,
        target: &mut smithay::backend::allocator::dmabuf::Dmabuf,
        cursor: Option<CaptureCursor>,
    ) -> Result<(), GlesError> {
        use smithay::backend::allocator::Buffer as _;

        let target_size = target.size();
        let render_size = Size::<i32, Physical>::from((target_size.w, target_size.h));
        let expected_size = Size::<i32, Buffer>::from((region.size.w.max(1), region.size.h.max(1)));
        if target_size != expected_size {
            return Err(GlesError::FramebufferBindingError);
        }

        let mut scene =
            self.tty_scene_elements(renderer, space, frames, windows, animation_config, None)?;
        if let Some(cursor) = cursor {
            if let Some(cursor) = capture_cursor_element(renderer, cursor)? {
                scene.splice(0..0, [cursor]);
            }
        }
        let mut framebuffer = renderer.bind(target)?;

        {
            let mut frame = renderer.render(&mut framebuffer, render_size, Transform::Normal)?;
            frame.clear(
                [0.06, 0.09, 0.11, 1.0].into(),
                &[Rectangle::from_size(render_size)],
            )?;
            draw_elements_back_to_front_with_offset(
                &mut frame,
                &scene,
                Point::<i32, Physical>::from((region.loc.x, region.loc.y)),
            )?;
            let _ = frame.finish()?;
        }

        Ok(())
    }

    #[cfg(feature = "tty-udev")]
    pub fn tty_scene_elements(
        &mut self,
        renderer: &mut GlesRenderer,
        space: &Space<Window>,
        frames: &[WindowFrame],
        windows: &WindowStore,
        animation_config: crate::config::AnimationConfig,
        pointer_location: Option<Point<f64, Logical>>,
    ) -> Result<Vec<YawcRenderElements>, GlesError> {
        let mut frame_by_window = HashMap::new();
        for frame in frames {
            frame_by_window.insert(frame.window.clone(), frame.clone());
        }

        let output_size = self
            .output
            .current_mode()
            .map(|mode| mode.size)
            .unwrap_or_else(|| Size::from((1, 1)));
        let output_geometry = self.output_geometry();
        let titlebar_shader = self.ensure_titlebar_shader(renderer)?.cloned();
        let mut deco_by_window = self.decoration_elements(renderer, frames)?;
        let mut live_csd_surfaces = HashSet::new();
        let mut elements = Vec::new();

        for window in space.elements().rev() {
            let mut window_elements = Vec::new();
            let mut blur_element = None;
            let mut shadow_element = None;
            let mut csd_snapshot = None;
            let frame_meta = frame_by_window.get(window);
            let animation = frame_meta
                .map(|frame| frame.animation)
                .or_else(|| {
                    window
                        .toplevel()
                        .map(|toplevel| windows.animation(toplevel.wl_surface(), animation_config))
                })
                .unwrap_or_default();

            if let Some(frame_meta) = frame_meta {
                if let Some(deco) = deco_by_window.remove(window) {
                    window_elements.extend(deco);
                    if let Some(fill) = self.titlebar_fill_element(renderer, frame_meta)? {
                        window_elements.push(fill);
                    }
                }

                if let (Some(shader), Some(surface), Some(capture)) = (
                    titlebar_shader.as_ref(),
                    window_wl_surface(window),
                    blur_capture_for_frame(frame_meta, output_geometry, BlurOrigin::TopLeft),
                ) {
                    let blur_texture = self.ensure_blur_texture(
                        renderer,
                        &surface,
                        capture.capture_w,
                        capture.capture_h,
                    )?;
                    blur_element = Some(YawcRenderElements::from(TitlebarBlurElement::new(
                        blur_texture,
                        shader.clone(),
                        capture.dst_loc,
                        capture.dst_size(),
                        capture,
                        Some(frame_meta.frame),
                        decoration_animation_for_frame(frame_meta),
                    )));
                }
            }

            if let Some(loc) = space.element_location(window) {
                let render_loc = Point::<i32, Logical>::from((
                    loc.x - window.geometry().loc.x,
                    loc.y - window.geometry().loc.y,
                ));
                let anchor = animation_anchor(space, window, frame_meta);
                if let Some(frame_meta) = frame_meta {
                    if !frame_meta.maximized && !frame_meta.fullscreen && !frame_meta.resizing {
                        shadow_element = self.shadow_element(
                            renderer,
                            frame_meta.frame,
                            FRAME_RADIUS,
                            Some(frame_meta.frame),
                            frame_meta.animation,
                        )?;
                    }
                    let phys_loc = Point::<i32, Physical>::from((render_loc.x, render_loc.y));
                    let popup_elements = window_popup_elements(renderer, window, phys_loc)
                        .into_iter()
                        .map(YawcRenderElements::from);
                    window_elements.splice(0..0, popup_elements);
                    let surf =
                        window.render_elements(renderer, phys_loc, RendererScale::from(1.0), 1.0);
                    window_elements
                        .extend(self.client_surface_elements(renderer, frame_meta, surf)?);
                } else {
                    let phys_loc = Point::<i32, Physical>::from((render_loc.x, render_loc.y));
                    let surf: Vec<
                        SpaceRenderElements<
                            GlesRenderer,
                            WaylandSurfaceRenderElement<GlesRenderer>,
                        >,
                    > = window.render_elements(renderer, phys_loc, RendererScale::from(1.0), 1.0);
                    let has_csd_surface = !surf.is_empty();
                    let csd_rect = csd_visual_rect(window, render_loc);
                    let tiled = window
                        .toplevel()
                        .map(|toplevel| {
                            windows.is_fullscreen(toplevel.wl_surface())
                                || windows.is_maximized(toplevel.wl_surface())
                        })
                        .unwrap_or(false);
                    let resizing = window
                        .toplevel()
                        .map(|toplevel| windows.is_resizing(toplevel.wl_surface()))
                        .unwrap_or(false);
                    if !tiled && !resizing {
                        shadow_element =
                            self.shadow_element(renderer, csd_rect, CSD_RADIUS, anchor, animation)?;
                    }
                    window_elements.extend(
                        self.csd_surface_elements(renderer, csd_rect, surf, anchor, animation)?,
                    );
                    if has_csd_surface {
                        if let Some(toplevel) = window.toplevel() {
                            csd_snapshot = Some((toplevel.wl_surface().clone(), csd_rect));
                        }
                    } else if let Some(toplevel) = window.toplevel() {
                        live_csd_surfaces.insert(toplevel.wl_surface().clone());
                    }
                }
            }

            // DrmOutput draws elements back-to-front from this front-to-back list.
            // Put blur after the client element in the list so it captures before
            // this window's own client is drawn, while the overlay remains above it.
            if let Some(blur_element) = blur_element {
                window_elements.push(blur_element);
            }
            if let Some(shadow_element) = shadow_element {
                window_elements.push(shadow_element);
            }
            if let Some((surface, rect)) = csd_snapshot {
                live_csd_surfaces.insert(surface.clone());
                self.update_csd_snapshot(
                    renderer,
                    &surface,
                    output_size,
                    self.output_location,
                    rect,
                    &window_elements,
                )?;
            }

            elements.extend(window_elements);
        }

        let destroyed = windows.destroyed_close_animations(animation_config);
        let snapshot_elements = self.csd_snapshot_elements(renderer, &destroyed);
        elements.splice(0..0, snapshot_elements);
        self.retain_csd_snapshots(&live_csd_surfaces, &destroyed);

        if let Some(tooltip) =
            self.legacy_tooltip_element(renderer, frames, pointer_location, output_geometry)?
        {
            elements.splice(0..0, [tooltip]);
        }

        elements.extend(desktop_elements(
            renderer,
            self.wallpaper_buffer.as_ref(),
            self.output_location.to_f64(),
        )?);

        Ok(elements)
    }

    pub fn resize(&mut self, size: Size<i32, Physical>) {
        let mode = Mode {
            size,
            refresh: 60_000,
        };

        self.output
            .change_current_state(Some(mode), Some(Transform::Flipped180), None, None);
        self.output.set_preferred(mode);
        self.overlay_cache.clear();
        self.blur_texture_cache.clear();
        self.csd_snapshot_cache.clear();
        self.rebuild_desktop_buffers(size);
    }

    fn update_csd_snapshot(
        &mut self,
        renderer: &mut GlesRenderer,
        surface: &WlSurface,
        output_size: Size<i32, Physical>,
        output_location: Point<i32, Logical>,
        rect: Rectangle<i32, Logical>,
        elements: &[YawcRenderElements],
    ) -> Result<(), GlesError> {
        if rect.size.w <= 0 || rect.size.h <= 0 {
            return Ok(());
        }

        let mut texture = self.csd_snapshot_texture(renderer, surface, output_size)?;
        {
            let mut framebuffer = renderer.bind(&mut texture)?;
            {
                let mut frame =
                    renderer.render(&mut framebuffer, output_size, Transform::Normal)?;
                frame.clear(
                    [0.0, 0.0, 0.0, 0.0].into(),
                    &[Rectangle::from_size(output_size)],
                )?;
                draw_elements_back_to_front_with_offset(
                    &mut frame,
                    elements,
                    output_location.to_physical(1),
                )?;
                let _ = frame.finish()?;
            }
        }

        match self.csd_snapshot_cache.get_mut(surface) {
            Some(snapshot) => {
                snapshot.texture = texture;
                snapshot.rect = rect;
                snapshot.output_location = output_location;
            }
            None => {
                self.csd_snapshot_cache.insert(
                    surface.clone(),
                    CsdWindowSnapshot {
                        texture,
                        rect,
                        output_location,
                    },
                );
            }
        }
        Ok(())
    }

    fn csd_snapshot_texture(
        &mut self,
        renderer: &mut GlesRenderer,
        surface: &WlSurface,
        output_size: Size<i32, Physical>,
    ) -> Result<GlesTexture, GlesError> {
        let buffer_size = Size::<i32, Buffer>::from((output_size.w.max(1), output_size.h.max(1)));
        if let Some(snapshot) = self.csd_snapshot_cache.get(surface) {
            if snapshot.texture.size() == buffer_size {
                return Ok(snapshot.texture.clone());
            }
        }

        renderer.create_buffer(Fourcc::Argb8888, buffer_size)
    }

    fn csd_snapshot_elements(
        &mut self,
        renderer: &mut GlesRenderer,
        destroyed: &[(WlSurface, WindowAnimation)],
    ) -> Vec<YawcRenderElements> {
        destroyed
            .iter()
            .filter_map(|(surface, animation)| {
                let Some(snapshot) = self.csd_snapshot_cache.get_mut(surface) else {
                    return None;
                };
                if snapshot.rect.size.w <= 0 || snapshot.rect.size.h <= 0 {
                    return None;
                }
                let buffer = TextureBuffer::from_texture(
                    renderer,
                    snapshot.texture.clone(),
                    1,
                    Transform::Normal,
                    None,
                );
                let (loc, size) = animated_rect(snapshot.rect, Some(snapshot.rect), *animation);
                let src = Rectangle::<f64, Logical>::new(
                    (
                        (snapshot.rect.loc.x - snapshot.output_location.x) as f64,
                        (snapshot.rect.loc.y - snapshot.output_location.y) as f64,
                    )
                        .into(),
                    (snapshot.rect.size.w as f64, snapshot.rect.size.h as f64).into(),
                );

                Some(YawcRenderElements::from(
                    TextureRenderElement::from_texture_buffer(
                        loc,
                        &buffer,
                        Some(animation.alpha),
                        Some(src),
                        Some(size),
                        Kind::Unspecified,
                    ),
                ))
            })
            .collect()
    }

    fn retain_csd_snapshots(
        &mut self,
        live_surfaces: &HashSet<WlSurface>,
        destroyed: &[(WlSurface, WindowAnimation)],
    ) {
        let destroyed_surfaces = destroyed
            .iter()
            .map(|(surface, _)| surface.clone())
            .collect::<HashSet<_>>();
        self.csd_snapshot_cache.retain(|surface, _| {
            live_surfaces.contains(surface) || destroyed_surfaces.contains(surface)
        });
    }

    #[cfg(feature = "winit-backend")]
    pub fn render_frame(
        &mut self,
        backend: &mut smithay::backend::winit::WinitGraphicsBackend<GlesRenderer>,
        state: &mut Yawc,
        display_handle: &mut DisplayHandle,
    ) -> Result<(), Box<dyn std::error::Error>> {
        state.reload_config_if_changed();
        state.finish_close_animations();
        let animation_config = state.config.animations();
        let controls_mode = state.config.window_controls();
        let pointer_location = state
            .seat
            .get_pointer()
            .map(|pointer| pointer.current_location());
        let size = backend.window_size();
        let damage = Rectangle::from_size(size);
        let all_frames = state
            .windows
            .frames(&state.space, animation_config, controls_mode);
        let mut frame_by_window = HashMap::new();
        for frame in &all_frames {
            frame_by_window.insert(frame.window.clone(), frame.clone());
        }

        let mut deco_by_window = {
            let renderer = backend.renderer();
            self.decoration_elements(renderer, &all_frames)?
        };
        let mut used_blur_surfaces = HashSet::new();
        let mut live_csd_surfaces = HashSet::new();

        {
            let (renderer, mut framebuffer) = backend.bind()?;
            let windows: Vec<Window> = state.space.elements().cloned().collect();
            let titlebar_shader = self.ensure_titlebar_shader(renderer)?.cloned();
            let desktop = desktop_elements(
                renderer,
                self.wallpaper_buffer.as_ref(),
                Point::<f64, Logical>::from((0.0, 0.0)),
            )?;
            {
                let mut frame = renderer.render(&mut framebuffer, size, Transform::Flipped180)?;
                frame.clear([0.06, 0.09, 0.11, 1.0].into(), &[damage])?;
                draw_elements(&mut frame, &desktop)?;
                let _ = frame.finish()?;
            }

            for window in &windows {
                let mut step_elements: Vec<YawcRenderElements> = Vec::new();
                let mut csd_snapshot = None;

                if let Some(frame_meta) = frame_by_window.get(window) {
                    if let Some(surface) = window_wl_surface(window) {
                        let Some(capture) = blur_capture_for_frame(
                            frame_meta,
                            Rectangle::from_size(size.to_logical(1)),
                            BlurOrigin::BottomLeft,
                        ) else {
                            continue;
                        };
                        used_blur_surfaces.insert(surface.clone());
                        let blur_texture = self.ensure_blur_texture(
                            renderer,
                            &surface,
                            capture.capture_w,
                            capture.capture_h,
                        )?;
                        let blur_texture_size = blur_texture.size();
                        let blur_buffer = TextureBuffer::from_texture(
                            renderer,
                            blur_texture.clone(),
                            1,
                            Transform::Normal,
                            None,
                        );
                        let blur_payload = titlebar_shader.as_ref().map(|shader| {
                            let capture_rect = Rectangle::new(
                                capture.dst_loc,
                                (capture.dst_w, capture.dst_h).into(),
                            );
                            let (blur_loc, blur_size) = animated_rect(
                                capture_rect,
                                Some(frame_meta.frame),
                                decoration_animation_for_frame(frame_meta),
                            );
                            (
                                blur_texture,
                                YawcRenderElements::from(TextureShaderElement::new(
                                    TextureRenderElement::from_texture_buffer(
                                        blur_loc,
                                        &blur_buffer,
                                        Some(decoration_animation_for_frame(frame_meta).alpha),
                                        Some(Rectangle::new(
                                            (capture.src_x as f64, capture.src_y as f64).into(),
                                            (capture.dst_w as f64, capture.dst_h as f64).into(),
                                        )),
                                        Some(blur_size),
                                        Kind::Unspecified,
                                    ),
                                    shader.clone(),
                                    titlebar_shader_uniforms(
                                        blur_size.w,
                                        blur_size.h,
                                        blur_texture_size.w,
                                        blur_texture_size.h,
                                        capture.src_x,
                                        capture.src_y,
                                        capture.flip_y,
                                    ),
                                )),
                                frame_meta.clone(),
                                capture,
                            )
                        });

                        if let Some(deco) = deco_by_window.remove(window) {
                            if !frame_meta.maximized
                                && !frame_meta.fullscreen
                                && !frame_meta.resizing
                            {
                                if let Some(shadow) = self.shadow_element(
                                    renderer,
                                    frame_meta.frame,
                                    FRAME_RADIUS,
                                    Some(frame_meta.frame),
                                    frame_meta.animation,
                                )? {
                                    step_elements.push(shadow);
                                }
                            }
                            if let Some(fill) = self.titlebar_fill_element(renderer, frame_meta)? {
                                step_elements.push(fill);
                            }
                            step_elements.extend(deco);
                        }

                        if let Some(loc) = state.space.element_location(window) {
                            let render_loc = Point::<i32, Logical>::from((
                                loc.x - window.geometry().loc.x,
                                loc.y - window.geometry().loc.y,
                            ));
                            let phys_loc =
                                Point::<i32, Physical>::from((render_loc.x, render_loc.y));
                            let surf = window.render_elements(
                                renderer,
                                phys_loc,
                                RendererScale::from(1.0),
                                1.0,
                            );
                            step_elements
                                .extend(self.client_surface_elements(renderer, frame_meta, surf)?);
                            step_elements.extend(
                                window_popup_elements(renderer, window, phys_loc)
                                    .into_iter()
                                    .map(YawcRenderElements::from),
                            );
                        }

                        let mut frame =
                            renderer.render(&mut framebuffer, size, Transform::Flipped180)?;
                        if let Some((blur_texture, blur_element, blur_frame_meta, capture)) =
                            blur_payload.as_ref()
                        {
                            self.capture_blur_texture(
                                &mut frame,
                                blur_texture,
                                blur_frame_meta,
                                *capture,
                            )?;
                            draw_elements(&mut frame, std::slice::from_ref(blur_element))?;
                        }
                        draw_elements(&mut frame, &step_elements)?;
                        let _ = frame.finish()?;
                        continue;
                    }
                }

                if let Some(loc) = state.space.element_location(window) {
                    let render_loc = Point::<i32, Logical>::from((
                        loc.x - window.geometry().loc.x,
                        loc.y - window.geometry().loc.y,
                    ));
                    let frame_meta = frame_by_window.get(window);
                    let animation = frame_meta
                        .map(|frame| frame.animation)
                        .or_else(|| {
                            window.toplevel().map(|toplevel| {
                                state
                                    .windows
                                    .animation(toplevel.wl_surface(), animation_config)
                            })
                        })
                        .unwrap_or_default();
                    let anchor = animation_anchor(&state.space, window, frame_meta);
                    let phys_loc = Point::<i32, Physical>::from((render_loc.x, render_loc.y));
                    let surf: Vec<
                        SpaceRenderElements<
                            GlesRenderer,
                            WaylandSurfaceRenderElement<GlesRenderer>,
                        >,
                    > = window.render_elements(renderer, phys_loc, RendererScale::from(1.0), 1.0);
                    if window.toplevel().is_some() {
                        if let Some(frame_meta) = frame_meta {
                            step_elements
                                .extend(self.client_surface_elements(renderer, frame_meta, surf)?);
                        } else {
                            let has_csd_surface = !surf.is_empty();
                            let csd_rect = csd_visual_rect(window, render_loc);
                            let tiled = window
                                .toplevel()
                                .map(|toplevel| {
                                    state.windows.is_fullscreen(toplevel.wl_surface())
                                        || state.windows.is_maximized(toplevel.wl_surface())
                                })
                                .unwrap_or(false);
                            let resizing = window
                                .toplevel()
                                .map(|toplevel| state.windows.is_resizing(toplevel.wl_surface()))
                                .unwrap_or(false);
                            if !tiled && !resizing {
                                if let Some(shadow) = self.shadow_element(
                                    renderer, csd_rect, CSD_RADIUS, anchor, animation,
                                )? {
                                    step_elements.push(shadow);
                                }
                            }
                            step_elements.extend(self.csd_surface_elements(
                                renderer, csd_rect, surf, anchor, animation,
                            )?);
                            if has_csd_surface {
                                if let Some(toplevel) = window.toplevel() {
                                    csd_snapshot = Some((toplevel.wl_surface().clone(), csd_rect));
                                }
                            } else if let Some(toplevel) = window.toplevel() {
                                live_csd_surfaces.insert(toplevel.wl_surface().clone());
                            }
                        }
                    } else {
                        step_elements.extend(surf.into_iter().map(YawcRenderElements::from));
                    }
                }

                if let Some((surface, rect)) = csd_snapshot {
                    live_csd_surfaces.insert(surface.clone());
                    self.update_csd_snapshot(
                        renderer,
                        &surface,
                        size,
                        (0, 0).into(),
                        rect,
                        &step_elements,
                    )?;
                }

                let mut frame = renderer.render(&mut framebuffer, size, Transform::Flipped180)?;
                draw_elements(&mut frame, &step_elements)?;
                let _ = frame.finish()?;
            }

            let destroyed = state.windows.destroyed_close_animations(animation_config);
            let ghost_elements = self.csd_snapshot_elements(renderer, &destroyed);
            if !ghost_elements.is_empty() {
                let mut frame = renderer.render(&mut framebuffer, size, Transform::Flipped180)?;
                draw_elements(&mut frame, &ghost_elements)?;
                let _ = frame.finish()?;
            }
            self.retain_csd_snapshots(&live_csd_surfaces, &destroyed);

            if let Some(tooltip) = self.legacy_tooltip_element(
                renderer,
                &all_frames,
                pointer_location,
                Rectangle::from_size(size.to_logical(1)),
            )? {
                let mut frame = renderer.render(&mut framebuffer, size, Transform::Flipped180)?;
                draw_elements(&mut frame, &[tooltip])?;
                let _ = frame.finish()?;
            }

            let dnd_icon = dnd_icon_elements(renderer, state.dnd_icon.as_ref(), pointer_location);
            if !dnd_icon.is_empty() {
                let mut frame = renderer.render(&mut framebuffer, size, Transform::Flipped180)?;
                draw_elements(&mut frame, &dnd_icon)?;
                let _ = frame.finish()?;
            }

            if state.screencopy_state.has_pending() {
                let requests = state.screencopy_state.take_pending();
                for request in requests {
                    let captured =
                        read_xrgb_framebuffer(renderer, &framebuffer, size, request.region())
                            .map_err(|error| format!("{error:?}"));
                    request.finish(captured);
                }
            }
        }

        self.blur_texture_cache
            .retain(|surface, _| used_blur_surfaces.contains(surface));

        backend.submit(Some(&[damage]))?;

        state.space.elements().for_each(|window| {
            window.send_frame(
                &self.output,
                state.start_time.elapsed(),
                Some(std::time::Duration::ZERO),
                |_, _| Some(self.output.clone()),
            );
        });
        if let Some(icon) = state.dnd_icon.as_ref() {
            send_frames_surface_tree(
                icon,
                &self.output,
                state.start_time.elapsed(),
                Some(std::time::Duration::ZERO),
                |_, _| Some(self.output.clone()),
            );
        }

        state.refresh_space_and_prune_windows();
        state.popups.cleanup();
        let _ = display_handle.flush_clients();

        Ok(())
    }

    fn ensure_titlebar_shader(
        &mut self,
        renderer: &mut GlesRenderer,
    ) -> Result<Option<&GlesTexProgram>, GlesError> {
        if self.titlebar_shader.is_some() || self.titlebar_shader_failed {
            return Ok(self.titlebar_shader.as_ref());
        }

        match renderer.compile_custom_texture_shader(
            TITLEBAR_BLUR_SHADER,
            &[
                UniformName::new("area_size", UniformType::_2f),
                UniformName::new("texel_size", UniformType::_2f),
                UniformName::new("radius", UniformType::_1f),
                UniformName::new("src_origin", UniformType::_2f),
                UniformName::new("src_size", UniformType::_2f),
                UniformName::new("flip_y", UniformType::_1f),
            ],
        ) {
            Ok(shader) => {
                self.titlebar_shader = Some(shader);
                info!("compiled GPU titlebar blur shader");
                Ok(self.titlebar_shader.as_ref())
            }
            Err(error) => {
                self.titlebar_shader_failed = true;
                warn!(?error, "failed to compile titlebar blur shader");
                Ok(None)
            }
        }
    }

    fn ensure_blur_texture(
        &mut self,
        renderer: &mut GlesRenderer,
        surface: &WlSurface,
        width: i32,
        height: i32,
    ) -> Result<GlesTexture, GlesError> {
        if let Some(texture) = self.blur_texture_cache.get(surface) {
            let size = texture.size();
            if size.w >= width && size.h >= height {
                return Ok(texture.clone());
            }
        }

        let old_size = self
            .blur_texture_cache
            .get(surface)
            .map(|texture| texture.size())
            .unwrap_or_else(|| Size::from((0, 0)));
        let width = round_blur_texture_extent(width.max(old_size.w));
        let height = round_blur_texture_extent(height.max(old_size.h));

        let tex = renderer.with_context(|gl| unsafe {
            let mut tex = 0;
            gl.GenTextures(1, &mut tex);
            gl.BindTexture(ffi::TEXTURE_2D, tex);
            gl.TexImage2D(
                ffi::TEXTURE_2D,
                0,
                ffi::RGB8 as i32,
                width,
                height,
                0,
                ffi::RGB,
                ffi::UNSIGNED_BYTE,
                std::ptr::null(),
            );
            gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_MIN_FILTER, ffi::LINEAR as i32);
            gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_MAG_FILTER, ffi::LINEAR as i32);
            gl.TexParameteri(
                ffi::TEXTURE_2D,
                ffi::TEXTURE_WRAP_S,
                ffi::CLAMP_TO_EDGE as i32,
            );
            gl.TexParameteri(
                ffi::TEXTURE_2D,
                ffi::TEXTURE_WRAP_T,
                ffi::CLAMP_TO_EDGE as i32,
            );
            gl.BindTexture(ffi::TEXTURE_2D, 0);
            tex
        })?;
        let texture = unsafe {
            // NVIDIA rejects copying an opaque DRM framebuffer into an alpha texture
            // ("Unable to up-convert the component count"), but Smithay's GLES shader
            // selector only accepts RGBA/BGRA-like texture metadata. The GL object is
            // RGB8; reporting it as opaque RGBA8 selects the no-alpha sampler path.
            GlesTexture::from_raw(
                renderer,
                Some(ffi::RGBA8),
                true,
                tex,
                Size::from((width, height)),
            )
        };
        self.blur_texture_cache
            .insert(surface.clone(), texture.clone());
        Ok(texture)
    }

    fn capture_blur_texture(
        &self,
        frame: &mut smithay::backend::renderer::gles::GlesFrame<'_, '_>,
        texture: &GlesTexture,
        _frame_meta: &WindowFrame,
        capture: BlurCapture,
    ) -> Result<(), GlesError> {
        frame.with_context(|gl| unsafe {
            gl.BindTexture(ffi::TEXTURE_2D, texture.tex_id());
            gl.CopyTexSubImage2D(
                ffi::TEXTURE_2D,
                0,
                0,
                0,
                capture.capture_x,
                capture.capture_y_gl,
                capture.capture_w,
                capture.capture_h,
            );
            gl.BindTexture(ffi::TEXTURE_2D, 0);
        })?;

        Ok(())
    }

    fn rebuild_desktop_buffers(&mut self, size: Size<i32, Physical>) {
        self.wallpaper_image = self.wallpaper_source.as_ref().map(|image| {
            DynamicImage::ImageRgba8(image.clone())
                .resize_to_fill(
                    size.w.max(1) as u32,
                    size.h.max(1) as u32,
                    FilterType::Triangle,
                )
                .to_rgba8()
        });
        self.wallpaper_buffer = self.wallpaper_image.as_ref().map(rgba_to_buffer);
    }

    fn decoration_elements(
        &mut self,
        renderer: &mut GlesRenderer,
        frames: &[WindowFrame],
    ) -> Result<HashMap<Window, Vec<YawcRenderElements>>, GlesError> {
        let mut result: HashMap<Window, Vec<YawcRenderElements>> = HashMap::new();
        let mut used_overlay_keys = HashSet::new();

        for frame in frames {
            let overlay_key = DecorationCacheKey::from_frame(frame);
            let app_icon = frame
                .app_id
                .as_deref()
                .and_then(|app_id| self.cached_icon(app_id));
            used_overlay_keys.insert(overlay_key.clone());

            if !self.overlay_cache.contains_key(&overlay_key) {
                let frame_buffer =
                    overlay_buffer(frame, self.title_font.as_ref(), app_icon.as_ref());
                self.overlay_cache.insert(overlay_key.clone(), frame_buffer);
            }
            let frame_buffer = self.overlay_cache.get(&overlay_key).unwrap();

            let mut frame_elements: Vec<YawcRenderElements> = Vec::new();

            let overlay_rect = Rectangle::new(
                frame.frame.loc,
                (frame.frame.size.w, TITLEBAR_HEIGHT).into(),
            );
            let (overlay_loc, overlay_size) =
                animated_rect(overlay_rect, Some(frame.frame), frame.animation);

            // CPU overlay: text, icons, close button (top layer).
            frame_elements.push(YawcRenderElements::from(
                MemoryRenderBufferRenderElement::from_buffer(
                    renderer,
                    overlay_loc,
                    frame_buffer,
                    Some(decoration_animation_for_frame(frame).alpha),
                    Some(Rectangle::from_size(overlay_rect.size.to_f64())),
                    Some(overlay_size),
                    Kind::Unspecified,
                )?,
            ));

            result.insert(frame.window.clone(), frame_elements);
        }

        self.overlay_cache
            .retain(|key, _| used_overlay_keys.contains(key));

        Ok(result)
    }

    fn ensure_client_clip_shader(
        &mut self,
        renderer: &mut GlesRenderer,
    ) -> Result<Option<&GlesTexProgram>, GlesError> {
        if self.client_clip_shader.is_some() || self.client_clip_shader_failed {
            return Ok(self.client_clip_shader.as_ref());
        }

        match renderer.compile_custom_texture_shader(
            CLIENT_CLIP_SHADER,
            &[
                UniformName::new("client_size", UniformType::_2f),
                UniformName::new("element_offset", UniformType::_2f),
                UniformName::new("element_size", UniformType::_2f),
                UniformName::new("radius", UniformType::_1f),
            ],
        ) {
            Ok(shader) => {
                self.client_clip_shader = Some(shader);
                info!("compiled GPU client clip shader");
                Ok(self.client_clip_shader.as_ref())
            }
            Err(error) => {
                self.client_clip_shader_failed = true;
                warn!(?error, "failed to compile client clip shader");
                Ok(None)
            }
        }
    }

    fn ensure_csd_clip_shader(
        &mut self,
        renderer: &mut GlesRenderer,
    ) -> Result<Option<&GlesTexProgram>, GlesError> {
        if self.csd_clip_shader.is_some() || self.csd_clip_shader_failed {
            return Ok(self.csd_clip_shader.as_ref());
        }

        match renderer.compile_custom_texture_shader(
            CSD_CLIP_SHADER,
            &[
                UniformName::new("client_size", UniformType::_2f),
                UniformName::new("element_offset", UniformType::_2f),
                UniformName::new("element_size", UniformType::_2f),
                UniformName::new("radius", UniformType::_1f),
            ],
        ) {
            Ok(shader) => {
                self.csd_clip_shader = Some(shader);
                info!("compiled GPU CSD clip shader");
                Ok(self.csd_clip_shader.as_ref())
            }
            Err(error) => {
                self.csd_clip_shader_failed = true;
                warn!(?error, "failed to compile CSD clip shader");
                Ok(None)
            }
        }
    }

    fn ensure_shape_shader(
        &mut self,
        renderer: &mut GlesRenderer,
    ) -> Result<Option<&GlesTexProgram>, GlesError> {
        if self.shape_shader.is_some() || self.shape_shader_failed {
            return Ok(self.shape_shader.as_ref());
        }

        match renderer.compile_custom_texture_shader(
            SHAPE_SHADER,
            &[
                UniformName::new("area_size", UniformType::_2f),
                UniformName::new("color", UniformType::_4f),
                UniformName::new("inner_rect", UniformType::_4f),
                UniformName::new("radius", UniformType::_1f),
                UniformName::new("mode", UniformType::_1f),
                UniformName::new("spread", UniformType::_1f),
            ],
        ) {
            Ok(shader) => {
                self.shape_shader = Some(shader);
                info!("compiled GPU shape shader");
                Ok(self.shape_shader.as_ref())
            }
            Err(error) => {
                self.shape_shader_failed = true;
                warn!(?error, "failed to compile shape shader");
                Ok(None)
            }
        }
    }

    fn ensure_shape_texture(
        &mut self,
        renderer: &mut GlesRenderer,
    ) -> Result<GlesTexture, GlesError> {
        if let Some(texture) = self.shape_texture.as_ref() {
            return Ok(texture.clone());
        }

        let pixel = [255_u8, 255, 255, 255];
        let tex = renderer.with_context(|gl| unsafe {
            let mut tex = 0;
            gl.GenTextures(1, &mut tex);
            gl.BindTexture(ffi::TEXTURE_2D, tex);
            gl.TexImage2D(
                ffi::TEXTURE_2D,
                0,
                ffi::RGBA8 as i32,
                1,
                1,
                0,
                ffi::RGBA,
                ffi::UNSIGNED_BYTE,
                pixel.as_ptr().cast(),
            );
            gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_MIN_FILTER, ffi::LINEAR as i32);
            gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_MAG_FILTER, ffi::LINEAR as i32);
            gl.TexParameteri(
                ffi::TEXTURE_2D,
                ffi::TEXTURE_WRAP_S,
                ffi::CLAMP_TO_EDGE as i32,
            );
            gl.TexParameteri(
                ffi::TEXTURE_2D,
                ffi::TEXTURE_WRAP_T,
                ffi::CLAMP_TO_EDGE as i32,
            );
            gl.BindTexture(ffi::TEXTURE_2D, 0);
            tex
        })?;
        let texture = unsafe {
            GlesTexture::from_raw(renderer, Some(ffi::RGBA8), false, tex, Size::from((1, 1)))
        };
        self.shape_texture = Some(texture.clone());
        Ok(texture)
    }

    fn titlebar_fill_element(
        &mut self,
        renderer: &mut GlesRenderer,
        frame: &WindowFrame,
    ) -> Result<Option<YawcRenderElements>, GlesError> {
        if frame.frame.size.w <= 0 || TITLEBAR_HEIGHT <= 0 {
            return Ok(None);
        }

        let Some(program) = self.ensure_shape_shader(renderer)?.cloned() else {
            return Ok(None);
        };
        let texture = self.ensure_shape_texture(renderer)?;
        let rect = Rectangle::new(
            frame.frame.loc,
            (frame.frame.size.w, TITLEBAR_HEIGHT).into(),
        );
        let (animated_loc, animated_size) = animated_rect(rect, Some(frame.frame), frame.animation);

        Ok(Some(YawcRenderElements::from(ShapeElement::new(
            texture,
            program,
            animated_loc.to_i32_round(),
            Size::from((animated_size.w.max(1), animated_size.h.max(1))),
            titlebar_fill_color(frame.close_tint),
            (
                0.0,
                0.0,
                animated_size.w.max(1) as f32,
                animated_size.h.max(1) as f32,
            ),
            FRAME_RADIUS as f32,
            TITLEBAR_SHAPE_MODE,
            1.0,
            decoration_animation_for_frame(frame).alpha,
        ))))
    }

    fn legacy_tooltip_element(
        &self,
        renderer: &mut GlesRenderer,
        frames: &[WindowFrame],
        pointer_location: Option<Point<f64, Logical>>,
        bounds: Rectangle<i32, Logical>,
    ) -> Result<Option<YawcRenderElements>, GlesError> {
        let Some(pointer_location) = pointer_location else {
            return Ok(None);
        };
        let Some(font) = self.title_font.as_ref() else {
            return Ok(None);
        };
        let Some(frame) = frames.iter().rev().find(|frame| {
            legacy_badge_rect(frame).is_some_and(|rect| contains_point(rect, pointer_location))
        }) else {
            return Ok(None);
        };

        let text_width = measure_text_width(font, LEGACY_TOOLTIP_TEXT, TOOLTIP_FONT_SIZE);
        let width = (text_width + TOOLTIP_PADDING_X * 2).max(1);
        let height = TOOLTIP_HEIGHT.max(1);
        let mut image = RgbaImage::from_pixel(width as u32, height as u32, Rgba([24, 27, 31, 224]));
        draw_text(
            &mut image,
            font,
            LEGACY_TOOLTIP_TEXT,
            Rgba([245, 247, 250, 242]),
            TOOLTIP_PADDING_X,
            TOOLTIP_FONT_SIZE,
            0,
            height,
            text_width,
        );

        let badge = legacy_badge_rect(frame).expect("hovered legacy frame has a badge rect");
        let mut loc = Point::<i32, Logical>::from((badge.loc.x, badge.loc.y + badge.size.h + 8));
        let max_x = bounds.loc.x + bounds.size.w - width;
        let max_y = bounds.loc.y + bounds.size.h - height;
        loc.x = loc.x.clamp(bounds.loc.x, max_x.max(bounds.loc.x));
        loc.y = loc.y.clamp(bounds.loc.y, max_y.max(bounds.loc.y));

        let buffer = rgba_to_buffer(&image);
        Ok(Some(YawcRenderElements::from(
            MemoryRenderBufferRenderElement::from_buffer(
                renderer,
                Point::<f64, Physical>::from((loc.x as f64, loc.y as f64)),
                &buffer,
                Some(1.0),
                None,
                None,
                Kind::Unspecified,
            )?,
        )))
    }

    fn shadow_element(
        &mut self,
        renderer: &mut GlesRenderer,
        rect: Rectangle<i32, Logical>,
        radius: i32,
        anchor: Option<Rectangle<i32, Logical>>,
        animation: WindowAnimation,
    ) -> Result<Option<YawcRenderElements>, GlesError> {
        if rect.size.w <= 0 || rect.size.h <= 0 {
            return Ok(None);
        }

        let Some(program) = self.ensure_shape_shader(renderer)?.cloned() else {
            return Ok(None);
        };
        let texture = self.ensure_shape_texture(renderer)?;
        let (animated_loc, animated_size) = animated_rect(rect, anchor, animation);
        let shadow_loc = Point::<i32, Physical>::from((
            (animated_loc.x - SHADOW_PAD as f64).round() as i32,
            (animated_loc.y - SHADOW_PAD as f64 + SHADOW_OFFSET_Y as f64).round() as i32,
        ));
        let shadow_size = Size::<i32, Physical>::from((
            (animated_size.w + SHADOW_PAD * 2).max(1),
            (animated_size.h + SHADOW_PAD * 2).max(1),
        ));

        Ok(Some(YawcRenderElements::from(ShapeElement::new(
            texture,
            program,
            shadow_loc,
            shadow_size,
            (0.0, 0.0, 0.0, SHADOW_OPACITY),
            (
                SHADOW_PAD as f32,
                (SHADOW_PAD - SHADOW_OFFSET_Y) as f32,
                animated_size.w.max(1) as f32,
                animated_size.h.max(1) as f32,
            ),
            radius.max(0) as f32,
            SHADOW_SHAPE_MODE,
            SHADOW_SPREAD,
            animation.alpha,
        ))))
    }

    fn client_surface_elements(
        &mut self,
        renderer: &mut GlesRenderer,
        frame: &WindowFrame,
        surfaces: Vec<SpaceRenderElements<GlesRenderer, WaylandSurfaceRenderElement<GlesRenderer>>>,
    ) -> Result<Vec<YawcRenderElements>, GlesError> {
        let shader = self.ensure_client_clip_shader(renderer)?.cloned();
        let client_rect = Rectangle::new(
            (frame.frame.loc.x, frame.frame.loc.y + TITLEBAR_HEIGHT).into(),
            (
                frame.frame.size.w,
                (frame.frame.size.h - TITLEBAR_HEIGHT).max(0),
            )
                .into(),
        );
        let (client_loc, client_size_logical) =
            animated_rect(client_rect, Some(frame.frame), frame.animation);
        let client_size =
            Size::<i32, Physical>::from((client_size_logical.w, client_size_logical.h));

        let mut elements = Vec::with_capacity(surfaces.len());
        for surface in surfaces {
            match (shader.as_ref(), surface) {
                (Some(shader), SpaceRenderElements::Surface(surface)) if client_size.h > 0 => {
                    elements.push(YawcRenderElements::from(RoundedSurfaceElement::new(
                        surface,
                        shader.clone(),
                        client_loc,
                        client_size,
                        Some(frame.frame),
                        frame.animation,
                    )));
                }
                (_, surface) => elements.push(YawcRenderElements::from(surface)),
            }
        }

        Ok(elements)
    }

    fn csd_surface_elements(
        &mut self,
        renderer: &mut GlesRenderer,
        visual_rect: Rectangle<i32, Logical>,
        surfaces: Vec<SpaceRenderElements<GlesRenderer, WaylandSurfaceRenderElement<GlesRenderer>>>,
        anchor: Option<Rectangle<i32, Logical>>,
        animation: WindowAnimation,
    ) -> Result<Vec<YawcRenderElements>, GlesError> {
        let shader = self.ensure_csd_clip_shader(renderer)?.cloned();
        let (visual_loc, visual_size_logical) = animated_rect(visual_rect, anchor, animation);
        let visual_size =
            Size::<i32, Physical>::from((visual_size_logical.w, visual_size_logical.h));

        let mut elements = Vec::with_capacity(surfaces.len());
        for surface in surfaces {
            match (shader.as_ref(), surface) {
                (Some(shader), SpaceRenderElements::Surface(surface)) if visual_size.h > 0 => {
                    elements.push(YawcRenderElements::from(CsdRoundedSurfaceElement::new(
                        surface,
                        shader.clone(),
                        visual_loc.to_i32_round(),
                        visual_size,
                        anchor,
                        animation,
                    )));
                }
                (_, surface) => {
                    elements.extend(animated_surface_elements(vec![surface], anchor, animation));
                }
            }
        }

        Ok(elements)
    }

    fn cached_icon(&mut self, app_id: &str) -> Option<RgbaImage> {
        if let Some(icon) = self.icon_cache.get(app_id) {
            return icon.clone();
        }

        let icon = load_app_icon(app_id);
        self.icon_cache.insert(app_id.to_string(), icon.clone());
        icon
    }
}

#[derive(Debug)]
struct ShapeElement {
    texture: GlesTexture,
    program: GlesTexProgram,
    id: Id,
    loc: Point<i32, Physical>,
    size: Size<i32, Physical>,
    color: (f32, f32, f32, f32),
    inner_rect: (f32, f32, f32, f32),
    radius: f32,
    mode: f32,
    spread: f32,
    alpha: f32,
}

impl ShapeElement {
    fn new(
        texture: GlesTexture,
        program: GlesTexProgram,
        loc: Point<i32, Physical>,
        size: Size<i32, Physical>,
        color: (f32, f32, f32, f32),
        inner_rect: (f32, f32, f32, f32),
        radius: f32,
        mode: f32,
        spread: f32,
        alpha: f32,
    ) -> Self {
        Self {
            texture,
            program,
            id: Id::new(),
            loc,
            size,
            color,
            inner_rect,
            radius,
            mode,
            spread,
            alpha,
        }
    }
}

impl Element for ShapeElement {
    fn id(&self) -> &Id {
        &self.id
    }

    fn current_commit(&self) -> CommitCounter {
        CommitCounter::default()
    }

    fn geometry(&self, _scale: RendererScale<f64>) -> Rectangle<i32, Physical> {
        Rectangle::new(self.loc, self.size)
    }

    fn transform(&self) -> Transform {
        Transform::Normal
    }

    fn src(&self) -> Rectangle<f64, smithay::utils::Buffer> {
        Rectangle::new((0.0, 0.0).into(), (1.0, 1.0).into())
    }

    fn damage_since(
        &self,
        _scale: RendererScale<f64>,
        _commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        DamageSet::from_slice(&[Rectangle::from_size(self.size)])
    }

    fn opaque_regions(&self, _scale: RendererScale<f64>) -> OpaqueRegions<i32, Physical> {
        OpaqueRegions::default()
    }

    fn alpha(&self) -> f32 {
        self.alpha
    }

    fn kind(&self) -> Kind {
        Kind::Unspecified
    }

    fn location(&self, _scale: RendererScale<f64>) -> Point<i32, Physical> {
        self.loc
    }
}

impl RenderElement<GlesRenderer> for ShapeElement {
    fn draw(
        &self,
        frame: &mut smithay::backend::renderer::gles::GlesFrame<'_, '_>,
        src: Rectangle<f64, smithay::utils::Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), GlesError> {
        let uniforms = vec![
            Uniform::new(
                "area_size",
                (dst.size.w.max(1) as f32, dst.size.h.max(1) as f32),
            ),
            Uniform::new("color", self.color),
            Uniform::new("inner_rect", self.inner_rect),
            Uniform::new("radius", self.radius),
            Uniform::new("mode", self.mode),
            Uniform::new("spread", self.spread),
        ];

        frame.render_texture_from_to(
            &self.texture,
            src,
            dst,
            damage,
            &[],
            Transform::Normal,
            self.alpha(),
            Some(&self.program),
            &uniforms,
        )
    }
}

#[derive(Debug)]
struct TitlebarBlurElement {
    texture: GlesTexture,
    program: GlesTexProgram,
    id: Id,
    titlebar_loc: Point<i32, Physical>,
    size: Size<i32, Physical>,
    capture: BlurCapture,
    alpha: f32,
}

impl TitlebarBlurElement {
    #[cfg_attr(not(feature = "tty-udev"), allow(dead_code))]
    fn new(
        texture: GlesTexture,
        program: GlesTexProgram,
        loc: Point<i32, Logical>,
        size: Size<i32, Physical>,
        capture: BlurCapture,
        anchor: Option<Rectangle<i32, Logical>>,
        animation: WindowAnimation,
    ) -> Self {
        let rect = Rectangle::new(loc, (size.w, size.h).into());
        let (animated_loc, animated_size) = animated_rect(rect, anchor, animation);
        Self {
            texture,
            program,
            id: Id::new(),
            titlebar_loc: animated_loc.to_i32_round(),
            size: Size::from((animated_size.w, animated_size.h)),
            capture,
            alpha: animation.alpha,
        }
    }
}

impl Element for TitlebarBlurElement {
    fn id(&self) -> &Id {
        &self.id
    }

    fn current_commit(&self) -> CommitCounter {
        CommitCounter::default()
    }

    fn geometry(&self, _scale: RendererScale<f64>) -> Rectangle<i32, Physical> {
        Rectangle::new(self.titlebar_loc, self.size)
    }

    fn transform(&self) -> Transform {
        Transform::Normal
    }

    fn src(&self) -> Rectangle<f64, smithay::utils::Buffer> {
        Rectangle::new(
            (self.capture.src_x as f64, self.capture.src_y as f64).into(),
            (self.size.w as f64, self.size.h as f64).into(),
        )
    }

    fn damage_since(
        &self,
        _scale: RendererScale<f64>,
        _commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        DamageSet::from_slice(&[Rectangle::from_size(
            self.geometry(RendererScale::from(1.0_f64)).size,
        )])
    }

    fn opaque_regions(&self, _scale: RendererScale<f64>) -> OpaqueRegions<i32, Physical> {
        OpaqueRegions::default()
    }

    fn alpha(&self) -> f32 {
        self.alpha
    }

    fn kind(&self) -> Kind {
        Kind::Unspecified
    }

    fn location(&self, _scale: RendererScale<f64>) -> Point<i32, Physical> {
        self.titlebar_loc
    }
}

impl RenderElement<GlesRenderer> for TitlebarBlurElement {
    fn draw(
        &self,
        frame: &mut smithay::backend::renderer::gles::GlesFrame<'_, '_>,
        src: Rectangle<f64, smithay::utils::Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), GlesError> {
        frame.with_context(|gl| unsafe {
            gl.BindTexture(ffi::TEXTURE_2D, self.texture.tex_id());
            gl.CopyTexSubImage2D(
                ffi::TEXTURE_2D,
                0,
                0,
                0,
                self.capture.capture_x,
                self.capture.capture_y_gl,
                self.capture.capture_w,
                self.capture.capture_h,
            );
            gl.BindTexture(ffi::TEXTURE_2D, 0);
        })?;

        let uniforms = titlebar_shader_uniforms(
            self.size.w,
            self.size.h,
            self.texture.size().w,
            self.texture.size().h,
            self.capture.src_x,
            self.capture.src_y,
            self.capture.flip_y,
        );
        frame.render_texture_from_to(
            &self.texture,
            src,
            dst,
            damage,
            &[],
            Transform::Normal,
            self.alpha(),
            Some(&self.program),
            &uniforms,
        )
    }
}

#[derive(Debug)]
struct RoundedSurfaceElement {
    inner: WaylandSurfaceRenderElement<GlesRenderer>,
    program: GlesTexProgram,
    id: Id,
    client_loc: Point<i32, Physical>,
    client_size: Size<i32, Physical>,
    anchor: Option<Rectangle<i32, Logical>>,
    animation: WindowAnimation,
}

impl RoundedSurfaceElement {
    fn new(
        inner: WaylandSurfaceRenderElement<GlesRenderer>,
        program: GlesTexProgram,
        client_loc: Point<f64, Physical>,
        client_size: Size<i32, Physical>,
        anchor: Option<Rectangle<i32, Logical>>,
        animation: WindowAnimation,
    ) -> Self {
        Self {
            inner,
            program,
            id: Id::new(),
            client_loc: client_loc.to_i32_round(),
            client_size,
            anchor,
            animation,
        }
    }
}

impl Element for RoundedSurfaceElement {
    fn id(&self) -> &Id {
        &self.id
    }

    fn current_commit(&self) -> CommitCounter {
        self.inner.current_commit()
    }

    fn geometry(&self, scale: RendererScale<f64>) -> Rectangle<i32, Physical> {
        animated_physical_rect(self.inner.geometry(scale), self.anchor, self.animation)
    }

    fn transform(&self) -> Transform {
        self.inner.transform()
    }

    fn src(&self) -> Rectangle<f64, smithay::utils::Buffer> {
        self.inner.src()
    }

    fn damage_since(
        &self,
        scale: RendererScale<f64>,
        _commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        DamageSet::from_slice(&[self.geometry(scale)])
    }

    fn opaque_regions(&self, _scale: RendererScale<f64>) -> OpaqueRegions<i32, Physical> {
        OpaqueRegions::default()
    }

    fn alpha(&self) -> f32 {
        self.inner.alpha() * self.animation.alpha
    }

    fn kind(&self) -> Kind {
        self.inner.kind()
    }

    fn location(&self, scale: RendererScale<f64>) -> Point<i32, Physical> {
        self.geometry(scale).loc
    }
}

impl RenderElement<GlesRenderer> for RoundedSurfaceElement {
    fn draw(
        &self,
        frame: &mut smithay::backend::renderer::gles::GlesFrame<'_, '_>,
        src: Rectangle<f64, smithay::utils::Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), GlesError> {
        match self.inner.texture() {
            WaylandSurfaceTexture::Texture(texture) => {
                let uniforms = vec![
                    Uniform::new(
                        "client_size",
                        (
                            self.client_size.w.max(1) as f32,
                            self.client_size.h.max(1) as f32,
                        ),
                    ),
                    Uniform::new(
                        "element_offset",
                        (
                            (dst.loc.x - self.client_loc.x) as f32,
                            (dst.loc.y - self.client_loc.y) as f32,
                        ),
                    ),
                    Uniform::new("element_size", (dst.size.w as f32, dst.size.h as f32)),
                    Uniform::new("radius", FRAME_RADIUS as f32),
                ];

                frame.render_texture_from_to(
                    texture,
                    src,
                    dst,
                    damage,
                    &[],
                    self.transform(),
                    self.alpha(),
                    Some(&self.program),
                    &uniforms,
                )
            }
            WaylandSurfaceTexture::SolidColor(color) => frame.draw_solid(dst, damage, *color),
        }
    }
}

#[derive(Debug)]
struct CsdRoundedSurfaceElement {
    inner: WaylandSurfaceRenderElement<GlesRenderer>,
    program: GlesTexProgram,
    id: Id,
    visual_loc: Point<i32, Physical>,
    visual_size: Size<i32, Physical>,
    anchor: Option<Rectangle<i32, Logical>>,
    animation: WindowAnimation,
}

impl CsdRoundedSurfaceElement {
    fn new(
        inner: WaylandSurfaceRenderElement<GlesRenderer>,
        program: GlesTexProgram,
        visual_loc: Point<i32, Physical>,
        visual_size: Size<i32, Physical>,
        anchor: Option<Rectangle<i32, Logical>>,
        animation: WindowAnimation,
    ) -> Self {
        Self {
            inner,
            program,
            id: Id::new(),
            visual_loc,
            visual_size,
            anchor,
            animation,
        }
    }
}

impl Element for CsdRoundedSurfaceElement {
    fn id(&self) -> &Id {
        &self.id
    }

    fn current_commit(&self) -> CommitCounter {
        self.inner.current_commit()
    }

    fn geometry(&self, scale: RendererScale<f64>) -> Rectangle<i32, Physical> {
        animated_physical_rect(self.inner.geometry(scale), self.anchor, self.animation)
    }

    fn transform(&self) -> Transform {
        self.inner.transform()
    }

    fn src(&self) -> Rectangle<f64, smithay::utils::Buffer> {
        self.inner.src()
    }

    fn damage_since(
        &self,
        scale: RendererScale<f64>,
        _commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        DamageSet::from_slice(&[self.geometry(scale)])
    }

    fn opaque_regions(&self, _scale: RendererScale<f64>) -> OpaqueRegions<i32, Physical> {
        OpaqueRegions::default()
    }

    fn alpha(&self) -> f32 {
        self.inner.alpha() * self.animation.alpha
    }

    fn kind(&self) -> Kind {
        self.inner.kind()
    }

    fn location(&self, scale: RendererScale<f64>) -> Point<i32, Physical> {
        self.geometry(scale).loc
    }
}

impl RenderElement<GlesRenderer> for CsdRoundedSurfaceElement {
    fn draw(
        &self,
        frame: &mut smithay::backend::renderer::gles::GlesFrame<'_, '_>,
        src: Rectangle<f64, smithay::utils::Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), GlesError> {
        match self.inner.texture() {
            WaylandSurfaceTexture::Texture(texture) => {
                let uniforms = vec![
                    Uniform::new(
                        "client_size",
                        (
                            self.visual_size.w.max(1) as f32,
                            self.visual_size.h.max(1) as f32,
                        ),
                    ),
                    Uniform::new(
                        "element_offset",
                        (
                            (dst.loc.x - self.visual_loc.x) as f32,
                            (dst.loc.y - self.visual_loc.y) as f32,
                        ),
                    ),
                    Uniform::new("element_size", (dst.size.w as f32, dst.size.h as f32)),
                    Uniform::new("radius", CSD_RADIUS as f32),
                ];

                frame.render_texture_from_to(
                    texture,
                    src,
                    dst,
                    damage,
                    &[],
                    self.transform(),
                    self.alpha(),
                    Some(&self.program),
                    &uniforms,
                )
            }
            WaylandSurfaceTexture::SolidColor(color) => frame.draw_solid(dst, damage, *color),
        }
    }
}

#[derive(Debug)]
struct AnimatedSurfaceElement {
    inner: WaylandSurfaceRenderElement<GlesRenderer>,
    id: Id,
    anchor: Option<Rectangle<i32, Logical>>,
    animation: WindowAnimation,
}

impl AnimatedSurfaceElement {
    fn new(
        inner: WaylandSurfaceRenderElement<GlesRenderer>,
        anchor: Option<Rectangle<i32, Logical>>,
        animation: WindowAnimation,
    ) -> Self {
        Self {
            inner,
            id: Id::new(),
            anchor,
            animation,
        }
    }
}

impl Element for AnimatedSurfaceElement {
    fn id(&self) -> &Id {
        &self.id
    }

    fn current_commit(&self) -> CommitCounter {
        self.inner.current_commit()
    }

    fn geometry(&self, scale: RendererScale<f64>) -> Rectangle<i32, Physical> {
        animated_physical_rect(self.inner.geometry(scale), self.anchor, self.animation)
    }

    fn transform(&self) -> Transform {
        self.inner.transform()
    }

    fn src(&self) -> Rectangle<f64, smithay::utils::Buffer> {
        self.inner.src()
    }

    fn damage_since(
        &self,
        scale: RendererScale<f64>,
        _commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        DamageSet::from_slice(&[self.geometry(scale)])
    }

    fn opaque_regions(&self, _scale: RendererScale<f64>) -> OpaqueRegions<i32, Physical> {
        OpaqueRegions::default()
    }

    fn alpha(&self) -> f32 {
        self.inner.alpha() * self.animation.alpha
    }

    fn kind(&self) -> Kind {
        self.inner.kind()
    }

    fn location(&self, scale: RendererScale<f64>) -> Point<i32, Physical> {
        self.geometry(scale).loc
    }
}

impl RenderElement<GlesRenderer> for AnimatedSurfaceElement {
    fn draw(
        &self,
        frame: &mut smithay::backend::renderer::gles::GlesFrame<'_, '_>,
        src: Rectangle<f64, smithay::utils::Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), GlesError> {
        match self.inner.texture() {
            WaylandSurfaceTexture::Texture(texture) => frame.render_texture_from_to(
                texture,
                src,
                dst,
                damage,
                &[],
                self.transform(),
                self.alpha(),
                None,
                &[],
            ),
            WaylandSurfaceTexture::SolidColor(color) => frame.draw_solid(dst, damage, *color),
        }
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct DecorationCacheKey {
    frame_w: i32,
    active: bool,
    maximized: bool,
    controls_mode: WindowControlsMode,
    close_tint: u8,
    legacy_x11: bool,
    title: String,
    app_id: Option<String>,
}

impl DecorationCacheKey {
    fn from_frame(frame: &WindowFrame) -> Self {
        Self {
            frame_w: frame.frame.size.w,
            active: frame.active,
            maximized: frame.maximized,
            controls_mode: frame.controls_mode,
            close_tint: (frame.close_tint.clamp(0.0, 1.0) * 255.0).round() as u8,
            legacy_x11: frame.legacy_x11,
            title: frame.title.clone(),
            app_id: frame.app_id.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
#[cfg_attr(not(feature = "tty-udev"), allow(dead_code))]
enum BlurOrigin {
    BottomLeft,
    TopLeft,
}

#[derive(Clone, Copy, Debug)]
struct BlurCapture {
    dst_loc: Point<i32, Logical>,
    dst_w: i32,
    dst_h: i32,
    capture_x: i32,
    capture_y_gl: i32,
    capture_w: i32,
    capture_h: i32,
    src_x: i32,
    src_y: i32,
    flip_y: bool,
}

impl BlurCapture {
    #[cfg_attr(not(feature = "tty-udev"), allow(dead_code))]
    fn dst_size(self) -> Size<i32, Physical> {
        Size::from((self.dst_w, self.dst_h))
    }
}

fn blur_capture_for_frame(
    frame: &WindowFrame,
    output_geometry: Rectangle<i32, Logical>,
    origin: BlurOrigin,
) -> Option<BlurCapture> {
    let titlebar_w = frame.frame.size.w.max(1);
    let titlebar_h = TITLEBAR_HEIGHT.max(1);
    let output_size = output_geometry.size;
    let local_frame = Rectangle::new(
        (
            frame.frame.loc.x - output_geometry.loc.x,
            frame.frame.loc.y - output_geometry.loc.y,
        )
            .into(),
        frame.frame.size,
    );

    let dst_left = local_frame.loc.x.max(0).min(output_size.w);
    let dst_top = local_frame.loc.y.max(0).min(output_size.h);
    let dst_right = (local_frame.loc.x + titlebar_w).max(0).min(output_size.w);
    let dst_bottom = (local_frame.loc.y + titlebar_h).max(0).min(output_size.h);
    let dst_w = dst_right - dst_left;
    let dst_h = dst_bottom - dst_top;
    if dst_w <= 0 || dst_h <= 0 {
        return None;
    }

    let left = (dst_left - BLUR_PAD_X).max(0);
    let top = (dst_top - BLUR_PAD_Y).max(0);
    let right = (dst_right + BLUR_PAD_X).min(output_size.w);
    let bottom = (dst_bottom + BLUR_PAD_Y).min(output_size.h);

    let capture_w = right - left;
    let capture_h = bottom - top;
    if capture_w <= 0 || capture_h <= 0 {
        return None;
    }

    let (capture_y_gl, flip_y) = match origin {
        BlurOrigin::BottomLeft => (output_size.h - bottom, true),
        BlurOrigin::TopLeft => (top, false),
    };

    Some(BlurCapture {
        dst_loc: Point::from((
            dst_left + output_geometry.loc.x,
            dst_top + output_geometry.loc.y,
        )),
        dst_w,
        dst_h,
        capture_x: left,
        capture_y_gl,
        capture_w,
        capture_h,
        src_x: dst_left - left,
        src_y: dst_top - top,
        flip_y,
    })
}

fn desktop_elements(
    renderer: &mut GlesRenderer,
    wallpaper: Option<&MemoryRenderBuffer>,
    location: Point<f64, Logical>,
) -> Result<Vec<YawcRenderElements>, GlesError> {
    let mut elements = Vec::new();

    if let Some(wallpaper) = wallpaper {
        elements.push(YawcRenderElements::from(
            MemoryRenderBufferRenderElement::from_buffer(
                renderer,
                location.to_physical(1.0),
                wallpaper,
                None,
                None,
                None,
                Kind::Unspecified,
            )?,
        ));
    }

    Ok(elements)
}

fn draw_elements(
    frame: &mut smithay::backend::renderer::gles::GlesFrame<'_, '_>,
    elements: &[YawcRenderElements],
) -> Result<(), GlesError> {
    for element in elements {
        let geometry = element.geometry(RendererScale::from(1.0));
        if geometry.size.w <= 0 || geometry.size.h <= 0 {
            continue;
        }
        let local_damage = [Rectangle::from_size(geometry.size)];
        element.draw(frame, element.src(), geometry, &local_damage, &[])?;
    }

    Ok(())
}

fn read_xrgb_framebuffer(
    renderer: &mut GlesRenderer,
    framebuffer: &GlesTarget<'_>,
    output_size: Size<i32, Physical>,
    region: Rectangle<i32, Logical>,
) -> Result<CapturedFrame, GlesError> {
    let width = region.size.w.max(1);
    let height = region.size.h.max(1);
    let read_y = output_size.h - region.loc.y - height;
    let read_region =
        Rectangle::<i32, Buffer>::new((region.loc.x, read_y.max(0)).into(), (width, height).into());
    let mapping = renderer.copy_framebuffer(framebuffer, read_region, Fourcc::Xrgb8888)?;
    let raw = renderer.map_texture(&mapping)?;
    let stride = width * 4;
    let mut data = vec![0_u8; (stride * height) as usize];
    let row_bytes = stride as usize;
    let height_usize = height as usize;

    for row in 0..height_usize {
        let src_start = row * row_bytes;
        let dst_start = row * row_bytes;
        data[dst_start..dst_start + row_bytes]
            .copy_from_slice(&raw[src_start..src_start + row_bytes]);
    }

    Ok(CapturedFrame {
        size: Size::from((width, height)),
        stride,
        data,
    })
}

fn draw_elements_back_to_front_with_offset(
    frame: &mut smithay::backend::renderer::gles::GlesFrame<'_, '_>,
    elements: &[YawcRenderElements],
    offset: Point<i32, Physical>,
) -> Result<(), GlesError> {
    for element in elements.iter().rev() {
        let geometry = element.geometry(RendererScale::from(1.0));
        if geometry.size.w <= 0 || geometry.size.h <= 0 {
            continue;
        }

        let geometry = Rectangle::new(
            (geometry.loc.x - offset.x, geometry.loc.y - offset.y).into(),
            geometry.size,
        );
        let local_damage = [Rectangle::from_size(geometry.size)];
        element.draw(frame, element.src(), geometry, &local_damage, &[])?;
    }

    Ok(())
}

fn decoration_animation_for_frame(frame: &WindowFrame) -> WindowAnimation {
    let mut animation = frame.animation;
    animation.alpha *= frame.decoration_opacity;
    animation
}

fn window_popup_elements(
    renderer: &mut GlesRenderer,
    window: &Window,
    location: Point<i32, Physical>,
) -> Vec<SpaceRenderElements<GlesRenderer, WaylandSurfaceRenderElement<GlesRenderer>>> {
    let Some(surface) = window.toplevel().map(|toplevel| toplevel.wl_surface()) else {
        return Vec::new();
    };

    PopupManager::popups_for_surface(surface)
        .flat_map(|(popup, popup_offset)| {
            let offset = (window.geometry().loc + popup_offset - popup.geometry().loc)
                .to_physical_precise_round(RendererScale::from(1.0));

            render_elements_from_surface_tree(
                renderer,
                popup.wl_surface(),
                location + offset,
                RendererScale::from(1.0),
                1.0,
                Kind::Unspecified,
            )
        })
        .collect()
}

fn window_wl_surface(window: &Window) -> Option<WlSurface> {
    if let Some(toplevel) = window.toplevel() {
        return Some(toplevel.wl_surface().clone());
    }

    #[cfg(feature = "xwayland")]
    if let Some(surface) = window.x11_surface() {
        return surface.wl_surface();
    }

    None
}

#[cfg(feature = "tty-udev")]
fn capture_cursor_element(
    renderer: &mut GlesRenderer,
    cursor: CaptureCursor,
) -> Result<Option<YawcRenderElements>, GlesError> {
    if cursor.location.x.is_nan() || cursor.location.y.is_nan() {
        return Ok(None);
    }

    Ok(Some(YawcRenderElements::from(
        MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            Point::<f64, Physical>::from((
                cursor.location.x - cursor.hotspot.x as f64,
                cursor.location.y - cursor.hotspot.y as f64,
            )),
            &cursor.buffer,
            None,
            None,
            None,
            Kind::Cursor,
        )?,
    )))
}

fn csd_visual_rect(window: &Window, render_loc: Point<i32, Logical>) -> Rectangle<i32, Logical> {
    let bbox = window.bbox();
    let geometry = window.geometry();
    let size = if bbox.size.w > 0 && bbox.size.h > 0 {
        bbox.size
    } else if geometry.size.w > 0 && geometry.size.h > 0 {
        geometry.size
    } else {
        (1, 1).into()
    };

    Rectangle::new(
        (render_loc.x + bbox.loc.x, render_loc.y + bbox.loc.y).into(),
        size,
    )
}

pub fn dnd_icon_elements(
    renderer: &mut GlesRenderer,
    icon: Option<&WlSurface>,
    pointer_location: Option<Point<f64, Logical>>,
) -> Vec<YawcRenderElements> {
    let (Some(icon), Some(location)) = (icon, pointer_location) else {
        return Vec::new();
    };

    let location =
        Point::<i32, Physical>::from((location.x.round() as i32, location.y.round() as i32));
    let surfaces: Vec<
        SpaceRenderElements<GlesRenderer, WaylandSurfaceRenderElement<GlesRenderer>>,
    > = render_elements_from_surface_tree(
        renderer,
        icon,
        location,
        RendererScale::from(1.0),
        1.0,
        Kind::Cursor,
    );

    surfaces.into_iter().map(YawcRenderElements::from).collect()
}

fn titlebar_shader_uniforms(
    width: i32,
    height: i32,
    texture_w: i32,
    texture_h: i32,
    src_x: i32,
    src_y: i32,
    flip_y: bool,
) -> Vec<Uniform<'static>> {
    vec![
        Uniform::new("area_size", (width as f32, height as f32)),
        Uniform::new(
            "texel_size",
            (1.0 / texture_w.max(1) as f32, 1.0 / texture_h.max(1) as f32),
        ),
        Uniform::new("radius", FRAME_RADIUS as f32),
        Uniform::new(
            "src_origin",
            (
                src_x as f32 / texture_w.max(1) as f32,
                src_y as f32 / texture_h.max(1) as f32,
            ),
        ),
        Uniform::new(
            "src_size",
            (
                width.max(1) as f32 / texture_w.max(1) as f32,
                height.max(1) as f32 / texture_h.max(1) as f32,
            ),
        ),
        Uniform::new("flip_y", if flip_y { 1.0 } else { 0.0 }),
    ]
}

fn round_blur_texture_extent(value: i32) -> i32 {
    const STEP: i32 = 256;
    ((value.max(1) + STEP - 1) / STEP) * STEP
}

// Overlay with text, icon, and SSD buttons. The tint below it is GPU-rendered.
fn overlay_buffer(
    frame: &WindowFrame,
    title_font: Option<&Font<'static>>,
    app_icon: Option<&RgbaImage>,
) -> MemoryRenderBuffer {
    let width = frame.frame.size.w.max(1) as u32;
    let height = TITLEBAR_HEIGHT.max(1) as u32;
    let mut image = RgbaImage::from_pixel(width, height, Rgba([0, 0, 0, 0]));

    let title = if frame.title.trim().is_empty() {
        "Untitled"
    } else {
        frame.title.as_str()
    };

    if frame.legacy_x11 {
        let badge = legacy_badge_local_rect();
        draw_warning_badge(&mut image, badge.loc.x, badge.loc.y, badge.size.w);
    }

    if let Some(font) = title_font {
        let content_left = if frame.legacy_x11 {
            TITLE_PADDING + LEGACY_BADGE_SIZE + LEGACY_BADGE_GAP
        } else {
            TITLE_PADDING
        };
        let content_right = match frame.controls_mode {
            WindowControlsMode::Buttons => {
                let minimize_left = frame.minimize_button.loc.x - frame.frame.loc.x;
                (minimize_left - BUTTON_PADDING).max(content_left)
            }
            WindowControlsMode::Gestures => (frame.frame.size.w - TITLE_PADDING).max(content_left),
        };
        let text_width = measure_text_width(font, title, TITLE_FONT_SIZE);
        let icon_width = app_icon.map(|_| ICON_SIZE as i32).unwrap_or(0);
        let group_width = text_width
            + if icon_width > 0 {
                icon_width + ICON_GAP
            } else {
                0
            };
        let available_width = (content_right - content_left).max(0);
        let start_x = (content_left + (available_width - group_width).max(0) / 2).max(content_left);
        let icon_y = ((TITLEBAR_HEIGHT - ICON_SIZE as i32) / 2).max(0);
        let text_x = start_x
            + if icon_width > 0 {
                icon_width + ICON_GAP
            } else {
                0
            };

        if let Some(icon) = app_icon {
            overlay_image(&mut image, icon, start_x, icon_y);
        }

        draw_text(
            &mut image,
            font,
            title,
            TITLE_COLOR,
            text_x,
            TITLE_FONT_SIZE,
            0,
            TITLEBAR_HEIGHT,
            (content_right - text_x).max(0),
        );
    }

    if frame.controls_mode == WindowControlsMode::Buttons {
        let minimize_button = Rectangle::new(
            (
                frame.minimize_button.loc.x - frame.frame.loc.x,
                frame.minimize_button.loc.y - frame.frame.loc.y,
            )
                .into(),
            frame.minimize_button.size,
        );
        draw_minimize_button(&mut image, minimize_button, CLOSE_COLOR);

        let maximize_button = Rectangle::new(
            (
                frame.maximize_button.loc.x - frame.frame.loc.x,
                frame.maximize_button.loc.y - frame.frame.loc.y,
            )
                .into(),
            frame.maximize_button.size,
        );
        draw_maximize_button(&mut image, maximize_button, CLOSE_COLOR, frame.maximized);

        let close_button = Rectangle::new(
            (
                frame.close_button.loc.x - frame.frame.loc.x,
                frame.close_button.loc.y - frame.frame.loc.y,
            )
                .into(),
            frame.close_button.size,
        );
        draw_close_button(&mut image, close_button, CLOSE_COLOR);
    }

    rgba_to_buffer(&image)
}

fn titlebar_fill_color(close_tint: f32) -> (f32, f32, f32, f32) {
    let red = [182, 86, 92, 158];
    let tint = close_tint.clamp(0.0, 1.0);
    let mut color = [0_u8; 4];
    for (index, value) in color.iter_mut().enumerate() {
        *value = (FRAME_FILL_RGBA[index] as f32
            + (red[index] as f32 - FRAME_FILL_RGBA[index] as f32) * tint)
            .round()
            .clamp(0.0, 255.0) as u8;
    }
    (
        color[0] as f32 / 255.0,
        color[1] as f32 / 255.0,
        color[2] as f32 / 255.0,
        color[3] as f32 / 255.0,
    )
}

fn legacy_badge_local_rect() -> Rectangle<i32, Logical> {
    Rectangle::new(
        (
            TITLE_PADDING,
            ((TITLEBAR_HEIGHT - LEGACY_BADGE_SIZE) / 2).max(0),
        )
            .into(),
        (LEGACY_BADGE_SIZE, LEGACY_BADGE_SIZE).into(),
    )
}

fn legacy_badge_rect(frame: &WindowFrame) -> Option<Rectangle<i32, Logical>> {
    if !frame.legacy_x11 {
        return None;
    }

    let local = legacy_badge_local_rect();
    Some(Rectangle::new(
        (
            frame.frame.loc.x + local.loc.x,
            frame.frame.loc.y + local.loc.y,
        )
            .into(),
        local.size,
    ))
}

fn contains_point(rect: Rectangle<i32, Logical>, point: Point<f64, Logical>) -> bool {
    point.x >= rect.loc.x as f64
        && point.y >= rect.loc.y as f64
        && point.x < (rect.loc.x + rect.size.w) as f64
        && point.y < (rect.loc.y + rect.size.h) as f64
}

fn draw_warning_badge(image: &mut RgbaImage, x: i32, y: i32, size: i32) {
    let ax = x as f32 + size as f32 / 2.0;
    let ay = y as f32 + 1.0;
    let bx = x as f32 + 1.0;
    let by = y as f32 + size as f32 - 1.0;
    let cx = x as f32 + size as f32 - 1.0;
    let cy = by;

    for py in y..(y + size) {
        for px in x..(x + size) {
            if px < 0 || py < 0 || px >= image.width() as i32 || py >= image.height() as i32 {
                continue;
            }

            let sample_x = px as f32 + 0.5;
            let sample_y = py as f32 + 0.5;
            if point_in_triangle(sample_x, sample_y, (ax, ay), (bx, by), (cx, cy)) {
                blend_pixel(image, px as u32, py as u32, LEGACY_BADGE_COLOR, 1.0);
            }
        }
    }

    let mark_x = x + size / 2;
    for py in (y + 5)..(y + size - 5) {
        for px in (mark_x - 1)..=(mark_x + 1) {
            if px >= 0 && py >= 0 && px < image.width() as i32 && py < image.height() as i32 {
                blend_pixel(image, px as u32, py as u32, LEGACY_BADGE_MARK_COLOR, 1.0);
            }
        }
    }
    for py in (y + size - 4)..(y + size - 2) {
        for px in (mark_x - 1)..=(mark_x + 1) {
            if px >= 0 && py >= 0 && px < image.width() as i32 && py < image.height() as i32 {
                blend_pixel(image, px as u32, py as u32, LEGACY_BADGE_MARK_COLOR, 1.0);
            }
        }
    }
}

fn point_in_triangle(px: f32, py: f32, a: (f32, f32), b: (f32, f32), c: (f32, f32)) -> bool {
    let d1 = triangle_edge(px, py, a, b);
    let d2 = triangle_edge(px, py, b, c);
    let d3 = triangle_edge(px, py, c, a);
    let has_neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let has_pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(has_neg && has_pos)
}

fn triangle_edge(px: f32, py: f32, a: (f32, f32), b: (f32, f32)) -> f32 {
    (px - b.0) * (a.1 - b.1) - (a.0 - b.0) * (py - b.1)
}

fn draw_text(
    image: &mut RgbaImage,
    font: &Font<'static>,
    text: &str,
    color: Rgba<u8>,
    start_x: i32,
    font_size: f32,
    top: i32,
    height: i32,
    max_width: i32,
) {
    let scale = Scale::uniform(font_size);
    let metrics = font.v_metrics(scale);
    let text_height = metrics.ascent - metrics.descent;
    let baseline_y = (top as f32 + ((height as f32 - text_height) / 2.0)) + metrics.ascent - 1.0;
    let max_x = start_x + max_width.max(0);
    let max_y = top + height;

    for glyph in font.layout(text, scale, point(start_x as f32, baseline_y)) {
        let Some(bounds) = glyph.pixel_bounding_box() else {
            continue;
        };
        if bounds.min.x >= max_x {
            break;
        }

        glyph.draw(|x, y, coverage| {
            let px = bounds.min.x + x as i32;
            let py = bounds.min.y + y as i32;
            if px < start_x
                || px >= max_x
                || py < top
                || py >= max_y
                || px >= image.width() as i32
                || py >= image.height() as i32
            {
                return;
            }

            blend_pixel(image, px as u32, py as u32, color, coverage);
        });
    }
}

fn measure_text_width(font: &Font<'static>, text: &str, font_size: f32) -> i32 {
    let scale = Scale::uniform(font_size);
    let glyphs = font.layout(text, scale, point(0.0, 0.0));
    let mut min_x = i32::MAX;
    let mut max_x = i32::MIN;

    for glyph in glyphs {
        let Some(bounds) = glyph.pixel_bounding_box() else {
            continue;
        };
        min_x = min_x.min(bounds.min.x);
        max_x = max_x.max(bounds.max.x);
    }

    if min_x == i32::MAX || max_x == i32::MIN {
        0
    } else {
        (max_x - min_x).max(0)
    }
}

fn blend_pixel(image: &mut RgbaImage, x: u32, y: u32, color: Rgba<u8>, coverage: f32) {
    let dst = image.get_pixel_mut(x, y);
    let src_alpha = (color[3] as f32 / 255.0) * coverage.clamp(0.0, 1.0);
    let inv_alpha = 1.0 - src_alpha;

    for channel in 0..3 {
        let src = color[channel] as f32;
        let dst_value = dst[channel] as f32;
        dst[channel] = (src * src_alpha + dst_value * inv_alpha)
            .round()
            .clamp(0.0, 255.0) as u8;
    }

    let dst_alpha = dst[3] as f32 / 255.0;
    dst[3] = ((src_alpha + dst_alpha * inv_alpha) * 255.0)
        .round()
        .clamp(0.0, 255.0) as u8;
}

fn rgba_to_buffer(image: &RgbaImage) -> MemoryRenderBuffer {
    MemoryRenderBuffer::from_slice(
        image.as_raw(),
        Fourcc::Abgr8888,
        (image.width() as i32, image.height() as i32),
        1,
        Transform::Normal,
        None,
    )
}

fn load_png(path: &Path) -> Option<RgbaImage> {
    match image::open(path) {
        Ok(image) => Some(image.to_rgba8()),
        Err(error) => {
            warn!(path = %path.display(), ?error, "failed to load compositor asset");
            None
        }
    }
}

fn draw_minimize_button(image: &mut RgbaImage, rect: Rectangle<i32, Logical>, color: Rgba<u8>) {
    let y = rect.loc.y + rect.size.h - 5;
    let left = rect.loc.x + 4;
    let right = rect.loc.x + rect.size.w - 4;

    for py in (y - 1).max(0)..(y + 2).min(image.height() as i32) {
        for px in left.max(0)..right.min(image.width() as i32) {
            blend_pixel(image, px as u32, py as u32, color, 1.0);
        }
    }
}

fn draw_maximize_button(
    image: &mut RgbaImage,
    rect: Rectangle<i32, Logical>,
    color: Rgba<u8>,
    restore: bool,
) {
    if restore {
        let back = Rectangle::new(
            (rect.loc.x + 6, rect.loc.y + 4).into(),
            (rect.size.w - 8, rect.size.h - 8).into(),
        );
        draw_outline_rect(image, back, color);
        let front = Rectangle::new(
            (rect.loc.x + 3, rect.loc.y + 7).into(),
            (rect.size.w - 8, rect.size.h - 8).into(),
        );
        draw_outline_rect(image, front, color);
    } else {
        let outline = Rectangle::new(
            (rect.loc.x + 4, rect.loc.y + 4).into(),
            (rect.size.w - 8, rect.size.h - 8).into(),
        );
        draw_outline_rect(image, outline, color);
    }
}

fn draw_outline_rect(image: &mut RgbaImage, rect: Rectangle<i32, Logical>, color: Rgba<u8>) {
    let left = rect.loc.x.max(0);
    let top = rect.loc.y.max(0);
    let right = (rect.loc.x + rect.size.w).min(image.width() as i32);
    let bottom = (rect.loc.y + rect.size.h).min(image.height() as i32);

    for y in top..bottom {
        for x in left..right {
            let is_edge = x <= left + 1 || x >= right - 2 || y <= top + 1 || y >= bottom - 2;
            if is_edge {
                blend_pixel(image, x as u32, y as u32, color, 1.0);
            }
        }
    }
}

fn draw_close_button(image: &mut RgbaImage, rect: Rectangle<i32, Logical>, color: Rgba<u8>) {
    let inset = 4.0;
    let left = rect.loc.x as f32 + inset;
    let top = rect.loc.y as f32 + inset;
    let right = (rect.loc.x + rect.size.w) as f32 - inset;
    let bottom = (rect.loc.y + rect.size.h) as f32 - inset;

    for y in rect.loc.y.max(0)..(rect.loc.y + rect.size.h).min(image.height() as i32) {
        for x in rect.loc.x.max(0)..(rect.loc.x + rect.size.w).min(image.width() as i32) {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let diag1 = distance_to_line(px, py, left, top, right, bottom);
            let diag2 = distance_to_line(px, py, left, bottom, right, top);
            if diag1 <= 1.1 || diag2 <= 1.1 {
                blend_pixel(image, x as u32, y as u32, color, 1.0);
            }
        }
    }
}

fn distance_to_line(px: f32, py: f32, x1: f32, y1: f32, x2: f32, y2: f32) -> f32 {
    let dx = x2 - x1;
    let dy = y2 - y1;
    let length_sq = dx * dx + dy * dy;
    if length_sq <= f32::EPSILON {
        return ((px - x1).powi(2) + (py - y1).powi(2)).sqrt();
    }

    let t = (((px - x1) * dx + (py - y1) * dy) / length_sq).clamp(0.0, 1.0);
    let proj_x = x1 + t * dx;
    let proj_y = y1 + t * dy;
    ((px - proj_x).powi(2) + (py - proj_y).powi(2)).sqrt()
}

fn overlay_image(image: &mut RgbaImage, icon: &RgbaImage, start_x: i32, start_y: i32) {
    for (x, y, pixel) in icon.enumerate_pixels() {
        let px = start_x + x as i32;
        let py = start_y + y as i32;
        if px < 0 || py < 0 || px >= image.width() as i32 || py >= image.height() as i32 {
            continue;
        }

        let alpha = pixel[3] as f32 / 255.0;
        if alpha <= 0.0 {
            continue;
        }
        blend_pixel(image, px as u32, py as u32, *pixel, alpha);
    }
}

fn output_scale(scale: f64) -> OutputScale {
    if (scale.fract()).abs() <= f64::EPSILON {
        OutputScale::Integer(scale.round() as i32)
    } else {
        OutputScale::Fractional(scale)
    }
}

fn load_title_font() -> Option<Font<'static>> {
    const CANDIDATES: &[&str] = &[
        "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
    ];

    for path in CANDIDATES {
        let Ok(bytes) = fs::read(path) else {
            continue;
        };
        match Font::try_from_vec(bytes) {
            Some(font) => {
                info!(font_path = %path, "loaded title font");
                return Some(font);
            }
            None => warn!(font_path = %path, "failed to parse title font"),
        }
    }

    warn!("no system sans-serif font found for title rendering");
    None
}

fn load_app_icon(app_id: &str) -> Option<RgbaImage> {
    const ICON_SIZE: u32 = 20;
    const SIZES: &[u32] = &[256, 192, 128, 96, 64, 48, 32, 24, 22, 16];

    for candidate in app_icon_candidates(app_id) {
        for size in SIZES {
            let path = format!(
                "/usr/share/icons/hicolor/{}x{}/apps/{}.png",
                size, size, candidate
            );
            if let Some(icon) = load_optional_png(Path::new(&path)) {
                return Some(
                    DynamicImage::ImageRgba8(icon)
                        .resize(ICON_SIZE, ICON_SIZE, FilterType::Lanczos3)
                        .to_rgba8(),
                );
            }
        }

        let pixmaps = format!("/usr/share/pixmaps/{}.png", candidate);
        if let Some(icon) = load_optional_png(Path::new(&pixmaps)) {
            return Some(
                DynamicImage::ImageRgba8(icon)
                    .resize(ICON_SIZE, ICON_SIZE, FilterType::Lanczos3)
                    .to_rgba8(),
            );
        }
    }

    None
}

fn load_optional_png(path: &Path) -> Option<RgbaImage> {
    if !path.exists() {
        return None;
    }

    load_png(path)
}

fn app_icon_candidates(app_id: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    let trimmed = app_id.trim();
    if trimmed.is_empty() {
        return candidates;
    }

    for value in [
        trimmed.to_string(),
        trimmed.to_lowercase(),
        trimmed.rsplit('.').next().unwrap_or(trimmed).to_string(),
        trimmed.rsplit('.').next().unwrap_or(trimmed).to_lowercase(),
    ] {
        if !value.is_empty() && !candidates.iter().any(|existing| existing == &value) {
            candidates.push(value);
        }
    }

    candidates
}

fn point_to_physical(point: Point<i32, Logical>) -> Point<f64, Physical> {
    Point::<f64, Physical>::from((point.x as f64, point.y as f64))
}

fn animation_anchor(
    space: &Space<Window>,
    window: &Window,
    frame: Option<&WindowFrame>,
) -> Option<Rectangle<i32, Logical>> {
    if let Some(frame) = frame {
        return Some(frame.frame);
    }

    let loc = space.element_location(window)?;
    let render_loc = Point::<i32, Logical>::from((
        loc.x - window.geometry().loc.x,
        loc.y - window.geometry().loc.y,
    ));
    let bbox = window.bbox();
    Some(Rectangle::new(
        (render_loc.x + bbox.loc.x, render_loc.y + bbox.loc.y).into(),
        bbox.size,
    ))
}

fn animated_surface_elements(
    surfaces: Vec<SpaceRenderElements<GlesRenderer, WaylandSurfaceRenderElement<GlesRenderer>>>,
    anchor: Option<Rectangle<i32, Logical>>,
    animation: WindowAnimation,
) -> Vec<YawcRenderElements> {
    surfaces
        .into_iter()
        .map(|surface| match surface {
            SpaceRenderElements::Surface(surface) => {
                YawcRenderElements::from(AnimatedSurfaceElement::new(surface, anchor, animation))
            }
            surface => YawcRenderElements::from(surface),
        })
        .collect()
}

fn animated_point(
    point: Point<f64, Physical>,
    anchor: Option<Rectangle<i32, Logical>>,
    animation: WindowAnimation,
) -> Point<f64, Physical> {
    let Some(anchor) = anchor else {
        return point;
    };

    if let Some(geometry) = animation.geometry {
        let current = interpolated_rect(geometry.from, geometry.to, geometry.progress);
        let scale_x = current.size.w as f64 / anchor.size.w.max(1) as f64;
        let scale_y = current.size.h as f64 / anchor.size.h.max(1) as f64;

        return Point::from((
            current.loc.x as f64 + (point.x - anchor.loc.x as f64) * scale_x,
            current.loc.y as f64 + (point.y - anchor.loc.y as f64) * scale_y,
        ));
    }

    let center_x = anchor.loc.x as f64 + anchor.size.w as f64 / 2.0;
    let center_y = anchor.loc.y as f64 + anchor.size.h as f64 / 2.0;

    Point::from((
        center_x + (point.x - center_x) * animation.scale,
        center_y + (point.y - center_y) * animation.scale,
    ))
}

fn animated_rect(
    rect: Rectangle<i32, Logical>,
    anchor: Option<Rectangle<i32, Logical>>,
    animation: WindowAnimation,
) -> (Point<f64, Physical>, Size<i32, Logical>) {
    if let (Some(anchor), Some(geometry)) = (anchor, animation.geometry) {
        let current = interpolated_rect(geometry.from, geometry.to, geometry.progress);
        let scale_x = current.size.w as f64 / anchor.size.w.max(1) as f64;
        let scale_y = current.size.h as f64 / anchor.size.h.max(1) as f64;
        let loc = animated_point(point_to_physical(rect.loc), Some(anchor), animation);
        let size = Size::from((
            ((rect.size.w as f64) * scale_x).round().max(1.0) as i32,
            ((rect.size.h as f64) * scale_y).round().max(1.0) as i32,
        ));

        return (loc, size);
    }

    let loc = animated_point(point_to_physical(rect.loc), anchor, animation);
    let size = Size::from((
        ((rect.size.w as f64) * animation.scale).round().max(1.0) as i32,
        ((rect.size.h as f64) * animation.scale).round().max(1.0) as i32,
    ));

    (loc, size)
}

fn animated_physical_rect(
    rect: Rectangle<i32, Physical>,
    anchor: Option<Rectangle<i32, Logical>>,
    animation: WindowAnimation,
) -> Rectangle<i32, Physical> {
    let logical_rect = Rectangle::<i32, Logical>::new(
        (rect.loc.x, rect.loc.y).into(),
        (rect.size.w, rect.size.h).into(),
    );
    let (loc, size) = animated_rect(logical_rect, anchor, animation);
    Rectangle::new(loc.to_i32_round(), Size::from((size.w, size.h)))
}

fn interpolated_rect(
    from: Rectangle<i32, Logical>,
    to: Rectangle<i32, Logical>,
    progress: f64,
) -> Rectangle<i32, Logical> {
    Rectangle::new(
        (
            lerp(from.loc.x as f64, to.loc.x as f64, progress)
                .round()
                .max(i32::MIN as f64)
                .min(i32::MAX as f64) as i32,
            lerp(from.loc.y as f64, to.loc.y as f64, progress)
                .round()
                .max(i32::MIN as f64)
                .min(i32::MAX as f64) as i32,
        )
            .into(),
        (
            lerp(from.size.w as f64, to.size.w as f64, progress)
                .round()
                .max(1.0) as i32,
            lerp(from.size.h as f64, to.size.h as f64, progress)
                .round()
                .max(1.0) as i32,
        )
            .into(),
    )
}

fn lerp(from: f64, to: f64, progress: f64) -> f64 {
    from + (to - from) * progress
}
