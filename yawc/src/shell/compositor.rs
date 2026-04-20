use smithay::{
    backend::renderer::utils::on_commit_buffer_handler,
    delegate_compositor, delegate_shm,
    reexports::wayland_server::{
        protocol::{wl_buffer, wl_surface::WlSurface},
        Client,
    },
    wayland::{
        buffer::BufferHandler,
        compositor::{
            get_parent, is_sync_subsurface, CompositorClientState, CompositorHandler,
            CompositorState,
        },
        shm::{ShmHandler, ShmState},
    },
};

use crate::{
    grabs::resize_grab,
    state::{ClientState, Yawc},
};

use super::xdg;

impl CompositorHandler for Yawc {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        &client.get_data::<ClientState>().unwrap().compositor_state
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);

        if !is_sync_subsurface(surface) {
            let mut root = surface.clone();
            while let Some(parent) = get_parent(&root) {
                root = parent;
            }

            let committed_window = self
                .space
                .elements()
                .find(|window| window.toplevel().unwrap().wl_surface() == &root)
                .cloned();

            if let Some(window) = committed_window {
                window.on_commit();
                let bbox = window.bbox();
                if bbox.size.w > 0 && bbox.size.h > 0 {
                    self.position_new_window_if_needed(&window, &root);
                    self.windows.start_map_animation_if_needed(&root);
                }
            }
        }

        xdg::handle_commit(&mut self.popups, &self.space, surface);
        resize_grab::handle_commit(&mut self.space, surface);
    }
}

impl BufferHandler for Yawc {
    fn buffer_destroyed(&mut self, _buffer: &wl_buffer::WlBuffer) {}
}

impl ShmHandler for Yawc {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

delegate_compositor!(Yawc);
delegate_shm!(Yawc);
