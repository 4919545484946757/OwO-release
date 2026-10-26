//! Bounded, immutable node edits. Does not save, execute, or authorize a workflow.
use owo_agent_protocol::{
    TeamTemplate, WorkflowEditorMetadata, WorkflowEditorNode, WorkflowNodePosition,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkflowPatch {
    UpdateNode {
        node_id: String,
        changes: BTreeMap<String, Value>,
    },
    Connect {
        from: String,
        to: String,
    },
    Disconnect {
        from: String,
        to: String,
    },
    SetPosition {
        node_id: String,
        x: f64,
        y: f64,
    },
}

/// Same UTF-16 FNV-1a identity as the web document editor, including non-BMP labels.
pub fn with_stable_node_ids(template: &TeamTemplate) -> Result<TeamTemplate, String> {
    let mut document = template.clone();
    let mut used = BTreeSet::new();
    for role in &document.roles {
        if let Some(id) = &role.node_id {
            if !used.insert(id.clone()) {
                return Err(format!("duplicate node_id: {id}"));
            }
        }
    }
    for role in &mut document.roles {
        if role.node_id.is_some() {
            continue;
        }
        let key = format!("{}\n{}", document.template_id, role.role);
        let hash = key.encode_utf16().fold(2166136261u32, |hash, c| {
            (hash ^ u32::from(c)).wrapping_mul(16777619)
        });
        let base = format!("node-{hash:x}");
        let mut id = base.clone();
        let mut suffix = 0;
        while used.contains(&id) {
            suffix += 1;
            id = format!("{base}-{suffix}");
        }
        used.insert(id.clone());
        role.node_id = Some(id);
    }
    Ok(document)
}
fn index(document: &TeamTemplate, id: &str) -> Result<usize, String> {
    document
        .roles
        .iter()
        .position(|role| role.node_id.as_deref() == Some(id))
        .ok_or_else(|| format!("node not found: {id}"))
}

pub fn propose_workflow_patch(
    template: &TeamTemplate,
    operations: &[WorkflowPatch],
) -> Result<TeamTemplate, String> {
    if operations.len() > 32
        || serde_json::to_vec(operations)
            .map_err(|e| e.to_string())?
            .len()
            > 64 * 1024
    {
        return Err("patch exceeds 32 operations or 64 KiB".into());
    }
    let mut candidate = with_stable_node_ids(template)?;
    for operation in operations {
        match operation {
            WorkflowPatch::UpdateNode { node_id, changes } => {
                let at = index(&candidate, node_id)?;
                let old_name = candidate.roles[at].role.clone();
                let mut value =
                    serde_json::to_value(&candidate.roles[at]).map_err(|e| e.to_string())?;
                for (field, content) in changes {
                    if !matches!(
                        field.as_str(),
                        "role"
                            | "assignee"
                            | "worker"
                            | "model"
                            | "tools"
                            | "write_paths"
                            | "capabilities"
                            | "handoff_contract"
                            | "verify"
                    ) {
                        return Err(format!("protected or unknown node field: {field}"));
                    }
                    value
                        .as_object_mut()
                        .unwrap()
                        .insert(field.clone(), content.clone());
                }
                let updated: owo_agent_protocol::TeamTemplateRole =
                    serde_json::from_value(value).map_err(|e| e.to_string())?;
                if updated.role != old_name
                    && candidate
                        .roles
                        .iter()
                        .enumerate()
                        .any(|(other, role)| other != at && role.role == updated.role)
                {
                    return Err(format!("role already exists: {}", updated.role));
                }
                let new_name = updated.role.clone();
                candidate.roles[at] = updated;
                if new_name != old_name {
                    for role in &mut candidate.roles {
                        for dependency in &mut role.depends_on {
                            if dependency == &old_name {
                                dependency.clone_from(&new_name);
                            }
                        }
                    }
                }
            }
            WorkflowPatch::Connect { from, to } | WorkflowPatch::Disconnect { from, to } => {
                if from == to {
                    return Err("a node cannot depend on itself".into());
                }
                let source = candidate.roles[index(&candidate, from)?].role.clone();
                let at = index(&candidate, to)?;
                let dependencies = &mut candidate.roles[at].depends_on;
                if matches!(operation, WorkflowPatch::Connect { .. }) {
                    if !dependencies.contains(&source) {
                        dependencies.push(source);
                    }
                } else {
                    dependencies.retain(|name| name != &source);
                }
            }
            WorkflowPatch::SetPosition { node_id, x, y } => {
                index(&candidate, node_id)?;
                if !x.is_finite()
                    || !y.is_finite()
                    || !(0.0..=100000.0).contains(x)
                    || !(0.0..=100000.0).contains(y)
                {
                    return Err("position outside 0..100000".into());
                }
                let editor = candidate
                    .editor
                    .get_or_insert_with(|| WorkflowEditorMetadata {
                        version: 1,
                        nodes: Vec::new(),
                    });
                let position = Some(WorkflowNodePosition { x: *x, y: *y });
                if let Some(node) = editor.nodes.iter_mut().find(|node| node.id == *node_id) {
                    node.position = position;
                } else {
                    editor.nodes.push(WorkflowEditorNode {
                        id: node_id.clone(),
                        position,
                    });
                }
            }
        }
    }
    Ok(candidate)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn template() -> TeamTemplate {
        serde_json::from_value(json!({"template_id":"patch-flow","name":"Patch","roles":[{"node_id":"first","role":"输入","model":"keep"},{"node_id":"second","role":"创作","depends_on":["输入"],"model":"old"},{"node_id":"third","role":"验收","depends_on":["创作"]}],"editor":{"version":1,"nodes":[{"id":"second","position":{"x":20,"y":30}}]}})).unwrap()
    }
    fn operations(value: Value) -> Vec<WorkflowPatch> {
        serde_json::from_value(value).unwrap()
    }
    #[test]
    fn patch_only_updates_requested_node_and_preserves_original() {
        let original = template();
        let before = serde_json::to_value(&original).unwrap();
        let edited = propose_workflow_patch(
            &original,
            &operations(json!([{"op":"update_node","node_id":"second","changes":{"model":"new"}}])),
        )
        .unwrap();
        assert_eq!(serde_json::to_value(&original).unwrap(), before);
        assert_eq!(edited.roles[0].model.as_deref(), Some("keep"));
        assert_eq!(edited.roles[1].model.as_deref(), Some("new"));
        assert_eq!(
            serde_json::to_value(&edited.roles[2]).unwrap(),
            before["roles"][2]
        );
        assert_eq!(
            serde_json::to_value(edited.editor).unwrap(),
            before["editor"]
        );
    }
    #[test]
    fn patch_rename_repairs_dependencies_without_changing_identity_or_layout() {
        let edited = propose_workflow_patch(
            &template(),
            &operations(
                json!([{"op":"update_node","node_id":"second","changes":{"role":"新创作"}}]),
            ),
        )
        .unwrap();
        assert_eq!(edited.roles[1].node_id.as_deref(), Some("second"));
        assert_eq!(edited.roles[2].depends_on, ["新创作"]);
        assert_eq!(edited.editor.unwrap().nodes[0].id, "second");
    }
    #[test]
    fn patch_rejects_identity_scope_unknown_fields_and_missing_nodes_atomically() {
        for changes in [
            json!({"node_id":"other"}),
            json!({"depends_on":[]}),
            json!({"unknown":true}),
        ] {
            assert!(propose_workflow_patch(
                &template(),
                &operations(json!([{"op":"update_node","node_id":"second","changes":changes}]))
            )
            .is_err());
        }
        assert!(serde_json::from_value::<WorkflowPatch>(
            json!({"op":"connect","from":"first","to":"second","extra":true})
        )
        .is_err());
        assert!(propose_workflow_patch(
            &template(),
            &operations(json!([{"op":"connect","from":"missing","to":"second"}]))
        )
        .is_err());
        assert!(propose_workflow_patch(
            &template(),
            &operations(json!([{"op":"set_position","node_id":"first","x":-1,"y":2}]))
        )
        .is_err());
    }
    #[test]
    fn patch_edges_positions_and_repeated_normalization_preserve_stable_ids() {
        let edited=propose_workflow_patch(&template(),&operations(json!([{"op":"disconnect","from":"first","to":"second"},{"op":"connect","from":"first","to":"third"},{"op":"set_position","node_id":"second","x":60,"y":70}]))).unwrap();
        assert!(edited.roles[1].depends_on.is_empty());
        assert_eq!(edited.roles[2].depends_on, ["创作", "输入"]);
        assert_eq!(edited.editor.unwrap().nodes[0].position.unwrap().x, 60.0);
        let mut legacy = template();
        for role in &mut legacy.roles {
            role.node_id = None;
        }
        legacy.editor = None;
        let first = with_stable_node_ids(&legacy).unwrap();
        let second = with_stable_node_ids(&first).unwrap();
        assert_eq!(
            serde_json::to_value(first).unwrap(),
            serde_json::to_value(second).unwrap()
        );
    }
    #[test]
    fn legacy_ids_match_web_utf16_golden_including_surrogate_pairs() {
        let template: TeamTemplate = serde_json::from_value(
            json!({"template_id":"patch-flow","roles":[{"role":"输入"},{"role":"语音🎧"}]}),
        )
        .unwrap();
        let normalized = with_stable_node_ids(&template).unwrap();
        assert_eq!(
            normalized.roles[0].node_id.as_deref(),
            Some("node-d73734da")
        );
        assert_eq!(
            normalized.roles[1].node_id.as_deref(),
            Some("node-2f19cca7")
        );
    }
}
