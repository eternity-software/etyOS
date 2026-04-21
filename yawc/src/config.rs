use std::{
    fs,
    path::PathBuf,
    time::{Duration, Instant, SystemTime},
};

use smithay::input::keyboard::XkbConfig;
use smithay::input::keyboard::{keysyms, ModifiersState};
use tracing::{info, warn};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HotkeyAction {
    ToggleMaximize,
    SnapLeft,
    SnapRight,
    ToggleFullscreen,
    ToggleMinimize,
    SwitchKeyboardLayout,
    CloseWindow,
    KillWindow,
}

#[derive(Clone, Debug)]
pub struct Config {
    path: PathBuf,
    modified: Option<SystemTime>,
    last_checked: Instant,
    hotkeys: Hotkeys,
    animations: AnimationConfig,
    keyboard: KeyboardConfig,
    window_controls: WindowControlsMode,
    screencopy_dmabuf: bool,
}

#[derive(Clone, Debug)]
pub struct Hotkeys {
    maximize: Option<KeyBinding>,
    snap_left: Option<KeyBinding>,
    snap_right: Option<KeyBinding>,
    fullscreen: Option<KeyBinding>,
    minimize: Option<KeyBinding>,
    close: Option<KeyBinding>,
    kill: Option<KeyBinding>,
    layout_switch: Option<ModifierBinding>,
}

#[derive(Clone, Copy, Debug)]
struct KeyBinding {
    ctrl: bool,
    alt: bool,
    shift: bool,
    logo: bool,
    key: u32,
}

#[derive(Clone, Copy, Debug)]
struct ModifierBinding {
    ctrl: bool,
    alt: bool,
    shift: bool,
    logo: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnimationConfig {
    pub enabled: bool,
    pub popup_ms: u64,
    pub geometry_ms: u64,
    pub decoration_ms: u64,
    pub close_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WindowControlsMode {
    Gestures,
    Buttons,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyboardConfig {
    pub layouts: String,
    pub model: String,
    pub variant: String,
    pub options: Option<String>,
}

impl Default for AnimationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            popup_ms: 180,
            geometry_ms: 220,
            decoration_ms: 140,
            close_ms: 220,
        }
    }
}

impl Default for WindowControlsMode {
    fn default() -> Self {
        Self::Gestures
    }
}

impl Default for KeyboardConfig {
    fn default() -> Self {
        Self {
            layouts: "us,ru".to_string(),
            model: String::new(),
            variant: String::new(),
            options: None,
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
            close: parse_binding("Super+W"),
            kill: parse_binding("Super+Q"),
            layout_switch: parse_modifier_binding("Alt+Shift"),
        }
    }
}

impl KeyboardConfig {
    pub fn xkb_config(&self) -> XkbConfig<'_> {
        XkbConfig {
            rules: "",
            model: self.model.as_str(),
            layout: self.layouts.as_str(),
            variant: self.variant.as_str(),
            options: self.options.clone(),
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
            keyboard: KeyboardConfig::default(),
            window_controls: WindowControlsMode::default(),
            screencopy_dmabuf: false,
        };
        config.reload();
        config
    }

    pub fn reload_if_changed(&mut self) -> bool {
        if self.last_checked.elapsed() < Duration::from_millis(100) {
            return false;
        }
        self.last_checked = Instant::now();

        let modified = fs::metadata(&self.path)
            .and_then(|metadata| metadata.modified())
            .ok();
        if modified != self.modified {
            return self.reload();
        }

        false
    }

    pub fn hotkey_action(&self, key: u32, modifiers: ModifiersState) -> Option<HotkeyAction> {
        self.hotkeys.action_for(key, modifiers)
    }

    pub fn modifier_hotkey_action(
        &self,
        key: u32,
        modifiers: ModifiersState,
    ) -> Option<HotkeyAction> {
        self.hotkeys.modifier_action_for(key, modifiers)
    }

    pub fn animations(&self) -> AnimationConfig {
        self.animations
    }

    pub fn keyboard(&self) -> KeyboardConfig {
        self.keyboard.clone()
    }

    pub fn window_controls(&self) -> WindowControlsMode {
        self.window_controls
    }

    pub fn screencopy_dmabuf(&self) -> bool {
        self.screencopy_dmabuf
    }

    fn reload(&mut self) -> bool {
        let contents = match fs::read_to_string(&self.path) {
            Ok(contents) => contents,
            Err(error) => {
                warn!(
                    ?error,
                    path = %self.path.display(),
                    "failed to read config; keeping current config"
                );
                return false;
            }
        };

        let parsed = parse_config(&contents);
        self.hotkeys = parsed.hotkeys;
        self.animations = parsed.animations;
        self.keyboard = parsed.keyboard;
        self.window_controls = parsed.window_controls;
        self.screencopy_dmabuf = parsed.screencopy_dmabuf;
        self.modified = fs::metadata(&self.path)
            .and_then(|metadata| metadata.modified())
            .ok();
        info!(path = %self.path.display(), "loaded YAWC config");
        true
    }
}

#[derive(Clone, Debug)]
struct ParsedConfig {
    hotkeys: Hotkeys,
    animations: AnimationConfig,
    keyboard: KeyboardConfig,
    window_controls: WindowControlsMode,
    screencopy_dmabuf: bool,
}

impl Hotkeys {
    fn action_for(&self, key: u32, modifiers: ModifiersState) -> Option<HotkeyAction> {
        [
            (HotkeyAction::ToggleMaximize, self.maximize),
            (HotkeyAction::SnapLeft, self.snap_left),
            (HotkeyAction::SnapRight, self.snap_right),
            (HotkeyAction::ToggleFullscreen, self.fullscreen),
            (HotkeyAction::ToggleMinimize, self.minimize),
            (HotkeyAction::CloseWindow, self.close),
            (HotkeyAction::KillWindow, self.kill),
        ]
        .into_iter()
        .find_map(|(action, binding)| {
            binding
                .filter(|binding| binding.matches(key, modifiers))
                .map(|_| action)
        })
    }

    fn modifier_action_for(&self, key: u32, modifiers: ModifiersState) -> Option<HotkeyAction> {
        self.layout_switch
            .filter(|binding| binding.matches(modifiers) && is_modifier_key(key))
            .map(|_| HotkeyAction::SwitchKeyboardLayout)
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

impl ModifierBinding {
    fn matches(self, modifiers: ModifiersState) -> bool {
        self.ctrl == modifiers.ctrl
            && self.alt == modifiers.alt
            && self.shift == modifiers.shift
            && self.logo == modifiers.logo
    }
}

fn parse_config(contents: &str) -> ParsedConfig {
    let mut hotkeys = Hotkeys::default();
    let mut animations = AnimationConfig::default();
    let mut keyboard = KeyboardConfig::default();
    let mut window_controls = WindowControlsMode::default();
    let mut screencopy_dmabuf = false;

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
            "hotkeyclose" | "close" => hotkeys.close = parse_binding(value),
            "hotkeykill" | "kill" | "killprocess" | "forcekill" => {
                hotkeys.kill = parse_binding(value)
            }
            "hotkeylayoutswitch" | "layoutswitch" | "keyboardlayoutswitch" => {
                hotkeys.layout_switch = parse_modifier_binding(value)
            }
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
                    animations.close_ms = duration;
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
            "decorationanimationms" | "decorationduration" | "decorationdurationms" => {
                if let Some(duration) = parse_duration_ms(value) {
                    animations.decoration_ms = duration;
                } else {
                    warn!(
                        line = line_number + 1,
                        value, "ignoring invalid decoration animation duration"
                    );
                }
            }
            "closeanimationms" | "closeduration" | "closedurationms" => {
                if let Some(duration) = parse_duration_ms(value) {
                    animations.close_ms = duration;
                } else {
                    warn!(
                        line = line_number + 1,
                        value, "ignoring invalid close animation duration"
                    );
                }
            }
            "windowcontrols" | "titlebarcontrols" | "controlsmode" => {
                if let Some(mode) = parse_window_controls(value) {
                    window_controls = mode;
                } else {
                    warn!(
                        line = line_number + 1,
                        value, "ignoring invalid window controls mode"
                    );
                }
            }
            "screencopydmabuf" | "screencopydmabufs" | "dmabufscreencopy" => {
                if let Some(enabled) = parse_bool(value) {
                    screencopy_dmabuf = enabled;
                } else {
                    warn!(
                        line = line_number + 1,
                        value, "ignoring invalid screencopy dmabuf boolean"
                    );
                }
            }
            "keyboardlayouts" | "xkblayout" | "layouts" => {
                keyboard.layouts = value.trim().to_string();
            }
            "keyboardmodel" | "xkbmodel" => {
                keyboard.model = value.trim().to_string();
            }
            "keyboardvariant" | "xkbvariant" => {
                keyboard.variant = value.trim().to_string();
            }
            "keyboardoptions" | "xkboptions" => {
                let value = value.trim();
                keyboard.options = (!value.is_empty()).then(|| value.to_string());
            }
            _ => warn!(line = line_number + 1, key, "ignoring unknown config key"),
        }
    }

    if keyboard.layouts.trim().is_empty() {
        warn!("keyboard_layouts is empty; falling back to us,ru");
        keyboard.layouts = "us,ru".to_string();
    }

    ParsedConfig {
        hotkeys,
        animations,
        keyboard,
        window_controls,
        screencopy_dmabuf,
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

fn parse_window_controls(value: &str) -> Option<WindowControlsMode> {
    match normalize_name(value).as_str() {
        "gestures" | "gesture" | "modern" | "hidden" | "none" | "buttonless" => {
            Some(WindowControlsMode::Gestures)
        }
        "buttons" | "button" | "classic" | "windows" | "windowsmode" | "default" => {
            Some(WindowControlsMode::Buttons)
        }
        _ => None,
    }
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

fn parse_modifier_binding(value: &str) -> Option<ModifierBinding> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("none")
        || value.eq_ignore_ascii_case("disabled")
        || value.eq_ignore_ascii_case("off")
    {
        return None;
    }

    let mut binding = ModifierBinding {
        ctrl: false,
        alt: false,
        shift: false,
        logo: false,
    };

    for part in value.split('+') {
        let normalized = normalize_name(part);
        match normalized.as_str() {
            "ctrl" | "control" => binding.ctrl = true,
            "alt" | "meta" => binding.alt = true,
            "shift" => binding.shift = true,
            "super" | "logo" | "mod4" | "win" | "windows" => binding.logo = true,
            _ => {
                warn!(binding = value, part, "ignoring invalid modifier hotkey");
                return None;
            }
        }
    }

    if !(binding.ctrl || binding.alt || binding.shift || binding.logo) {
        warn!(
            binding = value,
            "ignoring modifier hotkey without modifiers"
        );
        return None;
    }

    Some(binding)
}

fn is_modifier_key(key: u32) -> bool {
    matches!(
        key,
        keysyms::KEY_Shift_L
            | keysyms::KEY_Shift_R
            | keysyms::KEY_Control_L
            | keysyms::KEY_Control_R
            | keysyms::KEY_Alt_L
            | keysyms::KEY_Alt_R
            | keysyms::KEY_Super_L
            | keysyms::KEY_Super_R
            | keysyms::KEY_Meta_L
            | keysyms::KEY_Meta_R
    )
}

fn key_name_to_raw(name: &str) -> Option<u32> {
    match name {
        "up" | "arrowup" => Some(keysyms::KEY_Up),
        "left" | "arrowleft" => Some(keysyms::KEY_Left),
        "right" | "arrowright" => Some(keysyms::KEY_Right),
        "down" | "arrowdown" => Some(keysyms::KEY_Down),
        "f" => Some(keysyms::KEY_f),
        "m" => Some(keysyms::KEY_m),
        "q" => Some(keysyms::KEY_q),
        "w" => Some(keysyms::KEY_w),
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
#   Up, Down, Left, Right, F, M, Q, W
#
# Set any binding to "none" to disable it.

maximize = Super+Up
snap_left = Super+Left
snap_right = Super+Right
fullscreen = Ctrl+Alt+F
minimize = Ctrl+Alt+M
close = Super+W
kill = Super+Q
layout_switch = Alt+Shift

# Keyboard:
#   keyboard_layouts is an XKB comma-separated layout list, for example: us,ru
#   keyboard_model, keyboard_variant, and keyboard_options are passed to xkbcommon.
keyboard_layouts = us,ru
keyboard_model =
keyboard_variant =
keyboard_options =

# Animations:
#   animations = true/false
#   animation_ms changes both popup and maximize/snap timing.
#   popup_animation_ms, geometry_animation_ms, decoration_animation_ms, and close_animation_ms can tune them separately.
animations = true
popup_animation_ms = 180
geometry_animation_ms = 220
decoration_animation_ms = 140
close_animation_ms = 220

# Window controls:
#   gestures: no titlebar buttons, right-click titlebar to close, double-click titlebar to maximize.
#   buttons/windows/classic: show close/maximize/minimize buttons.
window_controls = gestures
screencopy_dmabuf = false
"#;
