//! Shared user-write service. A saved configuration grants no execution permission.
use crate::workswarm::{TeamTemplateRegistry, WorkflowRevision, WorkflowStoreError};
use owo_agent_protocol::TeamTemplate;

#[derive(Debug, thiserror::Error)]
pub enum WorkflowEditError {
    #[error("workflow validation failed: {0:?}")]
    Validation(Vec<String>),
    #[error(transparent)]
    Store(#[from] WorkflowStoreError),
}

pub fn prepare_user_workflow(
    registry: &TeamTemplateRegistry,
    mut template: TeamTemplate,
) -> Result<TeamTemplate, WorkflowEditError> {
    template.template_id = template.template_id.trim().to_string();
    match template.origin.as_deref() {
        None | Some("user") => template.origin = Some("user".into()),
        Some("community") => {}
        Some(_) => {
            return Err(WorkflowEditError::Validation(vec![
                "origin 只能是 user/community".into(),
            ]))
        }
    }
    if template.created_at.trim().is_empty() {
        template.created_at = registry
            .get_revision(&template.template_id)?
            .and_then(|record| record.template)
            .map(|existing| existing.created_at)
            .filter(|at| !at.is_empty())
            .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());
    }
    let template = super::with_stable_node_ids(&template)
        .map_err(|error| WorkflowEditError::Validation(vec![error]))?;
    let errors = super::validate_workflow_template(&template);
    if !errors.is_empty() {
        return Err(WorkflowEditError::Validation(errors));
    }
    Ok(template)
}

pub fn save_user_workflow(
    registry: &TeamTemplateRegistry,
    template: TeamTemplate,
    expected_revision: Option<u64>,
) -> Result<WorkflowRevision, WorkflowEditError> {
    let template = prepare_user_workflow(registry, template)?;
    Ok(registry.save_user_template(&template, expected_revision)?)
}

/// Validate the immutable proposal, then bind that exact source hash to its atomic adoption.
pub fn adopt_user_workflow_proposal(
    registry: &TeamTemplateRegistry,
    id: &str,
) -> Result<WorkflowRevision, WorkflowEditError> {
    let record = registry
        .get_proposal_record(id)?
        .ok_or_else(|| WorkflowStoreError::NotFound(format!("提案 {id} 不存在")))?;
    let template = prepare_user_workflow(registry, record.proposal.template)?;
    Ok(registry.adopt_reviewed_proposal(id, &record.proposal_hash, &template, None)?)
}
