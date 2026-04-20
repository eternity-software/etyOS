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
                surface::{WaylandSurfaceRenderElement, WaylandSurfaceTexture},
                texture::{TextureBuffer, TextureRenderElement},
                Element, Id, Kind, RenderElement,
            },
            gles::{
                element::TextureShaderElement, ffi, GlesError, GlesRenderer, GlesTexProgram,
                GlesTexture, Uniform, UniformName, UniformType,
            },
            utils::{CommitCounter, DamageSet, OpaqueRegions},
            Frame, Renderer, Texture,
        },
    },
    desktop::{space::SpaceRenderElements, Space, Window},
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::wayland_server::{protocol::wl_surface::WlSurface, DisplayHandle},
    utils::{Logical, Physical, Point, Rectangle, Scale as RendererScale, Size, Transform},
};
use tracing::{info, warn};

use crate::{
    state::Yawc,
    window::{WindowFrame, BUTTON_PADDING, FRAME_RADIUS, TITLEBAR_HEIGHT},
};

smithay::backend::renderer::element::render_elements! {
    pub YawcRenderElements<=GlesRenderer>;
    Space=SpaceRenderElements<GlesRenderer, WaylandSurfaceRenderElement<GlesRenderer>>,
    Memory=MemoryRenderBufferRenderElement<GlesRenderer>,
    TextureShader=TextureShaderElement,
    TitlebarBlur=TitlebarBlurElement,
    RoundedSurface=RoundedSurfaceElement,
}

const FRAME_FILL_RGBA: [u8; 4] = [92, 96, 102, 150];
const TITLE_COLOR: Rgba<u8> = Rgba([244, 246, 248, 238]);
const CLOSE_COLOR: Rgba<u8> = Rgba([244, 246, 248, 230]);
const TITLE_PADDING: i32 = 18;
const TITLE_FONT_SIZE: f32 = 17.5;
const ICON_SIZE: u32 = 20;
const ICON_GAP: i32 = 10;
const BLUR_PAD_X: i32 = 48;
const BLUR_PAD_Y: i32 = 96;
const TITLEBAR_BLUR_SHADER: &str = r#"
//_DEFINES_

#if defined(EXTERNAL)
#extension GL_OES_EGL_image_external : require
#endif

precision mediump float;
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
varying vec2 v_coords;

float inside_top_round(vec2 p, vec2 size, float r) {
    if (p.x < 0.0 || p.y < 0.0 || p.x >= size.x || p.y >= size.y) {
        return 0.0;
    }
    if (p.y >= r) {
        return 1.0;
    }
    if (p.x >= r && p.x < size.x - r) {
        return 1.0;
    }

    vec2 center = vec2(p.x < r ? r : size.x - r, r);
    vec2 delta = p - center;
    return dot(delta, delta) <= r * r ? 1.0 : 0.0;
}

void main() {
    vec2 local_coords = (v_coords - src_origin) / src_size;
    vec2 pos = local_coords * area_size;
    if (inside_top_round(pos, area_size, radius) < 0.5) {
        gl_FragColor = vec4(0.0);
        return;
    }

    vec2 sample_coords = vec2(v_coords.x, 1.0 - v_coords.y);
    vec4 color = vec4(0.0);
    float total = 0.0;
    const float sigma = 4.0;
    const float blur_step = 3.0;

    for (int ix = -4; ix <= 4; ++ix) {
        for (int iy = -4; iy <= 4; ++iy) {
            vec2 offset = vec2(float(ix), float(iy)) * texel_size * blur_step;
            float dist2 = float(ix * ix + iy * iy);
            float weight = exp(-dist2 / (2.0 * sigma * sigma));
            color += texture2D(tex, sample_coords + offset) * weight;
            total += weight;
        }
    }

    color /= total;

#if defined(NO_ALPHA)
    color = vec4(color.rgb, 1.0) * alpha;
#else
    color = color * alpha;
#endif

    gl_FragColor = color;
}"#;
const CLIENT_CLIP_SHADER: &str = r#"
//_DEFINES_

#if defined(EXTERNAL)
#extension GL_OES_EGL_image_external : require
#endif

precision mediump float;
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

float inside_bottom_round(vec2 p, vec2 size, float r) {
    if (p.x < 0.0 || p.y < 0.0 || p.x >= size.x || p.y >= size.y) {
        return 0.0;
    }
    if (p.y < size.y - r) {
        return 1.0;
    }
    if (p.x >= r && p.x < size.x - r) {
        return 1.0;
    }

    vec2 center = vec2(p.x < r ? r : size.x - r, size.y - r);
    vec2 delta = p - center;
    return dot(delta, delta) <= r * r ? 1.0 : 0.0;
}

void main() {
    vec2 pos = element_offset + v_coords * element_size;
    if (inside_bottom_round(pos, client_size, radius) < 0.5) {
        gl_FragColor = vec4(0.0);
        return;
    }

    vec4 color = texture2D(tex, v_coords);

#if defined(NO_ALPHA)
    color = vec4(color.rgb, 1.0) * alpha;
#else
    color = color * alpha;
#endif

    gl_FragColor = color;
}"#;

pub struct RenderState {
    output: Output,
    titlebar_shader: Option<GlesTexProgram>,
    titlebar_shader_failed: bool,
    client_clip_shader: Option<GlesTexProgram>,
    client_clip_shader_failed: bool,
    title_font: Option<Font<'static>>,
    icon_cache: HashMap<String, Option<RgbaImage>>,
    overlay_cache: HashMap<DecorationCacheKey, MemoryRenderBuffer>,
    blur_texture_cache: HashMap<WlSurface, GlesTexture>,
    wallpaper_source: Option<RgbaImage>,
    wallpaper_image: Option<RgbaImage>,
    wallpaper_buffer: Option<MemoryRenderBuffer>,
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
            "TTY Session",
        )
    }

    fn new_with_output(
        display_handle: &DisplayHandle,
        space: &mut Space<Window>,
        size: Size<i32, Physical>,
        refresh: i32,
        transform: Transform,
        model: &str,
    ) -> Self {
        let mode = Mode {
            size,
            refresh: refresh.max(1),
        };

        let output = Output::new(
            "yawc".to_string(),
            PhysicalProperties {
                size: (0, 0).into(),
                subpixel: Subpixel::Unknown,
                make: "YAWC".into(),
                model: model.into(),
            },
        );
        let _ = output.create_global::<Yawc>(display_handle);

        output.change_current_state(Some(mode), Some(transform), None, Some((0, 0).into()));
        output.set_preferred(mode);
        space.map_output(&output, (0, 0));

        let title_font = load_title_font();
        let wallpaper_source = load_png(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("desktop.png")
                .as_path(),
        );

        let mut state = Self {
            output,
            titlebar_shader: None,
            titlebar_shader_failed: false,
            client_clip_shader: None,
            client_clip_shader_failed: false,
            title_font,
            icon_cache: HashMap::new(),
            overlay_cache: HashMap::new(),
            blur_texture_cache: HashMap::new(),
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
    pub fn wallpaper_buffer(&self) -> Option<&MemoryRenderBuffer> {
        self.wallpaper_buffer.as_ref()
    }

    #[cfg(feature = "tty-udev")]
    pub fn tty_scene_elements(
        &mut self,
        renderer: &mut GlesRenderer,
        space: &Space<Window>,
        frames: &[WindowFrame],
    ) -> Result<Vec<YawcRenderElements>, GlesError> {
        let mut frame_by_surface = HashMap::new();
        for frame in frames {
            if let Some(toplevel) = frame.window.toplevel() {
                frame_by_surface.insert(toplevel.wl_surface().clone(), frame.clone());
            }
        }

        let output_size = self
            .output
            .current_mode()
            .map(|mode| mode.size)
            .unwrap_or_else(|| Size::from((1, 1)));
        let titlebar_shader = self.ensure_titlebar_shader(renderer)?.cloned();
        let mut deco_by_surface = self.decoration_elements(renderer, frames)?;
        let mut elements = Vec::new();

        for window in space.elements().rev() {
            let mut window_elements = Vec::new();
            let frame_meta = window
                .toplevel()
                .and_then(|toplevel| frame_by_surface.get(toplevel.wl_surface()));

            if let Some(toplevel) = window.toplevel() {
                if let Some(deco) = deco_by_surface.remove(toplevel.wl_surface()) {
                    window_elements.extend(deco);
                }

                if let (Some(frame_meta), Some(shader), Some(capture)) = (
                    frame_meta,
                    titlebar_shader.as_ref(),
                    frame_meta.and_then(|frame| blur_capture_for_frame(frame, output_size)),
                ) {
                    let blur_texture = self.ensure_blur_texture(
                        renderer,
                        toplevel.wl_surface(),
                        capture.capture_w,
                        capture.capture_h,
                    )?;
                    window_elements.push(YawcRenderElements::from(TitlebarBlurElement::new(
                        blur_texture,
                        shader.clone(),
                        frame_meta.frame.loc,
                        Size::from((frame_meta.frame.size.w, TITLEBAR_HEIGHT)),
                        capture,
                    )));
                }
            }

            if let Some(loc) = space.element_location(window) {
                let render_loc = Point::<i32, Logical>::from((
                    loc.x - window.geometry().loc.x,
                    loc.y - window.geometry().loc.y,
                ));
                let phys_loc = Point::<i32, Physical>::from((render_loc.x, render_loc.y));
                let surf: Vec<
                    SpaceRenderElements<GlesRenderer, WaylandSurfaceRenderElement<GlesRenderer>>,
                > = window.render_elements(renderer, phys_loc, RendererScale::from(1.0_f64), 1.0);

                if let Some(frame_meta) = frame_meta {
                    window_elements
                        .extend(self.client_surface_elements(renderer, frame_meta, surf)?);
                } else {
                    window_elements.extend(surf.into_iter().map(YawcRenderElements::from));
                }
            }

            elements.extend(window_elements);
        }

        elements.extend(desktop_elements(renderer, self.wallpaper_buffer.as_ref())?);

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
        self.rebuild_desktop_buffers(size);
    }

    #[cfg(feature = "winit-backend")]
    pub fn render_frame(
        &mut self,
        backend: &mut smithay::backend::winit::WinitGraphicsBackend<GlesRenderer>,
        state: &mut Yawc,
        display_handle: &mut DisplayHandle,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let size = backend.window_size();
        let damage = Rectangle::from_size(size);
        let all_frames = state.windows.frames(&state.space);
        let mut frame_by_surface = HashMap::new();
        for frame in &all_frames {
            if let Some(toplevel) = frame.window.toplevel() {
                frame_by_surface.insert(toplevel.wl_surface().clone(), frame.clone());
            }
        }

        let mut deco_by_surface = {
            let renderer = backend.renderer();
            self.decoration_elements(renderer, &all_frames)?
        };
        let mut used_blur_surfaces = HashSet::new();

        {
            let (renderer, mut framebuffer) = backend.bind()?;
            let windows: Vec<Window> = state.space.elements().cloned().collect();
            let titlebar_shader = self.ensure_titlebar_shader(renderer)?.cloned();
            let desktop = desktop_elements(renderer, self.wallpaper_buffer.as_ref())?;
            {
                let mut frame = renderer.render(&mut framebuffer, size, Transform::Flipped180)?;
                frame.clear([0.06, 0.09, 0.11, 1.0].into(), &[damage])?;
                draw_elements(&mut frame, &desktop)?;
                let _ = frame.finish()?;
            }

            for window in &windows {
                let mut step_elements: Vec<YawcRenderElements> = Vec::new();

                if let Some(toplevel) = window.toplevel() {
                    if let Some(frame_meta) = frame_by_surface.get(toplevel.wl_surface()) {
                        let Some(capture) = blur_capture_for_frame(frame_meta, size) else {
                            continue;
                        };
                        used_blur_surfaces.insert(toplevel.wl_surface().clone());
                        let blur_texture = self.ensure_blur_texture(
                            renderer,
                            toplevel.wl_surface(),
                            capture.capture_w,
                            capture.capture_h,
                        )?;
                        let blur_buffer = TextureBuffer::from_texture(
                            renderer,
                            blur_texture.clone(),
                            1,
                            Transform::Normal,
                            None,
                        );
                        let blur_payload = titlebar_shader.as_ref().map(|shader| {
                            (
                                blur_texture,
                                YawcRenderElements::from(TextureShaderElement::new(
                                    TextureRenderElement::from_texture_buffer(
                                        point_to_physical(frame_meta.frame.loc),
                                        &blur_buffer,
                                        None,
                                        Some(Rectangle::new(
                                            (capture.src_x as f64, capture.src_y as f64).into(),
                                            (
                                                frame_meta.frame.size.w as f64,
                                                TITLEBAR_HEIGHT as f64,
                                            )
                                                .into(),
                                        )),
                                        Some((frame_meta.frame.size.w, TITLEBAR_HEIGHT).into()),
                                        Kind::Unspecified,
                                    ),
                                    shader.clone(),
                                    titlebar_shader_uniforms(
                                        frame_meta.frame.size.w,
                                        TITLEBAR_HEIGHT,
                                        capture.capture_w,
                                        capture.capture_h,
                                        capture.src_x,
                                        capture.src_y,
                                    ),
                                )),
                                frame_meta.clone(),
                                capture,
                            )
                        });

                        if let Some(deco) = deco_by_surface.remove(toplevel.wl_surface()) {
                            step_elements.extend(deco);
                        }

                        if let Some(loc) = state.space.element_location(window) {
                            let render_loc = Point::<i32, Logical>::from((
                                loc.x - window.geometry().loc.x,
                                loc.y - window.geometry().loc.y,
                            ));
                            let phys_loc =
                                Point::<i32, Physical>::from((render_loc.x, render_loc.y));
                            let surf: Vec<
                                SpaceRenderElements<
                                    GlesRenderer,
                                    WaylandSurfaceRenderElement<GlesRenderer>,
                                >,
                            > = window.render_elements(
                                renderer,
                                phys_loc,
                                RendererScale::from(1.0_f64),
                                1.0,
                            );
                            step_elements
                                .extend(self.client_surface_elements(renderer, frame_meta, surf)?);
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
                    let phys_loc = Point::<i32, Physical>::from((render_loc.x, render_loc.y));
                    let surf: Vec<
                        SpaceRenderElements<
                            GlesRenderer,
                            WaylandSurfaceRenderElement<GlesRenderer>,
                        >,
                    > = window.render_elements(
                        renderer,
                        phys_loc,
                        RendererScale::from(1.0_f64),
                        1.0,
                    );
                    if let Some(toplevel) = window.toplevel() {
                        if let Some(frame_meta) = frame_by_surface.get(toplevel.wl_surface()) {
                            step_elements
                                .extend(self.client_surface_elements(renderer, frame_meta, surf)?);
                        } else {
                            step_elements.extend(surf.into_iter().map(YawcRenderElements::from));
                        }
                    } else {
                        step_elements.extend(surf.into_iter().map(YawcRenderElements::from));
                    }
                }

                let mut frame = renderer.render(&mut framebuffer, size, Transform::Flipped180)?;
                draw_elements(&mut frame, &step_elements)?;
                let _ = frame.finish()?;
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

        state.space.refresh();
        state.prune_windows();
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
            if texture.size() == Size::from((width, height)) {
                return Ok(texture.clone());
            }
        }

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
    ) -> Result<HashMap<WlSurface, Vec<YawcRenderElements>>, GlesError> {
        let mut result: HashMap<WlSurface, Vec<YawcRenderElements>> = HashMap::new();
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

            // CPU overlay: text, icons, close button (top layer).
            frame_elements.push(YawcRenderElements::from(
                MemoryRenderBufferRenderElement::from_buffer(
                    renderer,
                    point_to_physical(frame.frame.loc),
                    frame_buffer,
                    None,
                    None,
                    None,
                    Kind::Unspecified,
                )?,
            ));

            let Some(surface) = frame
                .window
                .toplevel()
                .map(|toplevel| toplevel.wl_surface().clone())
            else {
                continue;
            };

            result.insert(surface, frame_elements);
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
    fn client_surface_elements(
        &mut self,
        renderer: &mut GlesRenderer,
        frame: &WindowFrame,
        surfaces: Vec<SpaceRenderElements<GlesRenderer, WaylandSurfaceRenderElement<GlesRenderer>>>,
    ) -> Result<Vec<YawcRenderElements>, GlesError> {
        let shader = self.ensure_client_clip_shader(renderer)?.cloned();
        let client_loc =
            point_to_physical((frame.frame.loc.x, frame.frame.loc.y + TITLEBAR_HEIGHT).into());
        let client_size = Size::<i32, Physical>::from((
            frame.frame.size.w,
            (frame.frame.size.h - TITLEBAR_HEIGHT).max(0),
        ));

        let mut elements = Vec::with_capacity(surfaces.len());
        for surface in surfaces {
            match (shader.as_ref(), surface) {
                (Some(shader), SpaceRenderElements::Surface(surface)) if client_size.h > 0 => {
                    elements.push(YawcRenderElements::from(RoundedSurfaceElement::new(
                        surface,
                        shader.clone(),
                        client_loc,
                        client_size,
                    )));
                }
                (_, surface) => elements.push(YawcRenderElements::from(surface)),
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
struct TitlebarBlurElement {
    texture: GlesTexture,
    program: GlesTexProgram,
    id: Id,
    titlebar_loc: Point<i32, Physical>,
    size: Size<i32, Physical>,
    capture: BlurCapture,
}

impl TitlebarBlurElement {
    #[cfg_attr(not(feature = "tty-udev"), allow(dead_code))]
    fn new(
        texture: GlesTexture,
        program: GlesTexProgram,
        loc: Point<i32, Logical>,
        size: Size<i32, Physical>,
        capture: BlurCapture,
    ) -> Self {
        Self {
            texture,
            program,
            id: Id::new(),
            titlebar_loc: Point::<i32, Physical>::from((loc.x, loc.y)),
            size,
            capture,
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
        DamageSet::from_slice(&[Rectangle::new(
            Point::from((
                self.capture.loc.x - self.titlebar_loc.x,
                self.capture.loc.y - self.titlebar_loc.y,
            )),
            self.capture.size(),
        )])
    }

    fn opaque_regions(&self, _scale: RendererScale<f64>) -> OpaqueRegions<i32, Physical> {
        OpaqueRegions::default()
    }

    fn alpha(&self) -> f32 {
        1.0
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
            self.capture.capture_w,
            self.capture.capture_h,
            self.capture.src_x,
            self.capture.src_y,
        );
        frame.render_texture_from_to(
            &self.texture,
            src,
            dst,
            damage,
            &[],
            Transform::Normal,
            1.0,
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
}

impl RoundedSurfaceElement {
    fn new(
        inner: WaylandSurfaceRenderElement<GlesRenderer>,
        program: GlesTexProgram,
        client_loc: Point<f64, Physical>,
        client_size: Size<i32, Physical>,
    ) -> Self {
        Self {
            inner,
            program,
            id: Id::new(),
            client_loc: client_loc.to_i32_round(),
            client_size,
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
        self.inner.geometry(scale)
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
        commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        self.inner.damage_since(scale, commit)
    }

    fn opaque_regions(&self, _scale: RendererScale<f64>) -> OpaqueRegions<i32, Physical> {
        OpaqueRegions::default()
    }

    fn alpha(&self) -> f32 {
        self.inner.alpha()
    }

    fn kind(&self) -> Kind {
        self.inner.kind()
    }

    fn location(&self, scale: RendererScale<f64>) -> Point<i32, Physical> {
        self.inner.location(scale)
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

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct DecorationCacheKey {
    frame_w: i32,
    active: bool,
    title: String,
    app_id: Option<String>,
}

impl DecorationCacheKey {
    fn from_frame(frame: &WindowFrame) -> Self {
        Self {
            frame_w: frame.frame.size.w,
            active: frame.active,
            title: frame.title.clone(),
            app_id: frame.app_id.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct BlurCapture {
    loc: Point<i32, Physical>,
    capture_x: i32,
    capture_y_gl: i32,
    capture_w: i32,
    capture_h: i32,
    src_x: i32,
    src_y: i32,
}

impl BlurCapture {
    fn size(self) -> Size<i32, Physical> {
        Size::from((self.capture_w, self.capture_h))
    }
}

fn blur_capture_for_frame(
    frame: &WindowFrame,
    output_size: Size<i32, Physical>,
) -> Option<BlurCapture> {
    let titlebar_w = frame.frame.size.w.max(1);
    let titlebar_h = TITLEBAR_HEIGHT.max(1);

    let left = (frame.frame.loc.x - BLUR_PAD_X).max(0);
    let top = (frame.frame.loc.y - BLUR_PAD_Y).max(0);
    let right = (frame.frame.loc.x + titlebar_w + BLUR_PAD_X).min(output_size.w);
    let bottom = (frame.frame.loc.y + titlebar_h + BLUR_PAD_Y).min(output_size.h);

    let capture_w = (right - left).max(1);
    let capture_h = (bottom - top).max(1);
    if capture_w <= 0 || capture_h <= 0 {
        return None;
    }

    Some(BlurCapture {
        loc: Point::from((left, top)),
        capture_x: left,
        capture_y_gl: output_size.h - bottom,
        capture_w,
        capture_h,
        src_x: frame.frame.loc.x - left,
        src_y: frame.frame.loc.y - top,
    })
}

fn desktop_elements(
    renderer: &mut GlesRenderer,
    wallpaper: Option<&MemoryRenderBuffer>,
) -> Result<Vec<YawcRenderElements>, GlesError> {
    let mut elements = Vec::new();

    if let Some(wallpaper) = wallpaper {
        elements.push(YawcRenderElements::from(
            MemoryRenderBufferRenderElement::from_buffer(
                renderer,
                Point::<f64, Physical>::from((0.0, 0.0)),
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

fn titlebar_shader_uniforms(
    width: i32,
    height: i32,
    texture_w: i32,
    texture_h: i32,
    src_x: i32,
    src_y: i32,
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
    ]
}

// Overlay with the titlebar tint, text, icon, and close button.
fn overlay_buffer(
    frame: &WindowFrame,
    title_font: Option<&Font<'static>>,
    app_icon: Option<&RgbaImage>,
) -> MemoryRenderBuffer {
    let width = frame.frame.size.w.max(1) as u32;
    let height = frame.frame.size.h.max(1) as u32;
    let mut image = RgbaImage::from_pixel(width, height, Rgba([0, 0, 0, 0]));

    draw_titlebar_fill(&mut image);

    let title = if frame.title.trim().is_empty() {
        "Untitled"
    } else {
        frame.title.as_str()
    };

    if let Some(font) = title_font {
        let close_left = frame.close_button.loc.x - frame.frame.loc.x;
        let content_left = TITLE_PADDING;
        let content_right = (close_left - BUTTON_PADDING).max(content_left);
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

    let close_button = Rectangle::new(
        (
            frame.close_button.loc.x - frame.frame.loc.x,
            frame.close_button.loc.y - frame.frame.loc.y,
        )
            .into(),
        frame.close_button.size,
    );
    draw_close_button(&mut image, close_button, CLOSE_COLOR);

    rgba_to_buffer(&image)
}

// Draw the rounded-top translucent fill that sits above the blur backdrop.
fn draw_titlebar_fill(image: &mut RgbaImage) {
    let width = image.width() as i32;
    let height = image.height() as i32;
    let fill = Rgba(FRAME_FILL_RGBA);

    for y in 0..TITLEBAR_HEIGHT.min(height) {
        for x in 0..width {
            if inside_rounded_rect(x, y, 0, 0, width, height, FRAME_RADIUS) {
                image.put_pixel(x as u32, y as u32, fill);
            }
        }
    }
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

fn inside_rounded_rect(
    x: i32,
    y: i32,
    left: i32,
    top: i32,
    width: i32,
    height: i32,
    radius: i32,
) -> bool {
    if width <= 0 || height <= 0 {
        return false;
    }

    let radius = radius.max(0).min(width / 2).min(height / 2) as f32;
    let px = x as f32 + 0.5;
    let py = y as f32 + 0.5;
    let left = left as f32;
    let top = top as f32;
    let right = (left as i32 + width) as f32;
    let bottom = (top as i32 + height) as f32;

    if px < left || px >= right || py < top || py >= bottom {
        return false;
    }

    if radius <= 0.0 {
        return true;
    }

    if (px >= left + radius && px < right - radius) || (py >= top + radius && py < bottom - radius)
    {
        return true;
    }

    let center_x = if px < left + radius {
        left + radius
    } else {
        right - radius
    };
    let center_y = if py < top + radius {
        top + radius
    } else {
        bottom - radius
    };
    let dx = px - center_x;
    let dy = py - center_y;

    dx * dx + dy * dy <= radius * radius
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
