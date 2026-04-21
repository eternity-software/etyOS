mod backend;
mod config;
mod cursor;
mod grabs;
mod input;
mod render;
mod screencopy;
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
use tracing::{info, warn};
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

    ensure_session_environment_defaults();

    match cli.backend {
        #[cfg(feature = "winit-backend")]
        BackendKind::Winit => backend::winit::init(&mut event_loop, &mut data)?,
        #[cfg(feature = "tty-udev")]
        BackendKind::Standalone => backend::tty_udev::init(&mut event_loop, &mut data)?,
    }

    update_activation_environment();

    let startup_command = cli.command.or_else(|| default_startup_command(cli.backend));

    if let Some(command) = startup_command {
        spawn_startup_client(command, cli.backend);
    }

    event_loop.run(None, &mut data, |_| {})?;
    Ok(())
}

fn ensure_session_environment_defaults() {
    set_env_default("XDG_SESSION_TYPE", "wayland");
    set_env_default("XDG_CURRENT_DESKTOP", "etyDE:YAWC");
    set_env_default("XDG_SESSION_DESKTOP", "yawc");
    set_env_default("DESKTOP_SESSION", "yawc");
    set_env_default("GDK_BACKEND", "wayland");
    set_env_default("QT_QPA_PLATFORM", "wayland");
    set_env_default("SDL_VIDEODRIVER", "wayland");
    set_env_default("CLUTTER_BACKEND", "wayland");
    set_env_default("MOZ_ENABLE_WAYLAND", "1");
}

fn set_env_default(key: &str, value: &str) {
    if std::env::var_os(key).is_none() {
        std::env::set_var(key, value);
    }
}

fn update_activation_environment() {
    const ENV_NAMES: &[&str] = &[
        "WAYLAND_DISPLAY",
        "XDG_SESSION_TYPE",
        "XDG_CURRENT_DESKTOP",
        "XDG_SESSION_DESKTOP",
        "DESKTOP_SESSION",
        "XDG_DESKTOP_PORTAL_DIR",
        "XDG_DATA_DIRS",
        "XDG_DATA_HOME",
        "GDK_BACKEND",
        "QT_QPA_PLATFORM",
        "SDL_VIDEODRIVER",
        "CLUTTER_BACKEND",
        "MOZ_ENABLE_WAYLAND",
    ];

    let active_names: Vec<&str> = ENV_NAMES
        .iter()
        .copied()
        .filter(|name| std::env::var_os(name).is_some())
        .collect();
    if active_names.is_empty() {
        return;
    }

    let mut dbus = Command::new("dbus-update-activation-environment");
    dbus.arg("--systemd");
    dbus.args(&active_names);
    run_activation_command(dbus, "dbus-update-activation-environment");

    let mut systemctl = Command::new("systemctl");
    systemctl.arg("--user").arg("import-environment");
    systemctl.args(&active_names);
    run_activation_command(systemctl, "systemctl --user import-environment");
}

fn run_activation_command(mut command: Command, label: &str) {
    match command.status() {
        Ok(status) if status.success() => {}
        Ok(status) => {
            warn!(
                command = label,
                code = status.code(),
                "activation environment update command failed"
            );
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            warn!(
                ?error,
                command = label,
                "failed to run activation environment update command"
            );
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum BackendKind {
    #[cfg(feature = "winit-backend")]
    Winit,
    #[cfg(feature = "tty-udev")]
    Standalone,
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
            "--standalone" | "--tty-udev" => backend = BackendKind::Standalone,
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
        return Ok(BackendKind::Standalone);
    }

    #[allow(unreachable_code)]
    Err("no backend compiled in".to_string())
}

#[cfg(feature = "tty-udev")]
fn default_startup_command(backend: BackendKind) -> Option<String> {
    if matches!(backend, BackendKind::Standalone) {
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
    if matches!(_backend, BackendKind::Standalone) {
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
    let mut lines = vec!["USAGE: yawc [--winit|--standalone] [--command CMD]".to_string()];

    #[cfg(feature = "winit-backend")]
    lines.push("  --winit       Run YAWC as a nested compositor window.".to_string());
    #[cfg(feature = "tty-udev")]
    lines.push("  --standalone  Run YAWC as a standalone hardware session.".to_string());
    #[cfg(feature = "tty-udev")]
    lines.push("  --tty-udev    Deprecated alias for --standalone.".to_string());
    lines.push("  --command CMD Spawn a client after backend initialization.".to_string());

    lines.join("\n")
}
