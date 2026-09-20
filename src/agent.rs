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
    Ok(out.sessions.iter().map(|s| s.name.clone()).collect())
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
        let who = match m.role.as_str() {
            "user" => "You",
            "assistant" => "Agent",
            other => other,
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

pub async fn mailbox(base: &str, token: &str, id: &str) -> Result<Vec<String>, String> {
    let (_b, t, h) = transport(base, token);
    let body = pb::MailboxRequest { id: id.to_string() }.encode_to_vec();
    let r = t
        .send(req(base, &h, "/agent.v1.AgentService/Mailbox", body))
        .await
        .map_err(map_err)?;
    if r.status >= 300 {
        return Err(format!("mailbox: HTTP {}", r.status));
    }
    let out = pb::MailboxResponse::decode(&r.body[..]).map_err(|e| e.to_string())?;
    Ok(out
        .mailbox
        .iter()
        .map(|m| format!("{} · {}", m.msg_type, m.status))
        .collect())
}
