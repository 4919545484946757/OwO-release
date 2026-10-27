//! 「按声明范围写入」的预授权规则（用户自定义工作流）。
//!
//! 背景（用户实测）：自定义工作流的写节点在团队运行里被写权限拒绝，而审批请求
//! **不会出现在 `/approvals/pending`**（无人值守的团队运行没有交互式审批通道），
//! 于是团队必然失败。工作流面板早就有一个「允许节点按声明的范围写入当前工作区」
//! 复选框，但此前没有接到任何服务端语义。
//!
//! 这里把那个承诺落成规则：建队时显式勾选（`allow_scoped_writes`）后，**只有**
//! 落在该角色 `write_paths`（与写租约/变更追踪同一份白名单）内的写操作被放行；
//! 越界写、命令执行、联网等一律回落给原有审批链（仍然 deny/询问）。这不是
//! 「放开权限」——权限矩阵、工具面与写租约三层约束不变，只是把"工作区内已声明
//! 范围的写入"从"没人能点的审批"变成"建队时的一次显式授权"。

use std::path::{Path, PathBuf};

use owo_agent_core::permissions::{ApprovalOutcome, Decision, PermissionRequest};

use super::workspace_change_tracker::{path_in_allowed, simplify_path};

#[derive(Debug, Clone)]
pub(crate) struct ScopedWriteRule {
    /// 建队时是否显式勾选"允许按声明范围写入"。
    allow: bool,
    /// 该节点的写范围（绝对路径；与写租约/变更追踪同一份）。
    scopes: Vec<PathBuf>,
    /// 工作区根（相对路径解析基准）。
    root: PathBuf,
}

impl ScopedWriteRule {
    pub(crate) fn new(allow: bool, scopes: Vec<PathBuf>, root: PathBuf) -> Self {
        Self {
            allow,
            scopes: scopes.iter().map(|path| simplify_path(path)).collect(),
            root: simplify_path(&root),
        }
    }

    /// 这次请求是否属于"范围内写入"：写类工具 + 目标路径落在声明范围内。
    ///
    /// `apply_patch` 的参数是补丁正文而不是单个 `path`；必须解析每个
    /// `*** Add/Update/Delete File:` 头，**全部**落在声明范围内才预授权。
    /// 否则 worker 按提示优先用 apply_patch 时会被人审批拒绝，退回整文件
    /// write_file（实测浪费一轮并放大上下文）。
    pub(crate) fn is_scoped_write(&self, request: &PermissionRequest) -> bool {
        if !self.allow || self.scopes.is_empty() {
            return false;
        }
        if !matches!(
            request.tool.as_str(),
            "write_file" | "edit_file" | "multi_edit" | "apply_patch"
        ) {
            return false;
        }
        let paths: Vec<&str> = if request.tool == "apply_patch" {
            apply_patch_paths(&request.args)
        } else {
            request
                .args
                .get("path")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .into_iter()
                .collect()
        };
        !paths.is_empty() && paths.iter().all(|raw| self.path_is_scoped(raw))
    }

    fn path_is_scoped(&self, raw: &str) -> bool {
        let candidate = Path::new(raw);
        let absolute = if candidate.is_absolute() {
            simplify_path(candidate)
        } else {
            simplify_path(&self.root.join(candidate))
        };
        // 必须是工作区内的相对路径（越界写不预授权），且落在声明范围内。
        let Ok(relative) = absolute.strip_prefix(&self.root) else {
            return false;
        };
        let relative = relative.to_string_lossy().replace('\\', "/");
        !relative.is_empty() && path_in_allowed(&relative, &self.root, &self.scopes)
    }

    /// 用本次任务**实际生效**的写范围派生一份规则（角色范围 ∩ 绑定 ∩ 任务分配）。
    ///
    /// 任务图给每个节点再收窄一次写范围时，预授权必须跟着收窄，否则会出现
    /// "任务范围外也放行"的越权。
    pub(crate) fn for_scope(&self, scopes: &[PathBuf], root: &Path) -> Self {
        Self {
            allow: self.allow,
            scopes: scopes.iter().map(|path| simplify_path(path)).collect(),
            root: simplify_path(root),
        }
    }

    /// 预授权的决策与原因（审计/对话里可复核"凭什么放行"）。
    pub(crate) fn authorize(&self) -> (Decision, ApprovalOutcome) {
        (
            Decision::Allow,
            ApprovalOutcome {
                decision: Decision::Allow,
                reason: Some("approval/scoped_write_authorized".to_string()),
            },
        )
    }
}

/// Codex 风格补丁中所有文件操作的路径（与 core `parse_patch` 的头格式一致）。
fn apply_patch_paths(args: &serde_json::Value) -> Vec<&str> {
    let Some(patch) = args.get("patch").and_then(|value| value.as_str()) else {
        return Vec::new();
    };
    patch
        .lines()
        .filter_map(|line| {
            line.strip_prefix("*** Add File: ")
                .or_else(|| line.strip_prefix("*** Update File: "))
                .or_else(|| line.strip_prefix("*** Delete File: "))
        })
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use owo_agent_core::permissions::Level;
    use serde_json::json;

    fn rule(allow: bool, root: &Path, scopes: &[&str]) -> ScopedWriteRule {
        ScopedWriteRule::new(
            allow,
            scopes.iter().map(|s| root.join(s)).collect(),
            root.to_path_buf(),
        )
    }

    fn request(tool: &str, path: &str) -> PermissionRequest {
        PermissionRequest::new(tool, json!({ "path": path }), Level::Write, "写入文件")
    }

    fn patch_request(patch: &str) -> PermissionRequest {
        PermissionRequest::new(
            "apply_patch",
            json!({ "patch": patch }),
            Level::Write,
            "应用补丁",
        )
    }

    #[test]
    fn authorized_scoped_writes_only_cover_declared_paths() {
        let root = PathBuf::from(r"C:\ws");
        let scoped = rule(true, &root, &["outputs"]);

        assert!(scoped.is_scoped_write(&request("write_file", "outputs/a.md")));
        assert!(scoped.is_scoped_write(&patch_request(
            "*** Begin Patch\n*** Update File: outputs/deep/b.txt\n@@\n-a\n+b\n*** End Patch"
        )));
        assert!(
            !scoped.is_scoped_write(&patch_request(
                "*** Begin Patch\n*** Update File: outputs/a.txt\n@@\n-a\n+b\n\
                 *** Update File: src/escape.txt\n@@\n-a\n+b\n*** End Patch"
            )),
            "补丁只要有一个文件越界，整包不预授权"
        );
        assert!(
            !scoped.is_scoped_write(&patch_request("*** Begin Patch\n*** End Patch")),
            "没有文件头的补丁不预授权"
        );
        assert!(
            !scoped.is_scoped_write(&request("write_file", "src/a.md")),
            "范围外不预授权"
        );
        assert!(
            !scoped.is_scoped_write(&request("write_file", "../escape.md")),
            "越界不预授权"
        );
        assert!(
            !scoped.is_scoped_write(&request("run_command", "outputs/a.md")),
            "命令不预授权"
        );
        assert!(
            !scoped.is_scoped_write(&request("mcp__x__y", "outputs/a.md")),
            "MCP 不预授权"
        );

        // 未勾选 = 一切走原审批链（保持默认 deny/询问）。
        let gated = rule(false, &root, &["outputs"]);
        assert!(!gated.is_scoped_write(&request("write_file", "outputs/a.md")));
        // 没声明写范围的节点：不预授权。
        let unscoped = rule(true, &root, &[]);
        assert!(!unscoped.is_scoped_write(&request("write_file", "outputs/a.md")));
    }

    #[test]
    fn task_scope_narrowing_also_narrows_the_preauthorization() {
        let root = PathBuf::from(r"C:\ws");
        let role = rule(true, &root, &["outputs"]);
        // 任务只分配了 outputs/images → outputs/a.md 之外不再预授权。
        let task = role.for_scope(&[root.join("outputs/images")], &root);
        assert!(task.is_scoped_write(&request("write_file", "outputs/images/a.png")));
        assert!(!task.is_scoped_write(&request("write_file", "outputs/other.md")));
    }

    #[test]
    fn authorization_reason_is_reviewable() {
        let root = PathBuf::from(r"C:\ws");
        let scoped = rule(true, &root, &["outputs"]);
        let (decision, outcome) = scoped.authorize();
        assert_eq!(decision, Decision::Allow);
        assert_eq!(
            outcome.reason.as_deref(),
            Some("approval/scoped_write_authorized")
        );
    }
}
