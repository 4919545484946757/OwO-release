//! 会话域便捷方法（`AgentClient` 的 inherent impl）。

use crate::error::Result;
use crate::http::AgentClient;
use owo_agent_protocol::{CreateSessionRequest, FileDiff, SessionInfo};

impl AgentClient {
    /// 新建会话（workspace 必填）。
    pub async fn create_session(&self, workspace: &str) -> Result<SessionInfo> {
        self.create_session_with_model(workspace, None).await
    }

    /// 新建会话并指定模型（`None` = 由 Daemon 用工作区/默认设置决定）。
    pub async fn create_session_with_model(
        &self,
        workspace: &str,
        model: Option<String>,
    ) -> Result<SessionInfo> {
        let request = CreateSessionRequest {
            workspace: workspace.to_string(),
            model,
            system_prompt: None,
            mode: None,
        };
        self.post_json("/session", &request).await
    }

    /// 新建纯对话会话（只给联网检索工具，不开放本地文件与命令）。
    ///
    /// `workspace = None` 时不传工作区，由服务端分配隔离目录（`data/chat`）：
    /// 会话可以立即使用，但文件工具永远碰不到用户项目。
    pub async fn create_chat_session(&self, workspace: Option<&str>) -> Result<SessionInfo> {
        let request = CreateSessionRequest {
            workspace: workspace.unwrap_or_default().to_string(),
            model: None,
            system_prompt: None,
            mode: Some("chat".to_string()),
        };
        self.post_json("/session", &request).await
    }

    /// 切换会话工作模式（`agent` / `chat`）。
    pub async fn set_session_mode(&self, id: &str, mode: &str) -> Result<serde_json::Value> {
        self.post_json(
            &format!("/session/{id}/mode"),
            &serde_json::json!({ "mode": mode }),
        )
        .await
    }

    pub async fn list_sessions(&self) -> Result<Vec<SessionInfo>> {
        self.get_json("/sessions").await
    }

    pub async fn get_session(&self, id: &str) -> Result<SessionInfo> {
        self.get_json(&format!("/session/{id}")).await
    }

    /// 本次会话的文件改动（diff/revert 面）。
    pub async fn session_diff(&self, id: &str) -> Result<Vec<FileDiff>> {
        self.get_json(&format!("/session/{id}/diff")).await
    }

    /// 会话级模型覆盖（`None` = 清除覆盖回退默认解析链）。
    pub async fn session_set_model(
        &self,
        id: &str,
        model: Option<&str>,
    ) -> Result<serde_json::Value> {
        self.post_json(
            &format!("/session/{id}/model"),
            &serde_json::json!({ "model": model }),
        )
        .await
    }

    /// 回滚本次会话全部写操作。
    pub async fn session_revert(&self, id: &str) -> Result<serde_json::Value> {
        self.post_empty(&format!("/session/{id}/revert")).await
    }

    /// 回退会话历史到保留 `keep` 条消息。
    pub async fn session_rewind(&self, id: &str, keep: usize) -> Result<serde_json::Value> {
        self.post_json(
            &format!("/session/{id}/rewind"),
            &serde_json::json!({ "keep": keep }),
        )
        .await
    }

    /// 恢复最近一次 rewind。
    pub async fn session_redo(&self, id: &str) -> Result<serde_json::Value> {
        self.post_empty(&format!("/session/{id}/redo")).await
    }

    /// 在指定消息处创建子会话。
    pub async fn session_fork(&self, id: &str, message_index: usize) -> Result<SessionInfo> {
        self.post_json(
            &format!("/session/{id}/fork"),
            &serde_json::json!({ "message_index": message_index }),
        )
        .await
    }

    pub async fn session_rename(&self, id: &str, title: &str) -> Result<SessionInfo> {
        self.post_json(
            &format!("/session/{id}/rename"),
            &serde_json::json!({ "title": title }),
        )
        .await
    }

    pub async fn session_archive(&self, id: &str, archived: bool) -> Result<serde_json::Value> {
        self.post_json(
            &format!("/session/{id}/archive"),
            &serde_json::json!({ "archived": archived }),
        )
        .await
    }

    pub async fn session_pin(&self, id: &str, pinned: bool) -> Result<serde_json::Value> {
        self.post_json(
            &format!("/session/{id}/pin"),
            &serde_json::json!({ "pinned": pinned }),
        )
        .await
    }
}
