use super::{delivery, GatewayError, GatewayResult, ModelSelection, WorkerRole};
use crate::auth_token::AuthToken;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use reqwest::Client;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use uuid::Uuid;

const START_LIMIT: Duration = Duration::from_secs(40);
const HTTP_LIMIT: Duration = Duration::from_secs(12);
const GROUP_NAME: &str = "项目协作";
const INSTANCE: &str = "owo-agent";
const CUSTOM_BOT_SECTION: &str = "owobot-custom";

pub(super) struct Running {
    pub(super) base_url: String,
    pub(super) session: String,
    pub(super) group: String,
    members: [String; 3],
    pub(super) workspace: PathBuf,
    pub(super) worker_proof: AuthToken,
    pub(super) model: ModelSelection,
    pub(super) delivery: delivery::Store,
    child: Child,
    stdin: ChildStdin,
}

impl Running {
    pub(super) fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    pub(super) fn member(&self, role: WorkerRole) -> &str {
        &self.members[role as usize]
    }

    pub(super) fn role_for(&self, id: &str) -> Option<WorkerRole> {
        self.members
            .iter()
            .position(|member| member == id)
            .map(|index| WorkerRole::ALL[index])
    }

    pub(super) async fn stop(mut self) {
        let _ = self.stdin.shutdown().await;
        drop(self.stdin);
        if tokio::time::timeout(Duration::from_secs(4), self.child.wait())
            .await
            .is_err()
        {
            let _ = self.child.kill().await;
        }
    }
}

#[derive(Deserialize)]
struct Ready {
    event: String,
    port: u16,
}

#[derive(Deserialize)]
struct Pairing {
    code: String,
}

#[derive(Deserialize)]
struct Session {
    token: String,
}

#[derive(Deserialize)]
struct Bot {
    id: String,
    name: String,
    section: Option<String>,
}

#[derive(Deserialize)]
struct Group {
    id: String,
    name: String,
    cwd: Option<String>,
    #[serde(rename = "memberIds")]
    members: Vec<String>,
}

#[derive(Deserialize)]
struct Snapshot {
    bots: Vec<Bot>,
    groups: Vec<Group>,
}

#[derive(Deserialize)]
struct CreatedBot {
    bot: Bot,
}

#[derive(Deserialize)]
struct CreatedGroup {
    group: Group,
}

pub(super) async fn start(
    client: &Client,
    home: &Path,
    data_root: &Path,
    workspace: &Path,
    delivery_gate: Arc<Mutex<()>>,
) -> GatewayResult<Running> {
    tokio::time::timeout(
        START_LIMIT,
        launch(client, home, data_root, workspace, delivery_gate),
    )
    .await
    .map_err(|_| GatewayError::Unavailable("OwOBot 启动超时"))?
}

async fn launch(
    client: &Client,
    home: &Path,
    data_root: &Path,
    workspace: &Path,
    delivery_gate: Arc<Mutex<()>>,
) -> GatewayResult<Running> {
    let workspace = workspace
        .canonicalize()
        .map_err(|_| GatewayError::Unavailable("OwOBot 无法打开当前项目目录"))?;
    if !workspace.is_dir() {
        return Err(GatewayError::Unavailable("OwOBot 项目目录不存在"));
    }
    let workspace_text = utf8_path(&workspace)?;
    let project_key = format!("{:x}", Sha256::digest(workspace_text.as_bytes()));
    let state_home = data_root.join("owobot").join(project_key);
    let bot_home = state_home.join("home");
    tokio::fs::create_dir_all(&bot_home)
        .await
        .map_err(storage_error)?;
    write_config(&state_home, data_root, &workspace).await?;

    let mut seed = [0_u8; 32];
    seed[..16].copy_from_slice(Uuid::new_v4().as_bytes());
    seed[16..].copy_from_slice(Uuid::new_v4().as_bytes());
    let secret = URL_SAFE_NO_PAD.encode(seed);
    let worker_proof = AuthToken::generate();
    let mut command = Command::new(home.join("node.exe"));
    command
        .arg(home.join("dist-server").join("index.js"))
        .current_dir(home)
        .env_clear()
        .envs(process_environment())
        .env("OWOBOT_MANAGED", "1")
        .env("OWOBOT_WORKER_PROOF", worker_proof.token())
        .env("OMB_LOOPBACK_TRUST", "service")
        .env("OMB_CLI_OWNER_STDIN", "1")
        .env("OMB_DISABLE_ANNOUNCEMENTS", "1")
        .env("OMB_PORT", "0")
        .env("OMB_DATA_DIR", &state_home)
        .env("HOME", &bot_home)
        .env("USERPROFILE", &bot_home)
        .env("LOCALAPPDATA", &bot_home)
        .env("APPDATA", &bot_home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000);
    let mut child = command
        .spawn()
        .map_err(|_| GatewayError::Unavailable("无法启动 OwOBot 本机服务"))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or(GatewayError::Unavailable("OwOBot 启动通道不可用"))?;
    stdin
        .write_all(format!("{secret}\n").as_bytes())
        .await
        .map_err(|_| GatewayError::Unavailable("OwOBot 启动通道已关闭"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or(GatewayError::Unavailable("OwOBot 就绪通道不可用"))?;
    let mut output = BufReader::new(stdout).lines();
    let port = loop {
        let line = output
            .next_line()
            .await
            .map_err(|_| GatewayError::Unavailable("无法读取 OwOBot 就绪状态"))?
            .ok_or(GatewayError::Unavailable("OwOBot 在就绪前退出"))?;
        if let Ok(Ready { event, port }) = serde_json::from_str::<Ready>(&line) {
            if event == "owobot_ready" && port != 0 {
                break port;
            }
        }
    };
    tokio::spawn(async move { while matches!(output.next_line().await, Ok(Some(_))) {} });
    let base_url = format!("http://127.0.0.1:{port}");
    let opened: Pairing = answer(
        client
            .post(format!("{base_url}/api/auth/pairing"))
            .header("x-openmausbot-cli-owner", secret)
            .json(&json!({"scopes": ["admin", "client"]})),
    )
    .await?;
    let session: Session = answer(
        client
            .post(format!("{base_url}/api/auth/pair"))
            .json(&json!({"code": opened.code, "label": "OwO Daemon"})),
    )
    .await?;
    let (group, members) = prepare_group(client, &base_url, &session.token, workspace_text).await?;
    let delivery =
        delivery::Store::new(&state_home, workspace.clone(), group.clone(), delivery_gate)
            .map_err(|_| GatewayError::Unavailable("OwOBot 交付存储无法打开"))?;
    Ok(Running {
        base_url,
        session: session.token,
        group,
        members,
        workspace,
        worker_proof,
        model: ModelSelection::default(),
        delivery,
        child,
        stdin,
    })
}

fn process_environment() -> impl Iterator<Item = (std::ffi::OsString, std::ffi::OsString)> {
    const KEYS: &[&str] = &[
        "SystemRoot",
        "WINDIR",
        "PATH",
        "PATHEXT",
        "ComSpec",
        "TEMP",
        "TMP",
        "ProgramFiles",
        "ProgramFiles(x86)",
        "ProgramData",
        "NUMBER_OF_PROCESSORS",
    ];
    KEYS.iter()
        .filter_map(|key| std::env::var_os(key).map(|value| (std::ffi::OsString::from(key), value)))
}

fn utf8_path(path: &Path) -> GatewayResult<&str> {
    path.to_str()
        .filter(|text| !text.is_empty() && !text.contains(['"', '\r', '\n']))
        .ok_or(GatewayError::Unavailable("OwOBot 路径包含无法传递的字符"))
}

fn storage_error(_: std::io::Error) -> GatewayError {
    GatewayError::Unavailable("无法保存 OwOBot 项目数据")
}

async fn write_config(state_home: &Path, data_root: &Path, workspace: &Path) -> GatewayResult<()> {
    let agent = std::env::current_exe().map_err(storage_error)?;
    let data_root: PathBuf = data_root.canonicalize().map_err(storage_error)?;
    let cli = format!(
        "\"{}\" acp --owobot-worker --workspace \"{}\" --data-dir \"{}\"",
        utf8_path(&agent)?,
        utf8_path(workspace)?,
        utf8_path(&data_root)?,
    );
    let cfg = json!({
        "language": "zh",
        "instances": {INSTANCE: {"driver":"customAcp", "displayName":"OwO Agent", "config":{"cli":cli}}},
        "defaultModelSelection": {"instanceId": INSTANCE, "model":"agent-default"},
    });
    tokio::fs::write(state_home.join("config.json"), cfg.to_string())
        .await
        .map_err(storage_error)
}

async fn prepare_group(
    client: &Client,
    base_url: &str,
    session: &str,
    workspace: &str,
) -> GatewayResult<(String, [String; 3])> {
    let snapshot: Snapshot = answer(
        client
            .get(format!("{base_url}/api/bots"))
            .bearer_auth(session),
    )
    .await?;
    let group = snapshot
        .groups
        .iter()
        .find(|group| group.name == GROUP_NAME);
    let mut members = Vec::with_capacity(3);
    for role in WorkerRole::ALL {
        let name = role.name();
        let mut existing = snapshot.bots.iter().filter(|bot| bot.name == name);
        if let Some(bot) = existing.next() {
            if existing.next().is_some() {
                return Err(GatewayError::Unavailable("OwOBot 项目角色重复"));
            }
            members.push(bot.id.clone());
            continue;
        }
        if group.is_some() {
            return Err(GatewayError::Unavailable("OwOBot 项目缺少角色"));
        }
        let created: CreatedBot = answer(
            client
                .post(format!("{base_url}/api/bots"))
                .bearer_auth(session)
                .json(&json!({
                    "name": name,
                    "soul": role.instructions(),
                    "cwd": workspace,
                    "computer": "none",
                    "approvalMode":"ask",
                    "useDefaults":false,
                    "modelSelection":{"instanceId":INSTANCE, "model":"agent-default"}
                })),
        )
        .await?;
        members.push(created.bot.id);
    }
    let members: [String; 3] = members.try_into().expect("exactly three roles");
    if let Some(group) = group {
        if !members.iter().all(|id| group.members.contains(id))
            || group.members.iter().any(|id| {
                !members.contains(id)
                    && !snapshot.bots.iter().any(|bot| {
                        bot.id == *id && bot.section.as_deref() == Some(CUSTOM_BOT_SECTION)
                    })
            })
        {
            return Err(GatewayError::Unavailable("OwOBot 项目成员与角色不匹配"));
        }
        if group.cwd.as_deref() != Some(workspace) {
            pin_group_folder(client, base_url, session, &group.id, workspace).await?;
        }
        return Ok((group.id.clone(), members));
    }
    let group: CreatedGroup = answer(
        client
            .post(format!("{base_url}/api/groups"))
            .bearer_auth(session)
            .json(&json!({
                "name":GROUP_NAME,
                "memberIds":members,
                "setup":{"bulletin":"使用 @ 点名指定成员。", "defaultResponder":{"kind":"member","botId":members[0]}}
            })),
    )
    .await?;
    pin_group_folder(client, base_url, session, &group.group.id, workspace).await?;
    Ok((group.group.id, members))
}

async fn pin_group_folder(
    client: &Client,
    base_url: &str,
    session: &str,
    group: &str,
    workspace: &str,
) -> GatewayResult<()> {
    let _: Value = answer(
        client
            .patch(format!("{base_url}/api/groups/{group}"))
            .bearer_auth(session)
            .json(&json!({"cwd": workspace})),
    )
    .await?;
    Ok(())
}

async fn answer<T: DeserializeOwned>(request: reqwest::RequestBuilder) -> GatewayResult<T> {
    request
        .timeout(HTTP_LIMIT)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|_| GatewayError::Unavailable("OwOBot 本机服务拒绝了初始化请求"))?
        .json()
        .await
        .map_err(|_| GatewayError::Unavailable("OwOBot 返回了无效的初始化数据"))
}
