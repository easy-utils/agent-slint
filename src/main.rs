// Easy Agent — Slint (Rust) client for the standalone agent.
//
// Same easy-rpc (Connect) wire + generated agent-sdk-rust messages as every
// other Easy Agent client. The UI thread never blocks: every RPC runs on a
// tokio runtime and results are marshalled back with `invoke_from_event_loop`.

mod agent;
mod navigation;

use std::sync::{Arc, Mutex};

use slint::{Model, ModelRc, SharedString, VecModel};

use navigation::NavStore;

slint::include_modules!();

type Rt = Arc<tokio::runtime::Runtime>;

/// Mirror the nav store's tab + top page onto the Slint properties.
fn sync_nav(ui: &AppWindow, nav: &NavStore) {
    ui.set_tab(nav.tab.clone().into());
    ui.set_page(nav.top().into());
    ui.set_chat_title(if nav.top() == "chat_session" {
        nav.active_session_id.clone().into()
    } else {
        "".into()
    });
}

fn main() -> Result<(), slint::PlatformError> {
    let ui = AppWindow::new()?;
    let rt: Rt = Arc::new(tokio::runtime::Runtime::new().expect("tokio runtime"));
    let nav: Arc<Mutex<NavStore>> = Arc::new(Mutex::new(NavStore::new()));
    // Seed the config list from the shared contract constant.
    ui.set_config_items(ModelRc::new(VecModel::from(
        navigation::CONFIG_SUB_IDS
            .iter()
            .map(|s| SharedString::from(*s))
            .collect::<Vec<_>>(),
    )));
    let _ = (navigation::SIDER_TABS, navigation::SESSION_OVERLAYS, navigation::view_for);

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
                if result.is_ok() {
                    let _ = agent::resolve_username(&base, &token).await;
                }
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
        let nav = nav.clone();
        ui.on_session_picked(move |idx| {
            let ui = ui_weak.unwrap();
            let base = ui.get_base_url().to_string();
            let token = ui.get_token().to_string();
            let id = match ui.get_sessions().row_data(idx as usize) {
                Some(s) => s.to_string(),
                None => return,
            };
            nav.lock().unwrap().active_session_id = id.clone();
            nav.lock().unwrap().push("chat_session");
            sync_nav(&ui, &nav.lock().unwrap());
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
        let nav = nav.clone();
        ui.on_back_clicked(move || {
            if let Some(ui) = ui_weak.upgrade() {
                nav.lock().unwrap().pop();
                sync_nav(&ui, &nav.lock().unwrap());
                ui.set_transcript(ModelRc::new(VecModel::from(Vec::<SharedString>::new())));
            }
        });
    }

    // ---- tab switch ----
    {
        let ui_weak = ui.as_weak();
        let nav = nav.clone();
        ui.on_tab_clicked(move |tab| {
            if let Some(ui) = ui_weak.upgrade() {
                nav.lock().unwrap().tab = tab.to_string();
                sync_nav(&ui, &nav.lock().unwrap());
            }
        });
    }

    // ---- config sub-item ----
    {
        let ui_weak = ui.as_weak();
        let nav = nav.clone();
        let rt = rt.clone();
        ui.on_config_item_clicked(move |item| {
            let ui = match ui_weak.upgrade() {
                Some(u) => u,
                None => return,
            };
            let item = item.to_string();
            if item == "presets" {
                nav.lock().unwrap().push("preset_form_new");
                sync_nav(&ui, &nav.lock().unwrap());
                let (base, token) = (ui.get_base_url().to_string(), ui.get_token().to_string());
                let ui_weak = ui.as_weak();
                rt.spawn(async move {
                    let _ = agent::list_presets(&base, &token, "").await;
                    let _ = slint::invoke_from_event_loop(move || {
                        let _ = ui_weak.upgrade();
                    });
                });
            } else {
                nav.lock().unwrap().push(&format!("config_sub_{item}"));
                sync_nav(&ui, &nav.lock().unwrap());
            }
        });
    }

    // ---- providers ----
    {
        let ui_weak = ui.as_weak();
        let nav = nav.clone();
        let rt = rt.clone();
        ui.on_providers_clicked(move || {
            let ui = match ui_weak.upgrade() {
                Some(u) => u,
                None => return,
            };
            nav.lock().unwrap().push("providers_list");
            sync_nav(&ui, &nav.lock().unwrap());
            let (base, token) = (ui.get_base_url().to_string(), ui.get_token().to_string());
            let ui_weak = ui.as_weak();
            rt.spawn(async move {
                if let Ok(rows) = agent::list_providers(&base, &token).await {
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = ui_weak.upgrade() {
                            ui.set_providers(ModelRc::new(VecModel::from(
                                rows.into_iter().map(SharedString::from).collect::<Vec<_>>(),
                            )));
                        }
                    });
                }
            });
        });
    }

    // ---- mailbox overlay ----
    {
        let ui_weak = ui.as_weak();
        let nav = nav.clone();
        let rt = rt.clone();
        ui.on_mailbox_clicked(move || {
            let ui = match ui_weak.upgrade() {
                Some(u) => u,
                None => return,
            };
            nav.lock().unwrap().push("chat_overlay");
            sync_nav(&ui, &nav.lock().unwrap());
            let (base, token, sid) = (
                ui.get_base_url().to_string(),
                ui.get_token().to_string(),
                nav.lock().unwrap().active_session_id.clone(),
            );
            let ui_weak = ui.as_weak();
            rt.spawn(async move {
                if let Ok(rows) = agent::mailbox(&base, &token, &sid).await {
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = ui_weak.upgrade() {
                            ui.set_mailbox(ModelRc::new(VecModel::from(
                                rows.into_iter().map(SharedString::from).collect::<Vec<_>>(),
                            )));
                        }
                    });
                }
            });
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
