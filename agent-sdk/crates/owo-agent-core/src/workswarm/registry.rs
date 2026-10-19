use owo_agent_protocol::{
    TeamMode, TeamTemplate, TeamTemplateProposal, TeamTemplateProposalStatus,
};
use std::path::PathBuf;
/// 团队模板注册表：`templates/`（已采纳）+ `proposals/`（提案，只提案不自动启用）。
#[derive(Debug, Clone)]
pub struct TeamTemplateRegistry {
    dir: PathBuf,
}

impl TeamTemplateRegistry {
    pub fn new(root: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(root.join("templates"));
        let _ = std::fs::create_dir_all(root.join("proposals"));
        Self { dir: root }
    }

    // -- 模板 --

    pub fn save_template(&self, t: &TeamTemplate) -> std::io::Result<()> {
        let dir = self.dir.join("templates");
        std::fs::create_dir_all(&dir)?;
        std::fs::write(
            dir.join(format!("{}.json", t.template_id)),
            serde_json::to_string_pretty(t)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?,
        )
    }

    pub fn get_template(&self, id: &str) -> Option<TeamTemplate> {
        let path = self.dir.join("templates").join(format!("{id}.json"));
        let raw = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&raw).ok()
    }

    pub fn list_templates(&self) -> Vec<TeamTemplate> {
        let dir = self.dir.join("templates");
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) == Some("json") {
                if let Ok(raw) = std::fs::read_to_string(p) {
                    if let Ok(t) = serde_json::from_str::<TeamTemplate>(&raw) {
                        out.push(t);
                    }
                }
            }
        }
        out.sort_by(|a, b| a.template_id.cmp(&b.template_id));
        out
    }

    /// 模板优先匹配（§6.1）：同形态模板中，适用条件任一关键词命中 objective 即命中； S0 采用简单子串启发（关键词 = applicability 按分隔符切段，长度 ≥2）。
    pub fn find_match(&self, mode: TeamMode, objective: &str) -> Option<TeamTemplate> {
        let objective_lower = objective.to_lowercase();
        let templates = self.list_templates();
        let mut matched: Vec<&TeamTemplate> = templates
            .iter()
            .filter(|t| t.mode == mode)
            .filter(|t| {
                let tokens: Vec<String> = t
                    .applicability
                    .split([' ', '，', ',', '、', '/', '\n', '\t'])
                    .map(|s| s.to_string())
                    .filter(|s| s.chars().count() >= 2)
                    .collect();
                tokens
                    .iter()
                    .any(|tok| objective_lower.contains(&tok.to_lowercase()))
            })
            .collect();
        matched.sort_by(|a, b| b.created_at.cmp(&a.created_at)); // 最近创建的优先
        matched.into_iter().next().cloned()
    }

    // -- 提案（只提案，不自动启用） --

    pub fn save_proposal(&self, p: &TeamTemplateProposal) -> std::io::Result<()> {
        let dir = self.dir.join("proposals");
        std::fs::create_dir_all(&dir)?;
        std::fs::write(
            dir.join(format!("{}.json", p.proposal_id)),
            serde_json::to_string_pretty(p)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?,
        )
    }

    pub fn get_proposal(&self, id: &str) -> Option<TeamTemplateProposal> {
        let path = self.dir.join("proposals").join(format!("{id}.json"));
        let raw = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&raw).ok()
    }

    pub fn list_proposals(&self) -> Vec<TeamTemplateProposal> {
        let dir = self.dir.join("proposals");
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) == Some("json") {
                if let Ok(raw) = std::fs::read_to_string(p) {
                    if let Ok(t) = serde_json::from_str::<TeamTemplateProposal>(&raw) {
                        out.push(t);
                    }
                }
            }
        }
        out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        out
    }

    /// 删除模板：返回 `Ok(true)` 表示确实删掉了一个（`Ok(false)` = 本来就不存在）。
    ///
    /// 只负责文件删除；"内置模板不得删除"的策略由调用方（HTTP/CLI）判定——
    /// 注册表本身无法区分来源，`origin` 是模板数据的一部分。
    pub fn delete_template(&self, id: &str) -> std::io::Result<bool> {
        // 防穿越：id 直接进文件名，必须拒绝分隔符与 `..`。
        if id.trim().is_empty()
            || id.contains("..")
            || id.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|'])
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("非法模板 id：{id}"),
            ));
        }
        let path = self.dir.join("templates").join(format!("{id}.json"));
        match std::fs::remove_file(path) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }

    /// 采纳提案 → 进入模板注册表（幂等；已采纳直接返回模板；已拒绝报错）。
    pub fn adopt_proposal(&self, proposal_id: &str) -> Result<TeamTemplate, String> {
        let mut proposal = self
            .get_proposal(proposal_id)
            .ok_or_else(|| format!("提案 {proposal_id} 不存在"))?;
        match proposal.status {
            TeamTemplateProposalStatus::Adopted => {
                // 幂等：模板应已在注册表；缺失时自愈重建。
                if let Some(t) = self.get_template(&proposal.template.template_id) {
                    return Ok(t);
                }
                self.save_template(&proposal.template)
                    .map_err(|e| format!("模板落盘失败：{e}"))?;
                Ok(proposal.template)
            }
            TeamTemplateProposalStatus::Rejected => {
                Err(format!("提案 {proposal_id} 已被拒绝，不能采纳"))
            }
            TeamTemplateProposalStatus::Proposed => {
                self.save_template(&proposal.template)
                    .map_err(|e| format!("模板落盘失败：{e}"))?;
                proposal.status = TeamTemplateProposalStatus::Adopted;
                self.save_proposal(&proposal)
                    .map_err(|e| format!("提案更新失败：{e}"))?;
                Ok(proposal.template)
            }
        }
    }

    /// 拒绝提案（保留记录，可审计）。
    pub fn reject_proposal(&self, proposal_id: &str) -> Result<(), String> {
        let mut proposal = self
            .get_proposal(proposal_id)
            .ok_or_else(|| format!("提案 {proposal_id} 不存在"))?;
        if proposal.status == TeamTemplateProposalStatus::Adopted {
            return Err(format!("提案 {proposal_id} 已采纳，不能拒绝"));
        }
        proposal.status = TeamTemplateProposalStatus::Rejected;
        self.save_proposal(&proposal).map_err(|e| e.to_string())
    }
}
