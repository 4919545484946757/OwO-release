//! Host-selected persistence for admitted turn input and execution checkpoints.
//! The existing SessionStore remains canonical; no second history or tool registry.
use super::{commit_turn_messages, AgentError, ChatMessage, Session, TurnExecutionOptions};
use crate::session::SessionStore;
use std::sync::Arc;

/// Persistence is host-owned and cannot be enabled or redirected by model output.
#[derive(Clone)]
pub struct TurnSessionCheckpoint {
    store: Arc<dyn SessionStore>,
}
impl std::fmt::Debug for TurnSessionCheckpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TurnSessionCheckpoint")
            .finish_non_exhaustive()
    }
}
impl TurnSessionCheckpoint {
    pub fn new(store: Arc<dyn SessionStore>) -> Self {
        Self { store }
    }

    pub(super) fn save(&self, session: &Session) -> Result<(), AgentError> {
        self.store
            .save(session)
            .map_err(|error| AgentError::Session(format!("turn/checkpoint_failed: {error}")))
    }
}

pub(super) fn checkpoint_messages(
    session: &mut Session,
    messages: &[ChatMessage],
    options: &TurnExecutionOptions,
) -> Result<(), AgentError> {
    if let Some(checkpoint) = &options.session_checkpoint {
        commit_turn_messages(session, messages);
        checkpoint.save(session)?;
    }
    Ok(())
}
