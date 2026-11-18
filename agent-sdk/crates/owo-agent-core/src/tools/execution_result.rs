//! One host-owned success decision for events, audit and loop progress.
//! A process may return valid JSON while its actual command has failed. Keep that
//! JSON intact so the model and validators retain stdout, stderr and exit evidence.
use serde_json::Value;

pub(crate) fn execution_error(tool: &str, outcome: &Result<Value, String>) -> Option<String> {
    let value = match outcome {
        Ok(value) => value,
        Err(error) => return Some(error.clone()),
    };
    match (tool, value.get("exit_code").and_then(Value::as_i64)) {
        ("run_command" | "shell_output", Some(code)) if code != 0 => {
            Some(format!("executor/process_exit: exit_code={code}"))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn actual_exit_status_overrides_success_words_and_preserves_output() {
        let value = json!({"exit_code": 7, "stdout": "all tests passed", "stderr": "failure"});
        let result = Ok(value.clone());
        assert_eq!(
            execution_error("run_command", &result).as_deref(),
            Some("executor/process_exit: exit_code=7")
        );
        assert_eq!(value["stderr"], "failure");
        assert!(execution_error(
            "run_command",
            &Ok(json!({"exit_code": 0, "stdout": "error"}))
        )
        .is_none());
    }

    #[test]
    fn background_launch_polling_and_unrelated_json_keep_their_own_semantics() {
        assert!(execution_error(
            "run_command",
            &Ok(json!({"background": true, "shell_id": "s"}))
        )
        .is_none());
        assert!(execution_error(
            "shell_output",
            &Ok(json!({"running": true, "exit_code": null}))
        )
        .is_none());
        assert!(execution_error(
            "shell_output",
            &Ok(json!({"running": false, "exit_code": 0}))
        )
        .is_none());
        assert!(execution_error(
            "shell_output",
            &Ok(json!({"running": false, "exit_code": 3}))
        )
        .is_some());
        assert!(execution_error("read_file", &Ok(json!({"exit_code": 7}))).is_none());
        assert_eq!(
            execution_error("run_command", &Err("executor/process_timeout".into())).as_deref(),
            Some("executor/process_timeout")
        );
    }
}
