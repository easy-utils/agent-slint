// Navigation model — port of the Flutter/Compose/webui contract.
//
// Two side tabs, each with its own page stack. Page keys and config sub-ids are
// identical to every other Easy Agent client (guarded by tools/pages.py), so a
// page added here must be added everywhere.

/// The side-tab set (page contract).
pub const SIDER_TABS: &[&str] = &["chat", "config"];

/// The config drill-in sub-pages (page contract).
pub const CONFIG_SUB_IDS: &[&str] = &["appearance", "backends", "presets", "tools"];

/// The chat-tab overlays (page contract).
pub const SESSION_OVERLAYS: &[&str] = &["mailbox"];

/// Each static page key -> the view that dispatches it (the dispatch table the
/// guard checks).
pub fn view_for(key: &str) -> &'static str {
    match key {
        "chat_list" => "sessions_view",
        "chat_session" => "chat_view",
        "chat_overlay" => "mailbox_view",
        "config_root" => "config_view",
        "providers_list" => "providers_view",
        "preset_form_new" => "preset_form_view",
        "provider_form" => "provider_form_view",
        _ => "config_view",
    }
}

pub fn root_key(tab: &str) -> &'static str {
    if tab == "chat" {
        "chat_list"
    } else {
        "config_root"
    }
}

/// Per-tab page stacks with the same push/pop semantics as the other clients.
pub struct NavStore {
    chat: Vec<String>,
    config: Vec<String>,
    pub tab: String,
    pub active_session_id: String,
}

impl Default for NavStore {
    fn default() -> Self {
        Self::new()
    }
}

impl NavStore {
    pub fn new() -> Self {
        Self {
            chat: vec!["chat_list".to_string()],
            config: vec!["config_root".to_string()],
            tab: "chat".to_string(),
            active_session_id: String::new(),
        }
    }

    fn stack_mut(&mut self) -> &mut Vec<String> {
        if self.tab == "chat" {
            &mut self.chat
        } else {
            &mut self.config
        }
    }

    pub fn top(&self) -> String {
        let s = if self.tab == "chat" { &self.chat } else { &self.config };
        s.last().cloned().unwrap_or_else(|| root_key(&self.tab).to_string())
    }

    pub fn push(&mut self, key: &str) {
        self.stack_mut().push(key.to_string());
    }

    pub fn pop(&mut self) {
        let s = self.stack_mut();
        if s.len() > 1 {
            s.pop();
        }
    }
}
