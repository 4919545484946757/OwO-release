//! HTTP error mapping for the shared, versioned workflow write service.
use axum::{http::StatusCode, Json};
use owo_agent_core::user_workflow::WorkflowEditError;
use owo_agent_core::workswarm::WorkflowStoreError;
use serde_json::{json, Value};
pub type WorkflowApiError = (StatusCode, Json<Value>);
pub fn edit_error(error: WorkflowEditError) -> WorkflowApiError {
    match error {
        WorkflowEditError::Validation(errors) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"ok":false,"errors":errors})),
        ),
        WorkflowEditError::Store(error) => store_error(error),
    }
}
pub fn store_error(error: WorkflowStoreError) -> WorkflowApiError {
    match error {
        WorkflowStoreError::Conflict { expected, current } => (
            StatusCode::CONFLICT,
            Json(
                json!({"ok":false,"code":"workflow/revision_conflict","error":"工作流已被其他编辑者修改，请读取新版本并审阅差异后重试","expected_revision":expected,"current_revision":current}),
            ),
        ),
        WorkflowStoreError::DraftConflict { expected, current } => (
            StatusCode::CONFLICT,
            Json(json!({"ok":false,"code":"workflow/draft_conflict",
                "error":"草稿已被其他操作修改或丢弃，请重新读取并审阅后重试",
                "expected_draft_revision":expected,"current_draft_revision":current})),
        ),
        WorkflowStoreError::RevisionRequired { current } => (
            StatusCode::PRECONDITION_REQUIRED,
            Json(
                json!({"ok":false,"code":"workflow/revision_required","error":"更新或删除已有工作流必须提供读取时的 expected_revision；请先读取并审阅当前版本","current_revision":current}),
            ),
        ),
        WorkflowStoreError::Protected(id) => (
            StatusCode::CONFLICT,
            Json(
                json!({"ok":false,"code":"workflow/builtin_protected","error":format!("内置工作流不可修改：{id}；请保存为新的 template_id")}),
            ),
        ),
        WorkflowStoreError::NotFound(message) => (
            StatusCode::NOT_FOUND,
            Json(json!({"ok":false,"error":message})),
        ),
        WorkflowStoreError::ProposalState(message) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"ok":false,"error":message})),
        ),
        WorkflowStoreError::Invalid(message) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"ok":false,"errors":[message]})),
        ),
        error => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"ok":false,"code":"workflow/storage_failed","error":error.to_string()})),
        ),
    }
}
