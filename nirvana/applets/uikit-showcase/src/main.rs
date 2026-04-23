use std::{
    collections::BTreeMap,
    io::{self, BufRead, Write},
};

use nirvana_protocol::{
    AppletRequest, AppletResponse, Badge, HostCommand, SceneNode, StyleMap,
};
use serde_json::Value;

struct AppletState {
    open_windows: BTreeMap<String, Option<String>>,
    active_tab: usize,
    search: String,
    profile_name: String,
    blur_intensity: u64,
    reduce_motion: bool,
    notes: String,
    last_callback: String,
}

impl Default for AppletState {
    fn default() -> Self {
        Self {
            open_windows: BTreeMap::new(),
            active_tab: 1,
            search: String::new(),
            profile_name: String::from("etyOS shell profile"),
            blur_intensity: 36,
            reduce_motion: false,
            notes: String::new(),
            last_callback: String::from("No callbacks received yet"),
        }
    }
}

fn styles(entries: &[(&str, &str)]) -> StyleMap {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

fn hello_world_scene() -> SceneNode {
    SceneNode::Page {
        style: styles(&[("background", "#f5f7fa")]),
        vars: styles(&[
            ("page-max-width", "720px"),
            ("page-gutter", "28px"),
            ("page-padding-top", "24px"),
            ("page-padding-bottom", "24px"),
            ("page-gap", "0"),
        ]),
        children: vec![SceneNode::Grid {
            style: styles(&[
                ("minHeight", "calc(100vh - 48px)"),
                ("placeItems", "center"),
            ]),
            stack_on_mobile: false,
            children: vec![SceneNode::StatusMessage {
                title: "Hello world".into(),
                body: String::new(),
                tone: "default".into(),
            }],
        }],
    }
}

fn primary_scene(state: &AppletState) -> SceneNode {
    let active_tab_label = ["General", "Display", "Privacy"]
        .get(state.active_tab)
        .copied()
        .unwrap_or("Unknown");

    SceneNode::Page {
        style: styles(&[("background", "#f5f7fa")]),
        vars: styles(&[
            ("page-max-width", "1360px"),
            ("page-gutter", "48px"),
            ("page-padding-top", "28px"),
            ("page-padding-bottom", "34px"),
            ("page-gap", "18px"),
        ]),
        children: vec![
            SceneNode::PanelShell {
                eyebrow: "Nirvana / Rust".into(),
                title: "UI Kit Showcase".into(),
                subtitle: "Only generic components stay inside Nirvana. The Rust applet owns the scene, layout, compact spacing, callbacks, and window lifecycle.".into(),
                badge: "Rust Applet".into(),
                children: vec![SceneNode::Grid {
                    style: styles(&[
                        ("marginTop", "18px"),
                        ("gridTemplateColumns", "repeat(5, minmax(0, 1fr))"),
                        ("gap", "12px"),
                    ]),
                    stack_on_mobile: true,
                    children: vec![
                        SceneNode::Stat {
                            label: "Source".into(),
                            value: "Persistent Rust".into(),
                        },
                        SceneNode::Stat {
                            label: "Open Windows".into(),
                            value: state.open_windows.len().to_string(),
                        },
                        SceneNode::Stat {
                            label: "Active Tab".into(),
                            value: active_tab_label.into(),
                        },
                        SceneNode::Stat {
                            label: "Last Callback".into(),
                            value: state.last_callback.clone(),
                        },
                        SceneNode::Stat {
                            label: "Extra Windows".into(),
                            value: state
                                .open_windows
                                .values()
                                .filter(|role| role.as_deref() == Some("hello-world"))
                                .count()
                                .to_string(),
                        },
                    ],
                }],
            },
            SceneNode::Section {
                title: "Buttons & Tabs".into(),
                description: "Every interactive component now sends callbacks back into the Rust applet process.".into(),
                children: vec![
                    SceneNode::Grid {
                        style: styles(&[("gridTemplateColumns", "repeat(5, minmax(0, 1fr))")]),
                        stack_on_mobile: true,
                        children: vec![
                            SceneNode::Button {
                                id: "primary-action".into(),
                                label: "Primary Action".into(),
                                variant: "primary".into(),
                                quiet: false,
                            },
                            SceneNode::Button {
                                id: "secondary-action".into(),
                                label: "Secondary".into(),
                                variant: "secondary".into(),
                                quiet: false,
                            },
                            SceneNode::Button {
                                id: "ghost-action".into(),
                                label: "Ghost".into(),
                                variant: "ghost".into(),
                                quiet: false,
                            },
                            SceneNode::Button {
                                id: "danger-action".into(),
                                label: "Danger".into(),
                                variant: "danger".into(),
                                quiet: false,
                            },
                            SceneNode::Button {
                                id: "open-hello-window".into(),
                                label: "Open Window".into(),
                                variant: "secondary".into(),
                                quiet: false,
                            },
                        ],
                    },
                    SceneNode::Segmented {
                        id: "settings-tabs".into(),
                        items: vec!["General".into(), "Display".into(), "Privacy".into()],
                        active_index: state.active_tab,
                    },
                ],
            },
            SceneNode::Section {
                title: "Inputs".into(),
                description:
                    "Change any control and the Rust applet re-renders the scene with updated state."
                        .into(),
                children: vec![
                    SceneNode::Grid {
                        style: styles(&[
                            ("gridTemplateColumns", "repeat(2, minmax(0, 1fr))"),
                            ("alignItems", "start"),
                        ]),
                        stack_on_mobile: true,
                        children: vec![
                            SceneNode::Input {
                                id: "search".into(),
                                label: "Search".into(),
                                placeholder: "Search settings, apps, shortcuts".into(),
                                value: state.search.clone(),
                                leading: Some("⌕".into()),
                            },
                            SceneNode::Input {
                                id: "profile-name".into(),
                                label: "Display Name".into(),
                                placeholder: String::new(),
                                value: state.profile_name.clone(),
                                leading: None,
                            },
                            SceneNode::Slider {
                                id: "blur-intensity".into(),
                                label: "Blur Intensity".into(),
                                value: state.blur_intensity,
                                min: 0,
                                max: 100,
                            },
                            SceneNode::Switch {
                                id: "reduce-motion".into(),
                                label: "Reduce Motion".into(),
                                hint: "Lower shell animation amplitude for sensitive users".into(),
                                checked: state.reduce_motion,
                            },
                        ],
                    },
                    SceneNode::TextArea {
                        id: "notes".into(),
                        label: "Multiline Note".into(),
                        placeholder: "Describe the target experience for this applet".into(),
                        value: state.notes.clone(),
                    },
                ],
            },
            SceneNode::Section {
                title: "Cards & Status".into(),
                description: "Compact surfaces without oversized paddings, glow, or giant controls.".into(),
                children: vec![SceneNode::Grid {
                    style: styles(&[("gridTemplateColumns", "repeat(3, minmax(0, 1fr))")]),
                    stack_on_mobile: true,
                    children: vec![
                        SceneNode::Card {
                            title: "Interaction State".into(),
                            description: format!("Last callback: {}", state.last_callback),
                            tone: "default".into(),
                        },
                        SceneNode::Card {
                            title: "Search Query".into(),
                            description: if state.search.is_empty() {
                                "No active search".into()
                            } else {
                                state.search.clone()
                            },
                            tone: "accent".into(),
                        },
                        SceneNode::Card {
                            title: "Notes Length".into(),
                            description: format!("{} characters", state.notes.chars().count()),
                            tone: "muted".into(),
                        },
                    ],
                }],
            },
            SceneNode::Section {
                title: "Rows & Badges".into(),
                description:
                    "List rows stay generic components, but their arrangement is owned by the Rust applet."
                        .into(),
                children: vec![SceneNode::Grid {
                    style: styles(&[("gap", "10px")]),
                    stack_on_mobile: false,
                    children: vec![
                        SceneNode::ListRow {
                            title: "Appearance".into(),
                            subtitle: "Wallpaper, accent palette, shell density".into(),
                            meta: "Ctrl+,".into(),
                            badge: Some(Badge {
                                label: "System".into(),
                                accent: false,
                            }),
                        },
                        SceneNode::ListRow {
                            title: "Display Profile".into(),
                            subtitle: state.profile_name.clone(),
                            meta: format!("Blur {}", state.blur_intensity),
                            badge: Some(Badge {
                                label: if state.reduce_motion { "Calm" } else { "Lively" }.into(),
                                accent: true,
                            }),
                        },
                        SceneNode::ListRow {
                            title: "Window Lifecycle".into(),
                            subtitle: format!(
                                "{} window(s) attached to this applet",
                                state.open_windows.len()
                            ),
                            meta: "Persistent".into(),
                            badge: None,
                        },
                    ],
                }],
            },
            SceneNode::Section {
                title: "Notifications".into(),
                description:
                    "Static examples still render through the same generic component set.".into(),
                children: vec![SceneNode::Grid {
                    style: styles(&[("gridTemplateColumns", "repeat(2, minmax(0, 1fr))")]),
                    stack_on_mobile: true,
                    children: vec![
                        SceneNode::Notification {
                            title: "Rust Applet Online".into(),
                            body: "The applet process is alive, receives UI callbacks, and exits once all of its windows are closed.".into(),
                            badge: "Live".into(),
                            tone: "accent".into(),
                        },
                        SceneNode::Notification {
                            title: "Window Count Changed".into(),
                            body: format!(
                                "Current attached windows: {}",
                                state.open_windows.len()
                            ),
                            badge: "State".into(),
                            tone: "default".into(),
                        },
                    ],
                }],
            },
        ],
    }
}

fn scene(state: &AppletState, window_id: &str) -> SceneNode {
    match state
        .open_windows
        .get(window_id)
        .and_then(|role| role.as_deref())
    {
        Some("hello-world") => hello_world_scene(),
        _ => primary_scene(state),
    }
}

fn update_state(
    state: &mut AppletState,
    node_id: &str,
    event: &str,
    value: Option<Value>,
) -> Vec<HostCommand> {
    match (node_id, event) {
        ("primary-action", "click") => {
            state.last_callback = String::from("Clicked primary-action");
        }
        ("secondary-action", "click") => {
            state.last_callback = String::from("Clicked secondary-action");
        }
        ("ghost-action", "click") => {
            state.last_callback = String::from("Clicked ghost-action");
        }
        ("danger-action", "click") => {
            state.last_callback = String::from("Clicked danger-action");
        }
        ("open-hello-window", "click") => {
            state.last_callback = String::from("Requested hello-world window");
            return vec![HostCommand::OpenWindow {
                role: Some("hello-world".into()),
                title: Some("Hello World".into()),
                width: Some(460),
                height: Some(260),
                resizable: Some(false),
            }];
        }
        ("settings-tabs", "select") => {
            if let Some(index) = value
                .as_ref()
                .and_then(|payload| payload.get("index"))
                .and_then(Value::as_u64)
            {
                state.active_tab = index as usize;
                state.last_callback = format!("Selected tab {}", state.active_tab);
            }
        }
        ("search", "change") => {
            state.search = value
                .and_then(|value| value.as_str().map(ToOwned::to_owned))
                .unwrap_or_default();
            state.last_callback = format!("Changed search to '{}'", state.search);
        }
        ("profile-name", "change") => {
            state.profile_name = value
                .and_then(|value| value.as_str().map(ToOwned::to_owned))
                .unwrap_or_default();
            state.last_callback = format!("Changed profile name to '{}'", state.profile_name);
        }
        ("blur-intensity", "input") => {
            state.blur_intensity = value
                .and_then(|value| value.as_u64())
                .unwrap_or(state.blur_intensity);
            state.last_callback = format!("Adjusted blur to {}", state.blur_intensity);
        }
        ("reduce-motion", "change") => {
            state.reduce_motion = value
                .and_then(|value| value.as_bool())
                .unwrap_or(state.reduce_motion);
            state.last_callback = format!("Reduce motion set to {}", state.reduce_motion);
        }
        ("notes", "change") => {
            state.notes = value
                .and_then(|value| value.as_str().map(ToOwned::to_owned))
                .unwrap_or_default();
            state.last_callback =
                format!("Updated notes ({} chars)", state.notes.chars().count());
        }
        _ => {}
    }

    Vec::new()
}

fn send_response(stdout: &mut io::StdoutLock<'_>, response: &AppletResponse) {
    let payload = serde_json::to_string(response).expect("serialize applet response");
    writeln!(stdout, "{payload}").expect("write applet response");
    stdout.flush().expect("flush applet response");
}

fn main() {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    let mut state = AppletState::default();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(line) => line,
            Err(_) => break,
        };

        let request = match serde_json::from_str::<AppletRequest>(&line) {
            Ok(request) => request,
            Err(error) => {
                let response = AppletResponse::Error {
                    request_id: None,
                    message: format!("Invalid request: {error}"),
                };
                send_response(&mut stdout, &response);
                continue;
            }
        };

        match request {
            AppletRequest::WindowOpened { window_id, role } => {
                state.open_windows.insert(window_id.clone(), role.clone());
                state.last_callback = match role.as_deref() {
                    Some("hello-world") => format!("Window {window_id} opened as hello-world"),
                    Some(role) => format!("Window {window_id} opened as {role}"),
                    None => format!("Window {window_id} opened"),
                };
            }
            AppletRequest::Render {
                request_id,
                window_id,
            } => {
                send_response(
                    &mut stdout,
                    &AppletResponse::Scene {
                        request_id,
                        scene: scene(&state, &window_id),
                        commands: Vec::new(),
                    },
                );
            }
            AppletRequest::Event {
                request_id,
                window_id,
                node_id,
                event,
                value,
                ..
            } => {
                let commands = update_state(&mut state, &node_id, &event, value);
                send_response(
                    &mut stdout,
                    &AppletResponse::Scene {
                        request_id,
                        scene: scene(&state, &window_id),
                        commands,
                    },
                );
            }
            AppletRequest::WindowClosed { window_id } => {
                state.open_windows.remove(&window_id);
                state.last_callback = format!("Window {window_id} closed");

                if state.open_windows.is_empty() {
                    break;
                }
            }
        }
    }
}
