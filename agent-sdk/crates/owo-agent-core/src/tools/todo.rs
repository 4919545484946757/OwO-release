//! Session-local task list tool.

use super::{Tool, ToolContext, ToolSpec};
use async_trait::async_trait;
use serde_json::{json, Value};

/// 会话级任务清单（整表替换；CLI `/todo` 渲染）。
pub(super) struct TodoTool;

/// 清单规模上限：整表替换意味着模型可以一次写入任意长清单，而清单会进会话库、
/// 每回合回灌上下文与 UI；不设上限时一次"列 500 条 TODO"就能把上下文挤满。
pub(super) const MAX_TODOS: usize = 50;
/// 单条内容上限（字符）。
pub(super) const MAX_TODO_CHARS: usize = 200;

#[async_trait]
impl Tool for TodoTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "todo".into(),
            description: "写入/更新任务清单（整表替换；status: pending|in_progress|completed）。\
                          上限：最多 50 条、单条 200 字符——只列当前要推进的关键步骤"
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "todos": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "content": { "type": "string" },
                                "status": { "type": "string", "enum": ["pending", "in_progress", "completed"] }
                            },
                            "required": ["content", "status"]
                        }
                    }
                },
                "required": ["todos"]
            }),
            effect: None,
        }
    }

    async fn run(&self, ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let items = args
            .get("todos")
            .and_then(Value::as_array)
            .ok_or("todos 必须是数组")?;
        if items.len() > MAX_TODOS {
            return Err(format!(
                "任务清单最多 {MAX_TODOS} 条（收到 {} 条）：只列当前要推进的关键步骤，细节写进正文/文件",
                items.len()
            ));
        }
        let mut todos = Vec::with_capacity(items.len());
        for item in items {
            let content = item
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string();
            if content.is_empty() {
                return Err("todo.content 不能为空".to_string());
            }
            if content.chars().count() > MAX_TODO_CHARS {
                return Err(format!(
                    "todo.content 过长（{} 字符 > {MAX_TODO_CHARS}）：一句话说明这一步要做什么",
                    content.chars().count()
                ));
            }
            let status = item
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("pending");
            if !matches!(status, "pending" | "in_progress" | "completed") {
                return Err(format!(
                    "todo.status 非法：{status}（pending|in_progress|completed）"
                ));
            }
            todos.push(crate::session::TodoItem {
                content,
                status: status.to_string(),
            });
        }
        ctx.session.todos = todos;
        let rendered = ctx
            .session
            .todos
            .iter()
            .map(|todo| {
                let mark = match todo.status.as_str() {
                    "completed" => "x",
                    "in_progress" => ">",
                    _ => " ",
                };
                format!("[{mark}] {}", todo.content)
            })
            .collect::<Vec<_>>()
            .join("\n");
        Ok(json!({ "todos": ctx.session.todos, "rendered": rendered }))
    }
}
