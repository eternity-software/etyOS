mod backend;
mod grabs;
mod input;
mod render;
mod shell;
mod state;
mod window;

use std::process::Command;

use smithay::reexports::{
    calloop::EventLoop,
    wayland_server::{Display, DisplayHandle},
};
use tracing::info;
use tracing_subscriber::EnvFilter;

use crate::state::Yawc;

pub struct CalloopData {
    pub state: Yawc,
    pub display_handle: DisplayHandle,
}

fn init_tracing() {
    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,yawc=debug"));

    tracing_subscriber::fmt().with_env_filter(env_filter).init();
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_tracing();

    let mut event_loop: EventLoop<CalloopData> = EventLoop::try_new()?;
    let display: Display<Yawc> = Display::new()?;
    let display_handle = display.handle();
    let state = Yawc::new(&mut event_loop, display);

    let mut data = CalloopData {
        state,
        display_handle,
    };

    backend::winit::init(&mut event_loop, &mut data)?;

    if let Some(command) = parse_command() {
        info!(command = %command, "spawning startup client");
        Command::new(command).spawn().ok();
    }

    event_loop.run(None, &mut data, |_| {})?;
    Ok(())
}

fn parse_command() -> Option<String> {
    let mut args = std::env::args().skip(1);

    match (args.next().as_deref(), args.next()) {
        (Some("-c") | Some("--command"), Some(command)) => Some(command),
        _ => None,
    }
}
