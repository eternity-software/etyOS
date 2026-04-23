use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub type StyleMap = BTreeMap<String, String>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Badge {
    pub label: String,
    pub accent: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum SceneNode {
    #[serde(rename = "page")]
    Page {
        #[serde(default)]
        style: StyleMap,
        #[serde(default)]
        vars: StyleMap,
        children: Vec<SceneNode>,
    },
    #[serde(rename = "panel_shell")]
    PanelShell {
        eyebrow: String,
        title: String,
        subtitle: String,
        badge: String,
        children: Vec<SceneNode>,
    },
    #[serde(rename = "section")]
    Section {
        title: String,
        description: String,
        children: Vec<SceneNode>,
    },
    #[serde(rename = "grid")]
    Grid {
        #[serde(default)]
        style: StyleMap,
        #[serde(rename = "stackOnMobile")]
        stack_on_mobile: bool,
        children: Vec<SceneNode>,
    },
    #[serde(rename = "stat")]
    Stat { label: String, value: String },
    #[serde(rename = "button")]
    Button {
        id: String,
        label: String,
        variant: String,
        quiet: bool,
    },
    #[serde(rename = "segmented")]
    Segmented {
        id: String,
        items: Vec<String>,
        #[serde(rename = "activeIndex")]
        active_index: usize,
    },
    #[serde(rename = "input")]
    Input {
        id: String,
        label: String,
        placeholder: String,
        value: String,
        leading: Option<String>,
    },
    #[serde(rename = "slider")]
    Slider {
        id: String,
        label: String,
        value: u64,
        min: u64,
        max: u64,
    },
    #[serde(rename = "switch")]
    Switch {
        id: String,
        label: String,
        hint: String,
        checked: bool,
    },
    #[serde(rename = "textarea")]
    TextArea {
        id: String,
        label: String,
        placeholder: String,
        value: String,
    },
    #[serde(rename = "card")]
    Card {
        title: String,
        description: String,
        tone: String,
    },
    #[serde(rename = "list_row")]
    ListRow {
        title: String,
        subtitle: String,
        meta: String,
        badge: Option<Badge>,
    },
    #[serde(rename = "notification")]
    Notification {
        title: String,
        body: String,
        badge: String,
        tone: String,
    },
    #[serde(rename = "status_message")]
    StatusMessage {
        title: String,
        body: String,
        tone: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneEnvelope {
    pub scene: SceneNode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum HostCommand {
    #[serde(rename = "open_window")]
    OpenWindow {
        role: Option<String>,
        title: Option<String>,
        width: Option<u32>,
        height: Option<u32>,
        resizable: Option<bool>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AppletRequest {
    #[serde(rename = "window_opened")]
    WindowOpened {
        window_id: String,
        role: Option<String>,
    },
    #[serde(rename = "window_closed")]
    WindowClosed { window_id: String },
    #[serde(rename = "render")]
    Render { request_id: u64, window_id: String },
    #[serde(rename = "event")]
    Event {
        request_id: u64,
        window_id: String,
        node_id: String,
        event: String,
        value: Option<Value>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AppletResponse {
    #[serde(rename = "scene")]
    Scene {
        request_id: u64,
        scene: SceneNode,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        commands: Vec<HostCommand>,
    },
    #[serde(rename = "error")]
    Error {
        request_id: Option<u64>,
        message: String,
    },
}
