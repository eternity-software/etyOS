use std::{
    fs,
    path::PathBuf,
    time::{Duration, Instant, SystemTime},
};

use smithay::input::keyboard::{keysyms, ModifiersState};
use tracing::{info, warn};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HotkeyAction {
    ToggleMaximize,
    SnapLeft,
    SnapRight,
    ToggleFullscreen,
    ToggleMinimize,
}

#[derive(Clone, Debug)]
pub struct Config {
    path: PathBuf,
    modified: Option<SystemTime>,
    last_checked: Instant,
    hotkeys: Hotkeys,
    animations: AnimationConfig,
}

#[derive(Clone, Debug)]
pub struct Hotkeys {
    maximize: Option<KeyBinding>,
    snap_left: Option<KeyBinding>,
    snap_right: Option<KeyBinding>,
    fullscreen: Option<KeyBinding>,
    minimize: Option<KeyBinding>,
}

#[derive(Clone, Copy, Debug)]
struct KeyBinding {
    ctrl: bool,
    alt: bool,
    shift: bool,
    logo: bool,
    key: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnimationConfig {
    pub enabled: bool,
    pub popup_ms: u64,
    pub geometry_ms: u64,
}

impl Default for AnimationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            popup_ms: 180,
            geometry_ms: 220,
        }
    }
}

impl Default for Hotkeys {
    fn default() -> Self {
        Self {
            maximize: parse_binding("Super+Up"),
            snap_left: parse_binding("Super+Left"),
            snap_right: parse_binding("Super+Right"),
            fullscreen: parse_binding("Ctrl+Alt+F"),
            minimize: parse_binding("Ctrl+Alt+M"),
        }
    }
}

impl Config {
    pub fn load_or_create() -> Self {
        let path = config_path();
        if !path.exists() {
            if let Some(parent) = path.parent() {
                if let Err(error) = fs::create_dir_all(parent) {
                    warn!(?error, path = %path.display(), "failed to create config directory");
                }
            }
            if let Err(error) = fs::write(&path, DEFAULT_CONFIG) {
                warn!(?error, path = %path.display(), "failed to write default config");
            }
        }

        let mut config = Self {
            path,
            modified: None,
            last_checked: Instant::now() - Duration::from_secs(1),
            hotkeys: Hotkeys::default(),
            animations: AnimationConfig::default(),
        };
        config.reload();
        config
    }

    pub fn reload_if_changed(&mut self) {
        if self.last_checked.elapsed() < Duration::from_millis(100) {
            return;
        }
        self.last_checked = Instant::now();

        let modified = fs::metadata(&self.path)
            .and_then(|metadata| metadata.modified())
            .ok();
        if modified != self.modified {
            self.reload();
        }
    }

    pub fn hotkey_action(&self, key: u32, modifiers: ModifiersState) -> Option<HotkeyAction> {
        self.hotkeys.action_for(key, modifiers)
    }

    pub fn animations(&self) -> AnimationConfig {
        self.animations
    }

    fn reload(&mut self) {
        let contents = match fs::read_to_string(&self.path) {
            Ok(contents) => contents,
            Err(error) => {
                warn!(
                    ?error,
                    path = %self.path.display(),
                    "failed to read config; keeping current config"
                );
                return;
            }
        };

        let parsed = parse_config(&contents);
        self.hotkeys = parsed.hotkeys;
        self.animations = parsed.animations;
        self.modified = fs::metadata(&self.path)
            .and_then(|metadata| metadata.modified())
            .ok();
        info!(path = %self.path.display(), "loaded YAWC config");
    }
}

#[derive(Clone, Debug)]
struct ParsedConfig {
    hotkeys: Hotkeys,
    animations: AnimationConfig,
}

impl Hotkeys {
    fn action_for(&self, key: u32, modifiers: ModifiersState) -> Option<HotkeyAction> {
        [
            (HotkeyAction::ToggleMaximize, self.maximize),
            (HotkeyAction::SnapLeft, self.snap_left),
            (HotkeyAction::SnapRight, self.snap_right),
            (HotkeyAction::ToggleFullscreen, self.fullscreen),
            (HotkeyAction::ToggleMinimize, self.minimize),
        ]
        .into_iter()
        .find_map(|(action, binding)| {
            binding
                .filter(|binding| binding.matches(key, modifiers))
                .map(|_| action)
        })
    }
}

impl KeyBinding {
    fn matches(self, key: u32, modifiers: ModifiersState) -> bool {
        self.key == key
            && self.ctrl == modifiers.ctrl
            && self.alt == modifiers.alt
            && self.shift == modifiers.shift
            && self.logo == modifiers.logo
    }
}

fn parse_config(contents: &str) -> ParsedConfig {
    let mut hotkeys = Hotkeys::default();
    let mut animations = AnimationConfig::default();

    for (line_number, raw_line) in contents.lines().enumerate() {
        let line = raw_line
            .split_once('#')
            .map(|(line, _)| line)
            .unwrap_or(raw_line)
            .trim();
        if line.is_empty() {
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            warn!(line = line_number + 1, "ignoring malformed config line");
            continue;
        };
        let key = key.trim();
        let value = value.trim().trim_matches('"').trim_matches('\'');

        match normalize_name(key).as_str() {
            "hotkeymaximize" | "maximize" => hotkeys.maximize = parse_binding(value),
            "hotkeysnapleft" | "snapleft" => hotkeys.snap_left = parse_binding(value),
            "hotkeysnapright" | "snapright" => hotkeys.snap_right = parse_binding(value),
            "hotkeyfullscreen" | "fullscreen" => hotkeys.fullscreen = parse_binding(value),
            "hotkeyminimize" | "minimize" => hotkeys.minimize = parse_binding(value),
            "animations" | "animationsenabled" | "animationenabled" => {
                if let Some(enabled) = parse_bool(value) {
                    animations.enabled = enabled;
                } else {
                    warn!(
                        line = line_number + 1,
                        value, "ignoring invalid animation boolean"
                    );
                }
            }
            "animationms" | "animationduration" | "animationdurationms" => {
                if let Some(duration) = parse_duration_ms(value) {
                    animations.popup_ms = duration;
                    animations.geometry_ms = duration;
                } else {
                    warn!(
                        line = line_number + 1,
                        value, "ignoring invalid animation duration"
                    );
                }
            }
            "popupanimationms" | "popupduration" | "popupdurationms" => {
                if let Some(duration) = parse_duration_ms(value) {
                    animations.popup_ms = duration;
                } else {
                    warn!(
                        line = line_number + 1,
                        value, "ignoring invalid popup duration"
                    );
                }
            }
            "geometryanimationms" | "geometryduration" | "geometrydurationms" => {
                if let Some(duration) = parse_duration_ms(value) {
                    animations.geometry_ms = duration;
                } else {
                    warn!(
                        line = line_number + 1,
                        value, "ignoring invalid geometry duration"
                    );
                }
            }
            _ => warn!(line = line_number + 1, key, "ignoring unknown config key"),
        }
    }

    ParsedConfig {
        hotkeys,
        animations,
    }
}

fn parse_bool(value: &str) -> Option<bool> {
    match normalize_name(value).as_str() {
        "true" | "yes" | "on" | "enabled" | "enable" | "1" => Some(true),
        "false" | "no" | "off" | "disabled" | "disable" | "0" => Some(false),
        _ => None,
    }
}

fn parse_duration_ms(value: &str) -> Option<u64> {
    let trimmed = value.trim();
    let without_suffix = trimmed
        .strip_suffix("ms")
        .or_else(|| trimmed.strip_suffix("MS"))
        .unwrap_or(trimmed)
        .trim();
    without_suffix.parse::<u64>().ok()
}

fn parse_binding(value: &str) -> Option<KeyBinding> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("none")
        || value.eq_ignore_ascii_case("disabled")
        || value.eq_ignore_ascii_case("off")
    {
        return None;
    }

    let mut binding = KeyBinding {
        ctrl: false,
        alt: false,
        shift: false,
        logo: false,
        key: 0,
    };

    for part in value.split('+') {
        let normalized = normalize_name(part);
        match normalized.as_str() {
            "ctrl" | "control" => binding.ctrl = true,
            "alt" | "meta" => binding.alt = true,
            "shift" => binding.shift = true,
            "super" | "logo" | "mod4" | "win" | "windows" => binding.logo = true,
            _ => {
                let Some(key) = key_name_to_raw(&normalized) else {
                    warn!(binding = value, part, "ignoring invalid hotkey binding");
                    return None;
                };
                binding.key = key;
            }
        }
    }

    if binding.key == 0 {
        warn!(binding = value, "ignoring hotkey without a key");
        return None;
    }

    Some(binding)
}

fn key_name_to_raw(name: &str) -> Option<u32> {
    match name {
        "up" | "arrowup" => Some(keysyms::KEY_Up),
        "left" | "arrowleft" => Some(keysyms::KEY_Left),
        "right" | "arrowright" => Some(keysyms::KEY_Right),
        "down" | "arrowdown" => Some(keysyms::KEY_Down),
        "f" => Some(keysyms::KEY_f),
        "m" => Some(keysyms::KEY_m),
        _ => None,
    }
}

fn normalize_name(name: &str) -> String {
    name.chars()
        .filter(|ch| !matches!(ch, ' ' | '\t' | '-' | '_'))
        .flat_map(char::to_lowercase)
        .collect()
}

fn config_path() -> PathBuf {
    if let Some(config_home) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(config_home).join("yawc/config");
    }

    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config/yawc/config")
}

const DEFAULT_CONFIG: &str = r#"# YAWC config
# Edit this file while YAWC is running; hotkeys and animations reload automatically.
#
# Binding format:
#   Modifier+Modifier+Key
#
# Modifiers:
#   Super, Ctrl, Alt, Shift
#
# Keys currently supported by the config parser:
#   Up, Down, Left, Right, F, M
#
# Set any binding to "none" to disable it.

maximize = Super+Up
snap_left = Super+Left
snap_right = Super+Right
fullscreen = Ctrl+Alt+F
minimize = Ctrl+Alt+M

# Animations:
#   animations = true/false
#   animation_ms changes both popup and maximize/snap timing.
#   popup_animation_ms and geometry_animation_ms can tune them separately.
animations = true
popup_animation_ms = 180
geometry_animation_ms = 220
"#;
