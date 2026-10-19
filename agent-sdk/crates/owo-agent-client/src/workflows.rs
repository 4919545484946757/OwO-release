//! 用户自定义工作流（团队模板）与 AI 草稿的客户端调用。
//!
//! 与 `sessions.rs`/`mcp.rs` 分离：工作流域自身有 CRUD、草稿、社区包三段语义，
//! 由 CLI `owo-agent workflow` 与桌面面板共用同一组方法。

use crate::error::Result;
use crate::AgentClient;

impl AgentClient {
    /// 用户/社区工作流列表（含逐条校验结果）。
    pub async fn list_user_workflows(&self) -> Result<serde_json::Value> {
        self.get_json("/workflows/user").await
    }

    /// 某工作流最近一次运行（含可复用的 objective；从未运行 → `last_run` 为 null）。
    pub async fn workflow_last_run(&self, id: &str) -> Result<serde_json::Value> {
        self.get_json(&format!("/workflows/user/{id}/last-run"))
            .await
    }

    /// 创建/更新用户工作流（服务端先校验再落盘；非法 → 422）。
    pub async fn upsert_user_workflow(
        &self,
        template: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        self.post_json("/workflows/user", template).await
    }

    /// 删除用户/社区工作流（内置 403、不存在 404）。
    pub async fn delete_user_workflow(&self, id: &str) -> Result<serde_json::Value> {
        self.delete_json(&format!("/workflows/user/{id}")).await
    }

    /// AI 生成工作流草稿（自然语言 → 候选；可基于既有草稿迭代）。
    pub async fn create_workflow_draft(
        &self,
        description: &str,
        constraints: Option<&str>,
        refine_of: Option<&str>,
        based_on: Option<&str>,
    ) -> Result<serde_json::Value> {
        self.post_json(
            "/workflows/draft",
            &serde_json::json!({
                "description": description,
                "constraints": constraints,
                "refine_of": refine_of,
                // 基于已保存工作流迭代：服务端注入它的真实运行证据。
                "based_on": based_on,
            }),
        )
        .await
    }

    /// 有界多轮收敛生成（生成 → 校验 → 按错误重生成，直到通过或轮次用尽）。
    pub async fn converge_workflow_draft(
        &self,
        description: &str,
        constraints: Option<&str>,
        max_rounds: Option<usize>,
        based_on: Option<&str>,
    ) -> Result<serde_json::Value> {
        self.post_json(
            "/workflows/draft/converge",
            &serde_json::json!({
                "description": description,
                "constraints": constraints,
                "max_rounds": max_rounds,
                "based_on": based_on,
            }),
        )
        .await
    }

    /// 工作流使用统计（运行次数与成败聚合；只读）。
    pub async fn workflow_stats(&self) -> Result<serde_json::Value> {
        self.get_json("/workflows/stats").await
    }

    /// 草稿列表。
    pub async fn list_workflow_drafts(&self) -> Result<serde_json::Value> {
        self.get_json("/workflows/draft").await
    }

    /// 草稿详情（含模板与预览）。
    pub async fn get_workflow_draft(&self, id: &str) -> Result<serde_json::Value> {
        self.get_json(&format!("/workflows/draft/{id}")).await
    }

    /// 回写"采纳后验证运行"的结果（下一次基于同一工作流迭代时会作为反馈）。
    pub async fn record_workflow_draft_verify(
        &self,
        draft_id: &str,
        team_id: &str,
        objective: Option<&str>,
        compared_status: Option<&str>,
    ) -> Result<serde_json::Value> {
        self.post_json(
            &format!("/workflows/draft/{draft_id}/verify"),
            &serde_json::json!({
                "team_id": team_id,
                "objective": objective,
                "compared_status": compared_status,
            }),
        )
        .await
    }

    /// 采纳草稿 → 用户模板。
    pub async fn adopt_workflow_draft(&self, id: &str) -> Result<serde_json::Value> {
        self.post_json(
            &format!("/workflows/draft/{id}/adopt"),
            &serde_json::json!({}),
        )
        .await
    }

    /// 丢弃草稿。
    pub async fn discard_workflow_draft(&self, id: &str) -> Result<serde_json::Value> {
        self.delete_json(&format!("/workflows/draft/{id}")).await
    }

    /// 导出社区包（base64 + manifest + sha256 + 预审 findings）。
    pub async fn export_user_workflow(&self, id: &str) -> Result<serde_json::Value> {
        self.post_json(
            &format!("/workflows/user/{id}/export"),
            &serde_json::json!({}),
        )
        .await
    }

    /// 导入前评审（不落盘）。
    pub async fn review_workflow_import(&self, package_b64: &str) -> Result<serde_json::Value> {
        self.post_json(
            "/workflows/import/review",
            &serde_json::json!({ "package_b64": package_b64 }),
        )
        .await
    }

    /// 导出"我的社区来源清单"（template_id + sha256 + 导入时间；不含配置正文）。
    pub async fn export_workflow_provenance(&self) -> Result<serde_json::Value> {
        self.get_json("/workflows/provenance").await
    }

    /// 导入社区包（评审通过才落盘，origin=community）。
    pub async fn import_user_workflow(&self, package_b64: &str) -> Result<serde_json::Value> {
        self.post_json(
            "/workflows/import",
            &serde_json::json!({ "package_b64": package_b64 }),
        )
        .await
    }
}
