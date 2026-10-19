//! User question and skill interaction tools.

use super::{Tool, ToolContext, ToolSpec};
use async_trait::async_trait;
use serde_json::{json, Value};

pub(super) struct AskUserTool;

#[async_trait]
impl Tool for AskUserTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "ask_user".into(),
            description: "信息不足、需求含糊或存在会显著影响结果的关键分歧时，向用户提问并等待回答（回合暂停直到用户答复）。问题要具体、一次只问最关键的一两个点；能用选项固定答案时给出 options。已经明确的常规操作不要用它确认。".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "question": { "type": "string", "description": "要向用户提出的问题（简洁明确，一次只问一件事）" },
                    "options": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "可选：2-4 个备选答案，用户可直接点选"
                    }
                },
                "required": ["question"]
            }),
            effect: None,
        }
    }

    async fn run(&self, ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let question = args
            .get("question")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or("参数缺少字符串字段：question")?
            .to_string();
        let options: Vec<String> = args
            .get("options")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                    .map(str::to_string)
                    .take(4)
                    .collect()
            })
            .unwrap_or_default();
        let Some(questioner) = ctx.questioner else {
            return Err("当前运行环境没有可用的用户问答通道（无 UI 连接）。请把你的问题直接写进最终回复向用户提出，并给出你建议的默认方案。".to_string());
        };
        let request = crate::question::UserQuestion {
            question_id: uuid::Uuid::new_v4().to_string(),
            question,
            options,
        };
        match questioner.ask(&request).await {
            Some(answer) if !answer.answer.trim().is_empty() => Ok(json!({
                "answered": true,
                "answer": answer.answer,
            })),
            // 超时/中止/空回答：不给模型「卡住」的机会——明确告知并允许继续。
            _ => Ok(json!({
                "answered": false,
                "note": "用户未在时限内回答。请基于已有信息继续执行，并在最终回复里把不确定的部分标注出来。",
            })),
        }
    }
}

pub(super) struct UseSkillTool;

/// 技能指令回灌上限：技能来自工作区目录（`.agents/skills/*/SKILL.md`），属于外部内容，
/// 一份超长 SKILL.md 就能把回合上下文挤满；超限时截断并给出分页路径。
pub(super) const MAX_SKILL_CHARS: usize = 24_000;

#[async_trait]
impl Tool for UseSkillTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "use_skill".into(),
            description: "读取已加载技能（SKILL.md）的完整指令并按其流程执行；名称可通过 /skills 或技能清单查看。\
                          指令超过 24k 字符会截断并给出可继续读取的文件路径"
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "task": { "type": "string" }
                },
                "required": ["name"]
            }),
            effect: None,
        }
    }

    async fn run(&self, ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let name = args
            .get("name")
            .and_then(Value::as_str)
            .ok_or("参数缺少字符串字段：name")?;
        let Some(skill) = ctx.skills.get_enabled(name) else {
            let available = ctx
                .skills
                .list_enabled()
                .iter()
                .map(|skill| skill.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(format!(
                "未找到技能或技能已禁用：{name}；可用技能：{available}"
            ));
        };
        let task = args
            .get("task")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let total_chars = skill.instructions.chars().count();
        let truncated = total_chars > MAX_SKILL_CHARS;
        let instructions: String = if truncated {
            skill.instructions.chars().take(MAX_SKILL_CHARS).collect()
        } else {
            skill.instructions.clone()
        };
        // 相对路径优先（read_file 以会话工作区为基座解析）；技能若来自数据根则保持绝对路径。
        let path = skill
            .path
            .strip_prefix(ctx.workspace)
            .unwrap_or(&skill.path)
            .to_string_lossy()
            .replace('\\', "/");
        Ok(json!({
            "skill": skill.name,
            "description": skill.description,
            "task": task,
            "instructions": instructions,
            "instructions_path": path,
            "total_chars": total_chars,
            "truncated": truncated,
            "note": if truncated {
                format!(
                    "技能指令过长（{total_chars} 字符），已截断到 {MAX_SKILL_CHARS} 字符；\
                     继续读取用 read_file(path=\"{path}\", offset={MAX_SKILL_CHARS})"
                )
            } else {
                String::new()
            },
        }))
    }
}
