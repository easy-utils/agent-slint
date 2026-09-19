// Easy Agent — Slint (Rust) client for the standalone agent.
//
// Same easy-rpc (Connect) wire + generated agent-sdk-rust messages as every
// other Easy Agent client. The UI thread never blocks: every RPC runs on a
// tokio runtime and results are marshalled back with `invoke_from_event_loop`.

mod agent;

use std::sync::Arc;

use slint::{Model, ModelRc, SharedString, VecModel};

slint::include_modules!();

type Rt = Arc<tokio::runtime::Runtime>;

fn main() -> Result<(), slint::PlatformError> {
    let ui = AppWindow::new()?;
    let rt: Rt = Arc::new(tokio::runtime::Runtime::new().expect("tokio runtime"));

    // ---- connect ----
    {
        let ui_weak = ui.as_weak();
        let rt = rt.clone();
        ui.on_connect_clicked(move || {
            let ui = ui_weak.unwrap();
            let base = ui.get_base_url().to_string();
            let token = ui.get_token().to_string();
            ui.set_status("Connecting…".into());
            let ui_weak = ui.as_weak();
            rt.spawn(async move {
                let result = agent::connect(&base, &token).await;
                let sessions = if result.is_ok() {
                    agent::list_sessions(&base, &token).await.ok()
                } else {
                    None
                };
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        match result {
                            Ok(()) => {
                                ui.set_connected(true);
                                ui.set_status("".into());
                                if let Some(s) = sessions {
                                    ui.set_sessions(ModelRc::new(VecModel::from(
                                        s.into_iter().map(SharedString::from).collect::<Vec<_>>(),
                                    )));
                                }
                            }
                            Err(e) => ui.set_status(format!("Connect failed: {e}").into()),
                        }
                    }
                });
            });
        });
    }

    // ---- refresh ----
    {
        let ui_weak = ui.as_weak();
        let rt = rt.clone();
        ui.on_refresh_clicked(move || {
            let ui = ui_weak.unwrap();
            let base = ui.get_base_url().to_string();
            let token = ui.get_token().to_string();
            let ui_weak = ui.as_weak();
            rt.spawn(async move {
                if let Ok(s) = agent::list_sessions(&base, &token).await {
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = ui_weak.upgrade() {
                            ui.set_sessions(ModelRc::new(VecModel::from(
                                s.into_iter().map(SharedString::from).collect::<Vec<_>>(),
                            )));
                        }
                    });
                }
            });
        });
    }

    // ---- new session ----
    {
        let ui_weak = ui.as_weak();
        let rt = rt.clone();
        ui.on_new_session(move || {
            let ui = ui_weak.unwrap();
            let base = ui.get_base_url().to_string();
            let token = ui.get_token().to_string();
            let ui_weak = ui.as_weak();
            rt.spawn(async move {
                let name = format!("easy-{}", agent::now_ms());
                match agent::create_session(&base, &token, &name).await {
                    Ok(_) => {
                        if let Ok(s) = agent::list_sessions(&base, &token).await {
                            let _ = slint::invoke_from_event_loop(move || {
                                if let Some(ui) = ui_weak.upgrade() {
                                    ui.set_sessions(ModelRc::new(VecModel::from(
                                        s.into_iter()
                                            .map(SharedString::from)
                                            .collect::<Vec<_>>(),
                                    )));
                                }
                            });
                        }
                    }
                    Err(e) => {
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(ui) = ui_weak.upgrade() {
                                ui.set_status(format!("Create failed: {e}").into());
                            }
                        });
                    }
                }
            });
        });
    }

    // ---- open session ----
    {
        let ui_weak = ui.as_weak();
        let rt = rt.clone();
        ui.on_session_picked(move |idx| {
            let ui = ui_weak.unwrap();
            let base = ui.get_base_url().to_string();
            let token = ui.get_token().to_string();
            let id = match ui.get_sessions().row_data(idx as usize) {
                Some(s) => s.to_string(),
                None => return,
            };
            let ui_weak = ui.as_weak();
            rt.spawn(async move {
                if let Ok(history) = agent::list_messages(&base, &token, &id, 50).await {
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = ui_weak.upgrade() {
                            ui.set_chat_title(id.into());
                            ui.set_transcript(ModelRc::new(VecModel::from(
                                history
                                    .into_iter()
                                    .map(SharedString::from)
                                    .collect::<Vec<_>>(),
                            )));
                        }
                    });
                }
            });
        });
    }

    // ---- send prompt ----
    {
        let ui_weak = ui.as_weak();
        let rt = rt.clone();
        ui.on_send_clicked(move || {
            let ui = ui_weak.unwrap();
            let base = ui.get_base_url().to_string();
            let token = ui.get_token().to_string();
            let id = ui.get_chat_title().to_string();
            let text = ui.get_composer().trim().to_string();
            if text.is_empty() || id.is_empty() {
                return;
            }
            ui.set_composer("".into());
            push_line(&ui, format!("You: {text}"));
            push_line(&ui, "Agent: ".to_string());

            let ui_weak = ui.as_weak();
            rt.spawn(async move {
                let ui2 = ui_weak.clone();
                let result = agent::prompt(&base, &token, &id, &text, move |event, params| {
                    let line = match event {
                        "text-delta" => params.get("text").cloned().unwrap_or_default(),
                        "reasoning-delta" => format!(
                            "[reasoning] {}",
                            params.get("text").cloned().unwrap_or_default()
                        ),
                        "tool-call" => format!(
                            "\n[tool: {}]\n",
                            params
                                .get("toolName")
                                .or_else(|| params.get("name"))
                                .cloned()
                                .unwrap_or_else(|| "tool".into())
                        ),
                        _ => return,
                    };
                    if line.is_empty() {
                        return;
                    }
                    append_delta(ui2.clone(), &line);
                })
                .await;
                if let Err(e) = result {
                    if let Some(ui) = ui_weak.upgrade() {
                        push_line(&ui, format!("[error: {e}]"));
                    }
                }
            });
        });
    }

    // ---- back ----
    {
        let ui_weak = ui.as_weak();
        ui.on_back_clicked(move || {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_chat_title("".into());
                ui.set_transcript(ModelRc::new(VecModel::from(Vec::<SharedString>::new())));
            }
        });
    }

    ui.run()
}

/// Append a complete line to the transcript on the UI thread.
fn push_line(ui: &AppWindow, line: String) {
    let mut v: Vec<SharedString> = ui.get_transcript().iter().collect();
    v.push(line.into());
    ui.set_transcript(ModelRc::new(VecModel::from(v)));
}

/// Append streamed text to the last (Agent) transcript line. Must be called on
/// the UI thread.
fn append_delta(ui_weak: slint::Weak<AppWindow>, text: &str) {
    let text = text.to_string();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = ui_weak.upgrade() {
            let mut v: Vec<SharedString> = ui.get_transcript().iter().collect();
            match v.last_mut() {
                Some(last) => *last = SharedString::from(format!("{last}{text}")),
                None => v.push(SharedString::from(text)),
            }
            ui.set_transcript(ModelRc::new(VecModel::from(v)));
        }
    });
}
