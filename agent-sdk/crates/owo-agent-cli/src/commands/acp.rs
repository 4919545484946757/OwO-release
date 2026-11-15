//! ACP stdio entry point backed by the existing OwO Daemon.

use clap::Args;
use owo_agent_client::AgentClient;
use owo_agent_protocol::{PermissionResponse, SseEvent};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::io::{BufRead, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::{AsyncWriteExt, Stdout};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use uuid::Uuid;

const MAX_LINE: u64 = 1024 * 1024;
const MAX_SESSIONS: usize = 32;
const MAX_PROMPT: usize = 128 * 1024;

#[derive(Args)]
pub(crate) struct AcpArgs {
    /// Project directory assigned by the host.
    #[arg(long)]
    workspace: PathBuf,
    /// Data directory of the owning OwO Daemon.
    #[arg(long)]
    data_dir: PathBuf,
    #[arg(long, hide = true)]
    owobot_worker: bool,
}

struct ActiveTurn {
    request_id: Value,
    session_id: String,
    turn_id: String,
    cancelled: bool,
    worker: JoinHandle<Result<(), &'static str>>,
}

enum Output {
    Update { session_id: String, update: Value },
    Finished,
}

pub(crate) async fn run(args: AcpArgs) -> Result<(), Box<dyn std::error::Error>> {
    let worker_proof: Option<Arc<str>> = if args.owobot_worker {
        let proof = std::env::var("OWOBOT_WORKER_PROOF")?;
        if proof.len() != 64 || !proof.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("OwOBot worker capability is invalid".into());
        }
        Some(proof.into())
    } else {
        None
    };
    let workspace = args.workspace.canonicalize()?;
    if !workspace.is_dir() || workspace.to_str().is_none() {
        return Err("ACP requires a UTF-8 workspace directory".into());
    }
    let client =
        owo_agent_client::connect(&args.data_dir, Some(owo_build_info::API_VERSION)).await?;
    let health = client.health().await?;
    if !health.healthy || health.auto_approve {
        return Err("ACP requires a ready Daemon with its approval policy enabled".into());
    }

    let mut input = input_lines();
    let mut output = tokio::io::stdout();
    let (turn_tx, mut turn_rx) = mpsc::channel(32);
    let mut sessions = HashSet::new();
    let mut initialized = false;
    let mut active: Option<ActiveTurn> = None;

    let result = async {
        loop {
            tokio::select! {
                item = input.recv() => {
                    let Some(item) = item else { break };
                    let Ok(request) = item else {
                        send(&mut output, error(Value::Null, -32700, "Invalid ACP frame")).await?;
                        continue;
                    };
                    let id = request.get("id").cloned().unwrap_or(Value::Null);
                    let method = request.get("method").and_then(Value::as_str);
                    let params = &request["params"];
                    if request["jsonrpc"] != "2.0" || method.is_none() {
                        send(&mut output, error(id, -32600, "Invalid ACP request")).await?;
                        continue;
                    }

                    if method == Some("session/cancel") && request.get("id").is_none() {
                        if let Some(turn) = &mut active {
                            if params["sessionId"] == turn.session_id && !turn.cancelled {
                                if let Ok(receipt) = client.cancel_turn_id(&turn.session_id, &turn.turn_id).await {
                                    turn.cancelled = matches!(
                                        receipt["state"].as_str(),
                                        Some("cancellation_requested" | "cancellation_queued")
                                    );
                                }
                            }
                        }
                        continue;
                    }
                    if request.get("id").is_none() {
                        continue;
                    }

                    let response = match method {
                        Some("initialize") if !initialized && params["protocolVersion"].is_u64() => {
                            initialized = true;
                            reply(id, json!({
                                "protocolVersion": 1,
                                "agentInfo": {"name": "owo-agent", "title": "OwO Agent", "version": env!("CARGO_PKG_VERSION")},
                                "agentCapabilities": {"loadSession": false}
                            }))
                        }
                        Some(_) if !initialized => error(id, -32000, "Initialize ACP first"),
                        Some("session/new") => {
                            let cwd = params["cwd"].as_str().map(Path::new);
                            if !cwd.is_some_and(|cwd| resolves_to_workspace(cwd, &workspace)) {
                                error(id, -32602, "ACP workspace differs from the host-assigned directory")
                            } else if sessions.len() >= MAX_SESSIONS {
                                error(id, -32000, "ACP session capacity exhausted")
                            } else {
                                match client.create_session(workspace.to_str().unwrap()).await {
                                    Ok(session) => {
                                        sessions.insert(session.id.clone());
                                        reply(id, json!({"sessionId": session.id}))
                                    }
                                    Err(_) => error(id, -32000, "OwO could not create the session"),
                                }
                            }
                        }
                        Some("session/prompt") if active.is_some() =>
                            error(id, -32000, "An OwO turn is already running"),
                        Some("session/prompt") => {
                            match prompt_params(params, &sessions) {
                                Ok((session_id, prompt)) => {
                                    let turn_id = Uuid::new_v4().to_string();
                                    let worker = tokio::spawn(run_turn(
                                        client.clone(), turn_tx.clone(), session_id.clone(),
                                        turn_id.clone(), prompt, worker_proof.clone(),
                                    ));
                                    active = Some(ActiveTurn {
                                        request_id: id, session_id, turn_id, cancelled: false, worker,
                                    });
                                    continue;
                                }
                                Err(message) => error(id, -32602, message),
                            }
                        }
                        _ => error(id, -32601, "ACP method unavailable"),
                    };
                    send(&mut output, response).await?;
                }
                Some(message) = turn_rx.recv(), if active.is_some() => {
                    match message {
                        Output::Update { session_id, update } => {
                            send(&mut output, json!({"jsonrpc":"2.0", "method":"session/update",
                                "params":{"sessionId":session_id, "update":update}})).await?;
                        }
                        Output::Finished => {
                            if let Some(turn) = active.take() {
                                let result = turn.worker.await;
                                let response = match result {
                                    Ok(_) if turn.cancelled => reply(turn.request_id, json!({"stopReason":"cancelled"})),
                                    Ok(Ok(())) => reply(turn.request_id, json!({"stopReason":"end_turn"})),
                                    Ok(Err(message)) => error(turn.request_id, -32000, message),
                                    Err(_) => error(turn.request_id, -32000, "OwO ACP worker stopped"),
                                };
                                send(&mut output, response).await?;
                            }
                        }
                    }
                }
            }
        }
        Ok::<_, std::io::Error>(())
    }.await;

    if let Some(turn) = active {
        let _ = client.cancel_turn_id(&turn.session_id, &turn.turn_id).await;
        turn.worker.abort();
    }
    result?;
    Ok(())
}

fn input_lines() -> mpsc::Receiver<Result<Value, ()>> {
    let (sender, receiver) = mpsc::channel(16);
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        let mut input = stdin.lock();
        let mut line = Vec::new();
        loop {
            line.clear();
            match input
                .by_ref()
                .take(MAX_LINE + 1)
                .read_until(b'\n', &mut line)
            {
                Ok(0) | Err(_) => break,
                Ok(size) if size as u64 > MAX_LINE => {
                    let _ = sender.blocking_send(Err(()));
                    break;
                }
                Ok(_) => {
                    if sender
                        .blocking_send(serde_json::from_slice(&line).map_err(|_| ()))
                        .is_err()
                    {
                        break;
                    }
                }
            }
        }
    });
    receiver
}

fn resolves_to_workspace(cwd: &Path, workspace: &Path) -> bool {
    cwd.is_absolute() && cwd.canonicalize().is_ok_and(|path| path == workspace)
}

fn prompt_params(
    params: &Value,
    sessions: &HashSet<String>,
) -> Result<(String, String), &'static str> {
    let id = params["sessionId"]
        .as_str()
        .filter(|id| sessions.contains(*id))
        .ok_or("Unknown ACP session")?;
    let parts = params["prompt"]
        .as_array()
        .ok_or("ACP prompt must contain content blocks")?;
    let mut prompt = String::new();
    for part in parts {
        let text = part["text"]
            .as_str()
            .filter(|_| part["type"] == "text")
            .ok_or("OwO ACP currently accepts text prompts")?;
        if prompt.len().saturating_add(text.len()).saturating_add(1) > MAX_PROMPT {
            return Err("ACP prompt exceeds the size limit");
        }
        if !prompt.is_empty() {
            prompt.push('\n');
        }
        prompt.push_str(text);
    }
    if prompt.trim().is_empty() {
        return Err("ACP prompt is empty");
    }
    Ok((id.to_owned(), prompt))
}

async fn run_turn(
    client: AgentClient,
    sender: mpsc::Sender<Output>,
    session_id: String,
    turn_id: String,
    prompt: String,
    worker_proof: Option<Arc<str>>,
) -> Result<(), &'static str> {
    let result = stream_turn(
        &client,
        &sender,
        &session_id,
        &turn_id,
        &prompt,
        worker_proof.as_deref(),
    )
    .await;
    if result.is_err() {
        let _ = client.cancel_turn_id(&session_id, &turn_id).await;
    }
    let _ = sender.send(Output::Finished).await;
    result
}

async fn stream_turn(
    client: &AgentClient,
    sender: &mpsc::Sender<Output>,
    session_id: &str,
    turn_id: &str,
    prompt: &str,
    worker_proof: Option<&str>,
) -> Result<(), &'static str> {
    let events = match worker_proof {
        Some(proof) => {
            client
                .open_owobot_turn(session_id, prompt, turn_id, proof)
                .await
        }
        None => {
            client
                .open_turn_with_id(session_id, prompt, true, turn_id)
                .await
        }
    };
    let mut events = events.map_err(|_| "OwO could not start the read-only turn")?;
    let mut streamed = false;
    let mut final_message = false;
    let mut completed = false;
    while let Some(event) = events.next_event().await {
        let update = match event.map_err(|_| "OwO turn stream interrupted")? {
            SseEvent::TokenDelta { delta } if !delta.is_empty() => {
                streamed = true;
                Some(text_update(delta))
            }
            SseEvent::Final { text } => {
                final_message = true;
                (!streamed && !text.is_empty()).then(|| text_update(text))
            }
            SseEvent::ToolUse { id, tool, .. } => Some(json!({
                "sessionUpdate":"tool_call", "toolCallId":id, "title":tool, "status":"in_progress"
            })),
            SseEvent::ToolResult { id, ok, .. } => Some(json!({
                "sessionUpdate":"tool_call_update", "toolCallId":id,
                "status": if ok {"completed"} else {"failed"}
            })),
            SseEvent::PermissionRequest { request_id, .. } => {
                client
                    .respond_permission(
                        session_id,
                        &request_id,
                        &PermissionResponse {
                            allow: false,
                            remember: None,
                            scope: None,
                        },
                    )
                    .await
                    .map_err(|_| "OwO could not resolve the permission request")?;
                None
            }
            SseEvent::UserQuestion { .. } => return Err("OwO needs an answer from the user"),
            SseEvent::TurnFailed { .. } => return Err("OwO turn failed or was stopped"),
            SseEvent::TurnStats { .. } => {
                completed = true;
                None
            }
            _ => None,
        };
        if let Some(update) = update {
            sender
                .send(Output::Update {
                    session_id: session_id.to_owned(),
                    update,
                })
                .await
                .map_err(|_| "ACP stream closed")?;
        }
    }
    if final_message && completed {
        Ok(())
    } else {
        Err("OwO turn has no verified terminal event")
    }
}

fn text_update(text: String) -> Value {
    json!({"sessionUpdate":"agent_message_chunk", "content":{"type":"text","text":text}})
}

fn reply(id: Value, result: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"result":result})
}

fn error(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}

async fn send(output: &mut Stdout, value: Value) -> std::io::Result<()> {
    let mut frame = serde_json::to_vec(&value).map_err(std::io::Error::other)?;
    frame.push(b'\n');
    output.write_all(&frame).await?;
    output.flush().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_stays_with_a_session_created_by_this_process() {
        let sessions = HashSet::from(["owned".to_owned()]);
        let params = json!({"sessionId":"other","prompt":[{"type":"text","text":"task"}]});
        assert!(prompt_params(&params, &sessions).is_err());
        let params = json!({"sessionId":"owned","prompt":[{"type":"text","text":"整理"},{"type":"text","text":"名单"}]});
        assert_eq!(prompt_params(&params, &sessions).unwrap().1, "整理\n名单");
        let params = json!({"sessionId":"owned","prompt":[{"type":"image","data":"abc"}]});
        assert!(prompt_params(&params, &sessions).is_err());
    }
}
