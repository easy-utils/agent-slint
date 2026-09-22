// Thin RPC facade over the generated agent-sdk-rust messages + easy-rpc
// transport. Mirrors the JS/Python/Go clients: same POST paths, same
// application/connect+proto framing, same streamed Prompt events.
//
// Some methods are part of the shared client contract but not yet wired to a
// view; keep them (the guard checks the surface, not the call graph).
#![allow(dead_code)]

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use agent_sdk_rust::agent::v1 as pb;
use easy_rpc::bridge_reqwest::NewClient;
use easy_rpc::protocol::{Headers, Request, Stream, Transport};
use prost::Message as _;

fn transport(base: &str, token: &str) -> (String, Arc<dyn Transport>, Headers) {
    let base = base.trim_end_matches('/').to_string();
    let t: Arc<dyn Transport> = Arc::from(NewClient(base.clone()));
    let mut h = Headers::new();
    h.insert("Authorization".to_string(), vec![format!("Bearer {token}")]);
    (base, t, h)
}

fn req(base: &str, headers: &Headers, path: &str, body: Vec<u8>) -> Request {
    let _ = base;
    Request {
        url: path.to_string(),
        headers: headers.clone(),
        body: Some(body.into()),
    }
}

fn map_err(e: easy_rpc::protocol::RPCError) -> String {
    e.to_string()
}

pub fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// Verify the gateway is reachable and the token is accepted.
pub async fn connect(base: &str, token: &str) -> Result<(), String> {
    let (_b, t, h) = transport(base, token);
    let r = t
        .send(req(base, &h, "/agent.v1.AgentService/Health", pb::HealthRequest {}.encode_to_vec()))
        .await
        .map_err(map_err)?;
    if r.status >= 300 {
        return Err(format!("health: HTTP {} {}", r.status, String::from_utf8_lossy(&r.body)));
    }
    Ok(())
}

pub async fn list_sessions(base: &str, token: &str) -> Result<Vec<String>, String> {
    let (_b, t, h) = transport(base, token);
    let r = t
        .send(req(base, &h, "/agent.v1.AgentService/ListSessions", pb::ListSessionsRequest {}.encode_to_vec()))
        .await
        .map_err(map_err)?;
    if r.status >= 300 {
        return Err(format!("list_sessions: HTTP {}", r.status));
    }
    let out = pb::ListSessionsResponse::decode(&r.body[..]).map_err(|e| e.to_string())?;
    // Ordered most-recent-first (lastMessageAt -> updatedAt -> createdAt).
    let mut sessions: Vec<_> = out.sessions.iter().collect();
    sessions.sort_by_key(|s| std::cmp::Reverse(session_recency(s)));
    Ok(sessions.into_iter().map(|s| s.name.clone()).collect())
}

/// A session's epoch-ms recency: lastMessageAt -> updatedAt -> createdAt.
fn session_recency(s: &pb::Session) -> i64 {
    for v in [&s.last_message_at, &s.updated_at, &s.created_at] {
        if v.is_empty() {
            continue;
        }
        if let Ok(d) = v.parse::<chrono_lite::DateTime>() {
            return d.millis;
        }
    }
    0
}

/// Minimal RFC3339 -> epoch-ms parser (no chrono dependency). Handles the
/// `YYYY-MM-DDTHH:MM:SS(.fff)?Z` shape the agent emits.
mod chrono_lite {
    pub struct DateTime {
        pub millis: i64,
    }
    impl std::str::FromStr for DateTime {
        type Err = ();
        fn from_str(s: &str) -> Result<Self, Self::Err> {
            // 2026-09-22T10:00:00.000Z
            let bytes = s.as_bytes();
            if bytes.len() < 19 {
                return Err(());
            }
            let num = |a: usize, b: usize| -> Result<i64, ()> {
                s.get(a..b).ok_or(())?.parse::<i64>().map_err(|_| ())
            };
            let (y, mo, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
            let (hh, mm, ss) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
            let mut ms = 0i64;
            if bytes.get(19) == Some(&b'.') {
                ms = num(20, 23)?;
            }
            // days since epoch (civil algorithm)
            let y2 = if mo <= 2 { y - 1 } else { y };
            let era = if y2 >= 0 { y2 } else { y2 - 399 } / 400;
            let yoe = y2 - era * 400;
            let mp = (mo + 9) % 12;
            let doy = (153 * mp + 2) / 5 + d - 1;
            let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
            let days = era * 146097 + doe - 719468;
            Ok(DateTime {
                millis: ((days * 86400) + hh * 3600 + mm * 60 + ss) * 1000 + ms,
            })
        }
    }
}

pub async fn create_session(base: &str, token: &str, name: &str) -> Result<String, String> {
    let (_b, t, h) = transport(base, token);
    let body = pb::CreateSessionRequest {
        name: name.to_string(),
        ..Default::default()
    }
    .encode_to_vec();
    let r = t
        .send(req(base, &h, "/agent.v1.AgentService/CreateSession", body))
        .await
        .map_err(map_err)?;
    if r.status >= 300 {
        return Err(format!("create_session: HTTP {}", r.status));
    }
    let out = pb::CreateSessionResponse::decode(&r.body[..]).map_err(|e| e.to_string())?;
    if out.session_name.is_empty() {
        return Err("create_session: empty name".to_string());
    }
    Ok(out.session_name)
}

/// Render a stored message chain into display lines.
pub async fn list_messages(
    base: &str,
    token: &str,
    id: &str,
    limit: i32,
) -> Result<Vec<String>, String> {
    let (_b, t, h) = transport(base, token);
    let body = pb::ListMessagesRequest {
        id: id.to_string(),
        limit,
        ..Default::default()
    }
    .encode_to_vec();
    let r = t
        .send(req(base, &h, "/agent.v1.AgentService/ListMessages", body))
        .await
        .map_err(map_err)?;
    if r.status >= 300 {
        return Err(format!("list_messages: HTTP {}", r.status));
    }
    let out = pb::ListMessagesResponse::decode(&r.body[..]).map_err(|e| e.to_string())?;
    let mut lines = Vec::new();
    for m in &out.messages {
        let who = match m.source.as_str() {
            s if s.starts_with("session:") => format!("[{}]", &s["session:".len()..]),
            s if s.starts_with("system:") => format!("[system:{}]", &s["system:".len()..]),
            _ => match m.role.as_str() {
                "user" => "You".to_string(),
                "assistant" => "Agent".to_string(),
                other => other.to_string(),
            },
        };
        for p in &m.parts {
            match p.r#type.as_str() {
                "text" | "reasoning" => {
                    let d: serde_json::Value =
                        serde_json::from_str(&p.data).unwrap_or(serde_json::Value::Null);
                    if let Some(txt) = d.get("text").and_then(|v| v.as_str()) {
                        if !txt.trim().is_empty() {
                            lines.push(format!("{who}: {txt}"));
                        }
                    }
                }
                "tool" => {
                    let d: serde_json::Value =
                        serde_json::from_str(&p.data).unwrap_or(serde_json::Value::Null);
                    let name = d.get("name").and_then(|v| v.as_str()).unwrap_or("tool");
                    lines.push(format!("[tool: {name}]"));
                }
                _ => {}
            }
        }
    }
    Ok(lines)
}

/// Start a turn and invoke `on_event` for every streamed event until END.
pub async fn prompt<F>(
    base: &str,
    token: &str,
    id: &str,
    text: &str,
    mut on_event: F,
) -> Result<(), String>
where
    F: FnMut(&str, &std::collections::HashMap<String, String>) + Send + 'static,
{
    let (_b, t, h) = transport(base, token);
    let body = pb::PromptRequest {
        id: id.to_string(),
        prompt: text.to_string(),
        attachments: Vec::new(),
    }
    .encode_to_vec();
    let mut st: Box<dyn Stream> = t
        .open_stream(req(base, &h, "/agent.v1.AgentService/Prompt", body))
        .await
        .map_err(map_err)?;
    while let Some(chunk) = st.recv().await {
        let ev = match pb::PromptResponse::decode(&chunk[..]) {
            Ok(ev) => ev,
            Err(_) => continue,
        };
        on_event(&ev.event, &ev.params);
    }
    match st.last_error() {
        Some(e) => Err(e.to_string()),
        None => Ok(()),
    }
}

/// The caller's resolved identity (tenant id/name + role), from the token.
pub async fn identity(base: &str, token: &str) -> Result<pb::GetIdentityResponse, String> {
    let (_b, t, h) = transport(base, token);
    let r = t
        .send(req(base, &h, "/agent.v1.AgentService/GetIdentity", pb::GetIdentityRequest {}.encode_to_vec()))
        .await
        .map_err(map_err)?;
    if r.status >= 300 {
        return Err(format!("get_identity: HTTP {}", r.status));
    }
    pb::GetIdentityResponse::decode(&r.body[..]).map_err(|e| e.to_string())
}

/// Best-effort display name for the saved-backend list.
pub async fn resolve_username(base: &str, token: &str) -> String {
    match identity(base, token).await {
        Ok(r) if !r.tenant_name.is_empty() => r.tenant_name,
        Ok(r) => r.tenant,
        Err(_) => String::new(),
    }
}

/// Only model/preset/locale/variant are client-editable (proto v0.18).
pub async fn update_settings(
    base: &str,
    token: &str,
    id: &str,
    model: &str,
    preset: &str,
    locale: &str,
    variant: &str,
) -> Result<(), String> {
    let (_b, t, h) = transport(base, token);
    let body = pb::UpdateSettingsRequest {
        id: id.to_string(),
        model: model.to_string(),
        preset: preset.to_string(),
        locale: locale.to_string(),
        variant: variant.to_string(),
    }
    .encode_to_vec();
    let r = t
        .send(req(base, &h, "/agent.v1.AgentService/UpdateSettings", body))
        .await
        .map_err(map_err)?;
    if r.status >= 300 {
        return Err(format!("update_settings: HTTP {}", r.status));
    }
    Ok(())
}

pub async fn list_presets(base: &str, token: &str, locale: &str) -> Result<Vec<String>, String> {
    let (_b, t, h) = transport(base, token);
    let body = pb::ListPresetsRequest { locale: locale.to_string() }.encode_to_vec();
    let r = t
        .send(req(base, &h, "/agent.v1.AgentService/ListPresets", body))
        .await
        .map_err(map_err)?;
    if r.status >= 300 {
        return Err(format!("list_presets: HTTP {}", r.status));
    }
    let out = pb::ListPresetsResponse::decode(&r.body[..]).map_err(|e| e.to_string())?;
    Ok(out.presets.iter().map(|p| p.id.clone()).collect())
}

pub async fn list_providers(base: &str, token: &str) -> Result<Vec<String>, String> {
    let (_b, t, h) = transport(base, token);
    let body = pb::ListProvidersRequest {}.encode_to_vec();
    let r = t
        .send(req(base, &h, "/agent.v1.AgentService/ListProviders", body))
        .await
        .map_err(map_err)?;
    if r.status >= 300 {
        return Err(format!("list_providers: HTTP {}", r.status));
    }
    let out = pb::ListProvidersResponse::decode(&r.body[..]).map_err(|e| e.to_string())?;
    Ok(out
        .providers
        .iter()
        .map(|p| format!("{} · {}", p.provider_id, p.capability))
        .collect())
}

pub async fn list_tools(base: &str, token: &str, locale: &str) -> Result<Vec<String>, String> {
    let (_b, t, h) = transport(base, token);
    let body = pb::ListToolsRequest { locale: locale.to_string() }.encode_to_vec();
    let r = t
        .send(req(base, &h, "/agent.v1.AgentService/ListTools", body))
        .await
        .map_err(map_err)?;
    if r.status >= 300 {
        return Err(format!("list_tools: HTTP {}", r.status));
    }
    let out = pb::ListToolsResponse::decode(&r.body[..]).map_err(|e| e.to_string())?;
    Ok(out.tools.iter().map(|t| t.name.clone()).collect())
}

pub async fn get_config(base: &str, token: &str, key: &str) -> Result<String, String> {
    let (_b, t, h) = transport(base, token);
    let body = pb::GetConfigRequest { key: key.to_string() }.encode_to_vec();
    let r = t
        .send(req(base, &h, "/agent.v1.AgentService/GetConfig", body))
        .await
        .map_err(map_err)?;
    if r.status >= 300 {
        return Err(format!("get_config: HTTP {}", r.status));
    }
    let out = pb::GetConfigResponse::decode(&r.body[..]).map_err(|e| e.to_string())?;
    Ok(out.value)
}

pub async fn set_config(base: &str, token: &str, key: &str, value: &str) -> Result<(), String> {
    let (_b, t, h) = transport(base, token);
    let body = pb::SetConfigRequest {
        key: key.to_string(),
        value: value.to_string(),
    }
    .encode_to_vec();
    let r = t
        .send(req(base, &h, "/agent.v1.AgentService/SetConfig", body))
        .await
        .map_err(map_err)?;
    if r.status >= 300 {
        return Err(format!("set_config: HTTP {}", r.status));
    }
    Ok(())
}

/// One page of the mailbox (NEWEST-FIRST, paged backward).
pub struct MailboxPage {
    pub entries: Vec<String>,
    pub has_more: bool,
}

pub async fn mailbox(
    base: &str,
    token: &str,
    id: &str,
    before: &str,
    limit: i32,
) -> Result<MailboxPage, String> {
    let (_b, t, h) = transport(base, token);
    let body = pb::MailboxRequest {
        id: id.to_string(),
        before: before.to_string(),
        limit,
    }
    .encode_to_vec();
    let r = t
        .send(req(base, &h, "/agent.v1.AgentService/Mailbox", body))
        .await
        .map_err(map_err)?;
    if r.status >= 300 {
        return Err(format!("mailbox: HTTP {}", r.status));
    }
    let out = pb::MailboxResponse::decode(&r.body[..]).map_err(|e| e.to_string())?;
    Ok(MailboxPage {
        has_more: out.has_more,
        entries: out
            .mailbox
            .iter()
            .map(|m| format!("{} · {}", mailbox_label(&m.msg_type, &m.source), m.status))
            .collect(),
    })
}

/// (msgType, source) → a human label (mirrors the other clients).
pub fn mailbox_label(msg_type: &str, source: &str) -> String {
    if msg_type == "interrupt" {
        return "Interrupt".to_string();
    }
    if msg_type != "trigger" {
        return "Event".to_string();
    }
    if source == "user" {
        return "Message".to_string();
    }
    if let Some(name) = source.strip_prefix("session:") {
        return format!("From session · {name}");
    }
    if let Some(name) = source.strip_prefix("system:") {
        return format!("From system · {name}");
    }
    "Message".to_string()
}

/// Render a message origin label (session hand-off / system notice).
pub fn source_label(role: &str, source: &str) -> String {
    if let Some(name) = source.strip_prefix("session:") {
        return format!("[{name}]");
    }
    if let Some(name) = source.strip_prefix("system:") {
        return format!("[system:{name}]");
    }
    role.to_uppercase()
}

/// Fork a session WITHOUT opening it.
pub async fn fork(base: &str, token: &str, id: &str, branch: &str) -> Result<(), String> {
    let (_b, t, h) = transport(base, token);
    let body = pb::ForkRequest {
        id: id.to_string(),
        name: branch.to_string(),
        ..Default::default()
    }
    .encode_to_vec();
    let r = t
        .send(req(base, &h, "/agent.v1.AgentService/Fork", body))
        .await
        .map_err(map_err)?;
    if r.status >= 300 {
        return Err(format!("fork: HTTP {}", r.status));
    }
    Ok(())
}
