use image::load_from_memory;
use smithay::{
    backend::{
        renderer::gles::GlesRenderer,
        winit::{self, WinitEvent},
    },
    reexports::{calloop::EventLoop, winit as host_winit},
};
use tracing::{error, info};

use crate::{render::RenderState, CalloopData};

pub fn init(
    event_loop: &mut EventLoop<CalloopData>,
    data: &mut CalloopData,
) -> Result<(), Box<dyn std::error::Error>> {
    let attributes = host_winit::window::Window::default_attributes()
        .with_title("YAWC")
        .with_inner_size(host_winit::dpi::LogicalSize::new(1280.0, 800.0))
        .with_visible(true)
        .with_decorations(false)
        .with_transparent(false);

    let (mut backend, winit_loop) = winit::init_from_attributes::<GlesRenderer>(attributes)?;
    let size = backend.window_size();
    let mut render_state = RenderState::new(&data.display_handle, &mut data.state.space, size);
    backend.window().set_title("YAWC");
    backend.window().set_window_icon(load_window_icon());

    std::env::set_var("WAYLAND_DISPLAY", &data.state.socket_name);
    info!(
        display = %data.state.socket_name.to_string_lossy(),
        "exported WAYLAND_DISPLAY for nested clients"
    );

    backend.window().request_redraw();

    event_loop
        .handle()
        .insert_source(winit_loop, move |event, _, data| {
            let display_handle = &mut data.display_handle;
            let state = &mut data.state;

            match event {
                WinitEvent::Resized { size, .. } => {
                    render_state.resize(size);
                    backend.window().request_redraw();
                }
                WinitEvent::Input(event) => {
                    state.process_input_event(event);
                    backend.window().set_cursor(state.pending_cursor.to_winit());
                }
                WinitEvent::Redraw => {
                    if let Err(error) =
                        render_state.render_frame(&mut backend, state, display_handle)
                    {
                        error!(?error, "render pass failed");
                        state.loop_signal.stop();
                        return;
                    }

                    backend.window().request_redraw();
                }
                WinitEvent::CloseRequested => state.loop_signal.stop(),
                _ => {}
            }
        })?;

    Ok(())
}

fn load_window_icon() -> Option<host_winit::window::Icon> {
    let image = load_from_memory(include_bytes!("../../yawc_logo.png"))
        .ok()?
        .to_rgba8();
    let (width, height) = image.dimensions();

    host_winit::window::Icon::from_rgba(image.into_raw(), width, height).ok()
}
