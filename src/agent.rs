// Thin RPC facade over the generated agent-sdk-rust messages + easy-rpc
// transport. Mirrors the JS/Python/Go clients: same POST paths, same
// application/connect+proto framing, same streamed Prompt events.

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
