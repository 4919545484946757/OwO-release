//! Explicit workflow tools forwarded through the worker ToolHost and live parent registry.
use super::Agent;
use crate::tools::{Tool, ToolContext, ToolSpec};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

struct WorkflowTool {
    parent: Arc<Agent>,
    spec: ToolSpec,
}

#[async_trait]
impl Tool for WorkflowTool {
    fn spec(&self) -> ToolSpec {
        self.spec.clone()
    }

    async fn run(&self, ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        if self.parent.tool_disabled(&self.spec.name) {
            return Err(format!("workflow tool disabled: {}", self.spec.name));
        }
        let tool = self
            .parent
            .registry
            .read()
            .map_err(|_| "parent tool registry unavailable".to_string())?
            .get(&self.spec.name)
            .ok_or_else(|| format!("workflow tool disconnected: {}", self.spec.name))?;
        let current = tool.spec();
        if current.input_schema != self.spec.input_schema
            || serde_json::to_value(&current.effect).map_err(|e| e.to_string())?
                != serde_json::to_value(&self.spec.effect).map_err(|e| e.to_string())?
        {
            return Err("workflow tool schema or permissions changed; restart the node".into());
        }
        tool.run(ctx, args).await
    }
}

impl Agent {
    /// Return controlled proxies for explicitly selected, connected external tools.
    /// Workspace file/command tools always come from the scoped worker registry.
    pub fn declared_workflow_tools(
        self: &Arc<Self>,
        names: &[String],
    ) -> Result<Vec<Arc<dyn Tool>>, String> {
        let specs = self.visible_tool_specs();
        names
            .iter()
            .filter(|name| {
                name.starts_with("mcp__") || matches!(name.as_str(), "web_fetch" | "web_search")
            })
            .map(|name| {
                let spec = specs
                    .iter()
                    .find(|spec| spec.name == *name)
                    .cloned()
                    .ok_or_else(|| format!("workflow tool not connected or disabled: {name}"))?;
                Ok(Arc::new(WorkflowTool {
                    parent: Arc::clone(self),
                    spec,
                }) as Arc<dyn Tool>)
            })
            .collect()
    }
}
