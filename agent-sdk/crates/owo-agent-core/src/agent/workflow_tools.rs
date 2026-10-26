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

pub(super) fn is_parent_workflow_tool(name: &str) -> bool {
    name.starts_with("mcp__")
        || matches!(
            name,
            "web_fetch" | "web_search" | "generate_image" | "text_to_speech"
        )
}

struct HostGuardedTool {
    parent: Arc<Agent>,
    inner: Arc<dyn Tool>,
}
#[async_trait]
impl Tool for HostGuardedTool {
    fn spec(&self) -> ToolSpec {
        self.inner.spec()
    }
    async fn run(&self, ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        if self.parent.tool_disabled(&self.inner.spec().name) {
            return Err("workflow tool disabled by host".into());
        }
        self.inner.run(ctx, args).await
    }
}

impl Agent {
    pub(crate) fn guard_workflow_registry(
        self: &Arc<Self>,
        registry: &mut crate::tools::ToolRegistry,
    ) -> Result<(), String> {
        for spec in registry.specs() {
            if self.tool_disabled(&spec.name) {
                return Err(format!("workflow tool disabled: {}", spec.name));
            }
            let inner = registry
                .get(&spec.name)
                .ok_or_else(|| "worker registry changed".to_string())?;
            registry.register_arc(Arc::new(HostGuardedTool {
                parent: Arc::clone(self),
                inner,
            }));
        }
        Ok(())
    }

    /// Return controlled proxies for explicitly selected, connected external tools.
    /// Workspace file/command tools always come from the scoped worker registry.
    pub fn declared_workflow_tools(
        self: &Arc<Self>,
        names: &[String],
    ) -> Result<Vec<Arc<dyn Tool>>, String> {
        let specs = self.visible_tool_specs();
        names
            .iter()
            .filter(|name| is_parent_workflow_tool(name))
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
