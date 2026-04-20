mod backend;
mod cursor;
mod grabs;
mod input;
mod render;
mod shell;
mod state;
mod window;

#[cfg(feature = "tty-udev")]
use std::path::PathBuf;
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

    let cli = parse_cli().map_err(|message| {
        eprintln!("{message}");
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid command line")
    })?;

    let mut event_loop: EventLoop<CalloopData> = EventLoop::try_new()?;
    let display: Display<Yawc> = Display::new()?;
    let display_handle = display.handle();
    let state = Yawc::new(&mut event_loop, display);

    let mut data = CalloopData {
        state,
        display_handle,
    };

    match cli.backend {
        #[cfg(feature = "winit-backend")]
        BackendKind::Winit => backend::winit::init(&mut event_loop, &mut data)?,
        #[cfg(feature = "tty-udev")]
        BackendKind::TtyUdev => backend::tty_udev::init(&mut event_loop, &mut data)?,
    }

    let startup_command = cli.command.or_else(|| default_startup_command(cli.backend));

    if let Some(command) = startup_command {
        spawn_startup_client(command, cli.backend);
    }

    event_loop.run(None, &mut data, |_| {})?;
    Ok(())
}

#[derive(Clone, Copy, Debug)]
enum BackendKind {
    #[cfg(feature = "winit-backend")]
    Winit,
    #[cfg(feature = "tty-udev")]
    TtyUdev,
}

#[derive(Debug)]
struct Cli {
    backend: BackendKind,
    command: Option<String>,
}

fn parse_cli() -> Result<Cli, String> {
    let mut args = std::env::args().skip(1);
    let mut backend = default_backend()?;
    let mut command = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            #[cfg(feature = "winit-backend")]
            "--winit" => backend = BackendKind::Winit,
            #[cfg(feature = "tty-udev")]
            "--tty-udev" => backend = BackendKind::TtyUdev,
            "-c" | "--command" => {
                let value = args
                    .next()
                    .ok_or_else(|| "missing value for --command".to_string())?;
                command = Some(value);
            }
            "--help" | "-h" => return Err(usage()),
            other => return Err(format!("unknown argument: {other}\n\n{}", usage())),
        }
    }

    Ok(Cli { backend, command })
}

fn default_backend() -> Result<BackendKind, String> {
    #[cfg(feature = "winit-backend")]
    {
        return Ok(BackendKind::Winit);
    }

    #[cfg(all(not(feature = "winit-backend"), feature = "tty-udev"))]
    {
        return Ok(BackendKind::TtyUdev);
    }

    #[allow(unreachable_code)]
    Err("no backend compiled in".to_string())
}

#[cfg(feature = "tty-udev")]
fn default_startup_command(backend: BackendKind) -> Option<String> {
    if matches!(backend, BackendKind::TtyUdev) {
        for candidate in [
            "foot",
            "weston-terminal",
            "kgx",
            "alacritty",
            "wezterm",
            "kitty",
            "konsole",
            "gnome-terminal",
            "xfce4-terminal",
            "qterminal",
        ] {
            if let Some(path) = command_path(candidate) {
                return Some(path.to_string_lossy().into_owned());
            }
        }
    }

    None
}

#[cfg(not(feature = "tty-udev"))]
fn default_startup_command(_backend: BackendKind) -> Option<String> {
    None
}

#[cfg(feature = "tty-udev")]
fn command_path(command: &str) -> Option<PathBuf> {
    let path_var =
        std::env::var_os("PATH").unwrap_or_else(|| "/usr/local/bin:/usr/bin:/bin".into());

    std::env::split_paths(&path_var).find_map(|dir| {
        let mut path = PathBuf::from(dir);
        path.push(command);
        path.is_file().then_some(path)
    })
}

fn spawn_startup_client(command: String, _backend: BackendKind) {
    info!(command = %command, "spawning startup client");
    let mut child = Command::new("sh");
    child.arg("-lc").arg(&command);
    #[cfg(feature = "tty-udev")]
    if matches!(_backend, BackendKind::TtyUdev) {
        child
            .env("XDG_SESSION_TYPE", "wayland")
            .env("GDK_BACKEND", "wayland")
            .env("QT_QPA_PLATFORM", "wayland")
            .env("SDL_VIDEODRIVER", "wayland")
            .env("CLUTTER_BACKEND", "wayland")
            .env("MOZ_ENABLE_WAYLAND", "1");
    }

    if let Err(error) = child.spawn() {
        tracing::warn!(?error, command = %command, "failed to spawn startup client");
    }
}

fn usage() -> String {
    let mut lines = vec!["USAGE: yawc [--winit|--tty-udev] [--command CMD]".to_string()];

    #[cfg(feature = "winit-backend")]
    lines.push("  --winit       Run YAWC as a nested compositor window.".to_string());
    #[cfg(feature = "tty-udev")]
    lines.push("  --tty-udev    Run YAWC with the experimental tty/udev backend.".to_string());
    lines.push("  --command CMD Spawn a client after backend initialization.".to_string());

    lines.join("\n")
}
