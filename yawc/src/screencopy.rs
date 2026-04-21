use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use smithay::{
    backend::allocator::Fourcc,
    output::Output,
    reexports::{
        wayland_protocols_wlr::screencopy::v1::server::{
            zwlr_screencopy_frame_v1::{self, ZwlrScreencopyFrameV1},
            zwlr_screencopy_manager_v1::{self, ZwlrScreencopyManagerV1},
        },
        wayland_server::{
            backend::GlobalId,
            protocol::{wl_buffer::WlBuffer, wl_output::WlOutput, wl_shm},
            Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
        },
    },
    utils::{Logical, Rectangle, Size},
    wayland::shm::{with_buffer_contents_mut, BufferData},
};
use tracing::{debug, warn};

use crate::state::Yawc;

pub struct ScreencopyState {
    _global: GlobalId,
    pending: Vec<PendingScreencopy>,
}

#[derive(Clone, Debug)]
pub struct PendingScreencopy {
    frame: ZwlrScreencopyFrameV1,
    buffer: WlBuffer,
    region: Rectangle<i32, Logical>,
    with_damage: bool,
}

#[derive(Clone, Debug)]
pub struct CapturedFrame {
    pub size: Size<i32, Logical>,
    pub stride: i32,
    pub data: Vec<u8>,
}

struct ScreencopyFrameData {
    region: Rectangle<i32, Logical>,
    used: AtomicBool,
}

impl ScreencopyState {
    pub fn new(display: &DisplayHandle) -> Self {
        Self {
            _global: display.create_global::<Yawc, ZwlrScreencopyManagerV1, _>(3, ()),
            pending: Vec::new(),
        }
    }

    pub fn push_pending(&mut self, request: PendingScreencopy) {
        self.pending.push(request);
    }

    pub fn take_pending(&mut self) -> Vec<PendingScreencopy> {
        std::mem::take(&mut self.pending)
    }

    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }
}

impl PendingScreencopy {
    pub fn region(&self) -> Rectangle<i32, Logical> {
        self.region
    }

    pub fn finish(self, captured: Result<CapturedFrame, String>) {
        match captured {
            Ok(captured) => {
                if let Err(message) = copy_frame_to_buffer(&self.buffer, &captured) {
                    warn!(
                        message,
                        "failed to copy screencopy frame into client buffer"
                    );
                    self.frame.failed();
                    return;
                }

                if self.with_damage {
                    self.frame.damage(
                        0,
                        0,
                        captured.size.w.max(0) as u32,
                        captured.size.h.max(0) as u32,
                    );
                }

                self.frame.flags(zwlr_screencopy_frame_v1::Flags::empty());
                let (sec_hi, sec_lo, nsec) = now_timestamp();
                self.frame.ready(sec_hi, sec_lo, nsec);
                self.buffer.release();
                debug!(
                    width = captured.size.w,
                    height = captured.size.h,
                    stride = captured.stride,
                    "completed screencopy frame"
                );
            }
            Err(message) => {
                warn!(message, "failed to capture screencopy frame");
                self.frame.failed();
            }
        }
    }
}

impl GlobalDispatch<ZwlrScreencopyManagerV1, ()> for Yawc {
    fn bind(
        _state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<ZwlrScreencopyManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        debug!("client bound wlr screencopy manager");
        data_init.init(resource, ());
    }
}

impl Dispatch<ZwlrScreencopyManagerV1, ()> for Yawc {
    fn request(
        state: &mut Self,
        _client: &Client,
        _resource: &ZwlrScreencopyManagerV1,
        request: zwlr_screencopy_manager_v1::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            zwlr_screencopy_manager_v1::Request::CaptureOutput { frame, output, .. } => {
                let Some(region) = state.screencopy_region_for_output(&output, None) else {
                    warn!("screencopy capture_output requested an unknown output");
                    let frame = data_init.init(
                        frame,
                        ScreencopyFrameData {
                            region: Rectangle::from_size((1, 1).into()),
                            used: AtomicBool::new(true),
                        },
                    );
                    frame.failed();
                    return;
                };
                debug!(
                    x = region.loc.x,
                    y = region.loc.y,
                    width = region.size.w,
                    height = region.size.h,
                    "screencopy capture_output requested"
                );
                init_frame(frame, region, data_init);
            }
            zwlr_screencopy_manager_v1::Request::CaptureOutputRegion {
                frame,
                output,
                x,
                y,
                width,
                height,
                ..
            } => {
                let requested = Rectangle::new((x, y).into(), (width, height).into());
                let Some(region) = state.screencopy_region_for_output(&output, Some(requested))
                else {
                    warn!(
                        x,
                        y,
                        width,
                        height,
                        "screencopy capture_output_region requested an unknown or empty output region"
                    );
                    let frame = data_init.init(
                        frame,
                        ScreencopyFrameData {
                            region: Rectangle::from_size((1, 1).into()),
                            used: AtomicBool::new(true),
                        },
                    );
                    frame.failed();
                    return;
                };
                debug!(
                    x = region.loc.x,
                    y = region.loc.y,
                    width = region.size.w,
                    height = region.size.h,
                    "screencopy capture_output_region requested"
                );
                init_frame(frame, region, data_init);
            }
            zwlr_screencopy_manager_v1::Request::Destroy => {}
            _ => {}
        }
    }
}

impl Dispatch<ZwlrScreencopyFrameV1, ScreencopyFrameData> for Yawc {
    fn request(
        state: &mut Self,
        _client: &Client,
        frame: &ZwlrScreencopyFrameV1,
        request: zwlr_screencopy_frame_v1::Request,
        data: &ScreencopyFrameData,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            zwlr_screencopy_frame_v1::Request::Copy { buffer } => {
                queue_copy(state, frame, data, buffer, false);
            }
            zwlr_screencopy_frame_v1::Request::CopyWithDamage { buffer } => {
                queue_copy(state, frame, data, buffer, true);
            }
            zwlr_screencopy_frame_v1::Request::Destroy => {}
            _ => {}
        }
    }
}

impl Yawc {
    fn screencopy_region_for_output(
        &self,
        requested_output: &WlOutput,
        requested_region: Option<Rectangle<i32, Logical>>,
    ) -> Option<Rectangle<i32, Logical>> {
        let output = self.output_for_screencopy(requested_output)?;
        let geometry = self.space.output_geometry(output)?;
        let full = Rectangle::new((0, 0).into(), geometry.size);

        let region = requested_region.unwrap_or(full);
        clip_rect(region, full)
    }

    fn output_for_screencopy(&self, requested_output: &WlOutput) -> Option<&Output> {
        self.space
            .outputs()
            .find(|output| output.owns(requested_output))
            .or_else(|| self.space.outputs().next())
    }
}

fn init_frame(
    frame: New<ZwlrScreencopyFrameV1>,
    region: Rectangle<i32, Logical>,
    data_init: &mut DataInit<'_, Yawc>,
) {
    let width = region.size.w.max(1) as u32;
    let height = region.size.h.max(1) as u32;
    let stride = width * 4;
    let frame = data_init.init(
        frame,
        ScreencopyFrameData {
            region,
            used: AtomicBool::new(false),
        },
    );

    frame.buffer(wl_shm::Format::Xrgb8888, width, height, stride);
    if frame.version() >= 3 {
        // xdg-desktop-portal-wlr validates that v3 screencopy exposes a dmabuf
        // format before it starts PipeWire, but it can still fall back to the
        // wl_shm buffer path when no linux-dmabuf global/modifiers are present.
        frame.linux_dmabuf(Fourcc::Xrgb8888 as u32, width, height);
        frame.buffer_done();
    }
}

fn queue_copy(
    state: &mut Yawc,
    frame: &ZwlrScreencopyFrameV1,
    data: &ScreencopyFrameData,
    buffer: WlBuffer,
    with_damage: bool,
) {
    if data.used.swap(true, Ordering::AcqRel) {
        frame.post_error(
            zwlr_screencopy_frame_v1::Error::AlreadyUsed,
            "screencopy frame already used",
        );
        return;
    }

    state.screencopy_state.push_pending(PendingScreencopy {
        frame: frame.clone(),
        buffer,
        region: data.region,
        with_damage,
    });
    debug!(
        x = data.region.loc.x,
        y = data.region.loc.y,
        width = data.region.size.w,
        height = data.region.size.h,
        with_damage,
        "queued screencopy frame"
    );
}

fn copy_frame_to_buffer(buffer: &WlBuffer, captured: &CapturedFrame) -> Result<(), String> {
    with_buffer_contents_mut(buffer, |ptr, len, metadata| {
        copy_frame_to_shm(ptr, len, metadata, captured)
    })
    .map_err(|error| format!("buffer is not writable wl_shm: {error:?}"))?
}

fn copy_frame_to_shm(
    ptr: *mut u8,
    len: usize,
    metadata: BufferData,
    captured: &CapturedFrame,
) -> Result<(), String> {
    if metadata.width != captured.size.w || metadata.height != captured.size.h {
        return Err(format!(
            "buffer size mismatch: client={}x{}, capture={}x{}",
            metadata.width, metadata.height, captured.size.w, captured.size.h
        ));
    }
    if metadata.format != wl_shm::Format::Xrgb8888 && metadata.format != wl_shm::Format::Argb8888 {
        return Err(format!("unsupported buffer format: {:?}", metadata.format));
    }
    if metadata.stride < captured.stride {
        return Err(format!(
            "buffer stride too small: client={}, capture={}",
            metadata.stride, captured.stride
        ));
    }

    let offset = metadata.offset.max(0) as usize;
    let stride = metadata.stride.max(0) as usize;
    let width_bytes = (captured.size.w.max(0) * 4) as usize;
    let height = captured.size.h.max(0) as usize;
    let required = offset.saturating_add(stride.saturating_mul(height.saturating_sub(1)));
    if required.saturating_add(width_bytes) > len {
        return Err("client buffer is smaller than advertised metadata".to_string());
    }

    let dst = unsafe { std::slice::from_raw_parts_mut(ptr, len) };
    for row in 0..height {
        let src_start = row * captured.stride as usize;
        let src_end = src_start + width_bytes;
        let dst_start = offset + row * stride;
        let dst_end = dst_start + width_bytes;
        dst[dst_start..dst_end].copy_from_slice(&captured.data[src_start..src_end]);
    }

    Ok(())
}

fn clip_rect(
    rect: Rectangle<i32, Logical>,
    bounds: Rectangle<i32, Logical>,
) -> Option<Rectangle<i32, Logical>> {
    let left = rect.loc.x.max(bounds.loc.x);
    let top = rect.loc.y.max(bounds.loc.y);
    let right = (rect.loc.x + rect.size.w).min(bounds.loc.x + bounds.size.w);
    let bottom = (rect.loc.y + rect.size.h).min(bounds.loc.y + bounds.size.h);
    let width = right - left;
    let height = bottom - top;
    (width > 0 && height > 0).then(|| Rectangle::new((left, top).into(), (width, height).into()))
}

fn now_timestamp() -> (u32, u32, u32) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();
    ((secs >> 32) as u32, secs as u32, now.subsec_nanos())
}
