//! Optional graph metadata never grants execution rights.
use owo_agent_protocol::TeamTemplate;
use std::collections::BTreeSet;

pub(crate) fn validate_node_ids<'a>(
    nodes: impl Iterator<Item = (&'a str, Option<&'a str>)>,
) -> Vec<String> {
    let mut errors = Vec::new();
    let mut ids = BTreeSet::new();
    for (role, id) in nodes {
        let Some(id) = id else { continue };
        if id.is_empty()
            || id.len() > 128
            || !id.as_bytes()[0].is_ascii_alphabetic()
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        {
            errors.push(format!(
                "角色 {role} 的 node_id 非法（字母开头、最多 128 个 ASCII 字母数字或 -_.）"
            ));
        }
        if !ids.insert(id) {
            errors.push(format!("node_id 重复：{id}"));
        }
    }
    errors
}

pub(super) fn validate_template_graph(template: &TeamTemplate) -> Vec<String> {
    let mut errors = validate_node_ids(
        template
            .roles
            .iter()
            .map(|r| (r.role.as_str(), r.node_id.as_deref())),
    );
    let Some(editor) = &template.editor else {
        return errors;
    };
    if editor.version != 1 {
        errors.push("editor.version 仅支持 1".into());
    }
    if editor.nodes.len() > super::MAX_ROLES {
        errors.push("editor.nodes 超过角色数上限".into());
    }
    let ids = template
        .roles
        .iter()
        .filter_map(|r| r.node_id.as_deref())
        .collect::<BTreeSet<_>>();
    let mut seen = BTreeSet::new();
    for node in &editor.nodes {
        if !ids.contains(node.id.as_str()) {
            errors.push(format!("布局引用不存在的 node_id：{}", node.id));
        }
        if !seen.insert(node.id.as_str()) {
            errors.push(format!("布局重复 node_id：{}", node.id));
        }
        if let Some(position) = node.position {
            if !position.x.is_finite()
                || !position.y.is_finite()
                || !(0.0..=100_000.0).contains(&position.x)
                || !(0.0..=100_000.0).contains(&position.y)
            {
                errors.push(format!("节点 {} 的位置必须在 0..100000 范围内", node.id));
            }
        }
    }
    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn template() -> TeamTemplate {
        serde_json::from_value(json!({"template_id":"graph-fixture","name":"Fixture","applicability":"fixture","roles":[{"node_id":"node-a","role":"A"},{"node_id":"node-b","role":"B","depends_on":["A"]}],"editor":{"version":1,"nodes":[{"id":"node-a","position":{"x":12.5,"y":40}},{"id":"node-b"}]}})).unwrap()
    }
    #[test]
    fn workflow_graph_identity_and_layout_survive_role_spec_and_template_roundtrip() {
        let mut value = template();
        assert!(validate_template_graph(&value).is_empty());
        value.roles[0].role = "Renamed".into();
        value.roles[1].depends_on = vec!["Renamed".into()];
        let spec = crate::workswarm::RoleSpec::from(value.roles[0].clone());
        let reopened: crate::workswarm::RoleSpec =
            serde_json::from_value(serde_json::to_value(spec).unwrap()).unwrap();
        assert_eq!(reopened.node_id.as_deref(), Some("node-a"));
        let reopened: TeamTemplate =
            serde_json::from_value(serde_json::to_value(value).unwrap()).unwrap();
        assert_eq!(reopened.editor.unwrap().nodes[0].position.unwrap().x, 12.5);
        assert_eq!(reopened.roles[1].depends_on, ["Renamed"]);
        assert!(validate_template_graph(&TeamTemplate::default()).is_empty());
    }
    #[test]
    fn workflow_graph_rejects_dangling_duplicate_invalid_identity_and_layout() {
        let mut value = template();
        value.roles[1].node_id = Some("node-a".into());
        assert!(validate_template_graph(&value)
            .iter()
            .any(|e| e.contains("重复")));
        let mut value = template();
        value.editor.as_mut().unwrap().nodes[0].id = "missing".into();
        assert!(validate_template_graph(&value)
            .iter()
            .any(|e| e.contains("不存在")));
        let mut value = template();
        value.editor.as_mut().unwrap().version = 2;
        value.editor.as_mut().unwrap().nodes[0]
            .position
            .as_mut()
            .unwrap()
            .x = f64::NAN;
        assert_eq!(validate_template_graph(&value).len(), 2);
        for id in ["", "../escape", "数字", "1node"] {
            let mut value = template();
            value.roles[0].node_id = Some(id.into());
            assert!(!validate_template_graph(&value).is_empty());
        }
    }
}
