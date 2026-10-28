//! turn 域：开启 SSE 流 + 取消。

use crate::error::Result;
use crate::http::AgentClient;
use crate::sse::TurnStream;
use owo_agent_protocol::TurnRequest;
use serde_json::Value;

impl AgentClient {
    /// 发起一轮对话并返回事件流（尚未读取任何 body；由调用方驱动）。
    pub async fn open_turn(&self, session_id: &str, prompt: &str) -> Result<TurnStream> {
        self.open_turn_read_only(session_id, prompt, false).await
    }

    /// Open a narrowing-only turn. Unscoped callers remain compatible with older hosts.
    pub async fn open_turn_read_only(
        &self,
        session_id: &str,
        prompt: &str,
        read_only: bool,
    ) -> Result<TurnStream> {
        self.open_turn_request(session_id, prompt, read_only, None)
            .await
    }

    /// Preallocate identity so cancellation cannot race startup or affect a later turn.
    pub async fn open_turn_with_id(
        &self,
        session_id: &str,
        prompt: &str,
        read_only: bool,
        turn_id: &str,
    ) -> Result<TurnStream> {
        let id = owo_agent_protocol::canonical_turn_id(turn_id)
            .ok_or_else(|| crate::ClientError::Protocol("turn/invalid_id".into()))?;
        self.open_turn_request(session_id, prompt, read_only, Some(id))
            .await
    }

    async fn open_turn_request(
        &self,
        session_id: &str,
        prompt: &str,
        read_only: bool,
        turn_id: Option<String>,
    ) -> Result<TurnStream> {
        if read_only || turn_id.is_some() {
            let capabilities: Value = self.get_json("/capabilities").await?;
            if read_only
                && capabilities
                    .pointer("/constraints/request_read_only")
                    .and_then(Value::as_bool)
                    != Some(true)
            {
                return Err(crate::ClientError::Protocol(
                    "permission/read_only_unsupported: no turn was submitted".into(),
                ));
            }
            if turn_id.is_some()
                && capabilities
                    .pointer("/constraints/scoped_turn_cancellation")
                    .and_then(Value::as_bool)
                    != Some(true)
            {
                return Err(crate::ClientError::Protocol(
                    "turn/scoped_cancellation_unsupported: no turn was submitted".into(),
                ));
            }
        }
        let expected = turn_id.clone();
        let request = TurnRequest {
            model_connection: None,
            prompt: prompt.into(),
            attachments: Vec::new(),
            content_parts: Vec::new(),
            read_only: read_only.then_some(true),
            turn_id,
        };
        let response = self
            .send_stream(self.post_stream(&format!("/session/{session_id}/turn"), &request))
            .await?;
        let received = response
            .headers()
            .get("x-owo-turn-id")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        if expected.is_some() && expected != received {
            return Err(crate::ClientError::Protocol(
                "turn/identity_mismatch".into(),
            ));
        }
        Ok(TurnStream::new(
            response,
            self.clone(),
            session_id.into(),
            received,
        ))
    }

    /// 回答当前会话中挂起的 ask_user 问题。
    pub async fn answer_question(
        &self,
        session_id: &str,
        question_id: &str,
        answer: &str,
    ) -> Result<Value> {
        #[derive(serde::Serialize)]
        struct QuestionAnswerRequest<'a> {
            question_id: &'a str,
            answer: &'a str,
        }
        self.post_json(
            &format!("/session/{session_id}/answer/{question_id}"),
            &QuestionAnswerRequest {
                question_id,
                answer,
            },
        )
        .await
    }

    /// 取消运行中的回合（幂等；无运行中回合时服务端仍返回 ok）。
    pub async fn cancel_turn(&self, session_id: &str) -> Result<Value> {
        self.post_empty(&format!("/session/{session_id}/abort"))
            .await
    }
    pub async fn cancel_turn_id(&self, session_id: &str, turn_id: &str) -> Result<Value> {
        let id = owo_agent_protocol::canonical_turn_id(turn_id)
            .ok_or_else(|| crate::ClientError::Protocol("turn/invalid_id".into()))?;
        let capabilities: Value = self.get_json("/capabilities").await?;
        if capabilities
            .pointer("/constraints/scoped_turn_cancellation")
            .and_then(Value::as_bool)
            != Some(true)
        {
            return Err(crate::ClientError::Protocol(
                "turn/scoped_cancellation_unsupported: no cancel was submitted".into(),
            ));
        }
        self.post_json(
            &format!("/session/{session_id}/abort"),
            &serde_json::json!({"turn_id":id}),
        )
        .await
    }
}
