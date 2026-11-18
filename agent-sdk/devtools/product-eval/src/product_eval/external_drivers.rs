//! Fail-closed external driver registry and bound evidence validation.
//!
//! The evaluator never treats a Desktop/LongHorizon checker pass as sufficient.
//! A registered driver executable must be run for the exact matrix attempt, must
//! report a matching attempt binding, and must produce an independent bundle
//! whose files are hashed and verified by the runner. A checker-only result,
//! reference replay, stale/replayed evidence, mock surface, or failed driver is
//! rejected. Historical reports remain parseable through serde defaults.

use crate::product_eval::{
    EvalCategory, ExecContext, ExternalDriverKind, MatrixKey, ProductEvalCase, RawExecOutcome,
    RunOptions,
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The command a registered external driver receives as argv[1].
pub const DRIVER_COMMAND: &str = "execute";
pub const EVIDENCE_SCHEMA_VERSION: u32 = 1;
const MAX_DRIVER_STDOUT_BYTES: u64 = 1024 * 1024;
const MAX_EVIDENCE_FILE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_EVIDENCE_TOTAL_BYTES: u64 = 128 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DriverRequest {
    pub command: String,
    pub schema_version: u32,
    pub driver_kind: String,
    pub driver_id: String,
    pub driver_identity_sha256: String,
    pub case_id: String,
    pub case_contract_sha256: String,
    pub category: String,
    pub execution: String,
    pub agent_mode: String,
    pub repetition: u32,
    pub batch_label: Option<String>,
    pub repair_index: u32,
    pub repair_limit: u32,
    pub fault_kind: Option<String>,
    pub fault_after_step: Option<u32>,
    pub sandbox: String,
    pub evidence_dir: String,
    pub config: Option<String>,
    pub allow_write: Vec<String>,
    pub expected_artifacts: Vec<String>,
    pub nonce: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceFile {
    pub path: String,
    pub size_bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DriverArtifact {
    /// Relative path inside the case sandbox; must match a declared allow_write.
    pub path: String,
    /// UTF-8 artifact body produced by the real driver execution.
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DriverEvidence {
    pub schema_version: u32,
    pub driver_kind: String,
    pub driver_id: String,
    pub driver_identity_sha256: String,
    pub case_id: String,
    pub case_contract_sha256: String,
    pub execution: String,
    pub agent_mode: String,
    pub repetition: u32,
    pub batch_label: Option<String>,
    pub repair_index: u32,
    pub repair_limit: u32,
    pub nonce: String,
    pub status: String,
    pub real_execution: bool,
    pub fault_kind: Option<String>,
    pub fault_after_step: Option<u32>,
    pub action_receipt_count: usize,
    pub independent_observation_count: usize,
    pub human_interventions: u32,
    pub artifacts: Vec<DriverArtifact>,
    pub files: Vec<EvidenceFile>,
    /// Runner-authoritative absolute evidence root; the driver repeats it so a
    /// replayed report cannot be re-pointed at another directory.
    pub evidence_dir: String,
}

#[derive(Debug, Clone)]
pub struct ExternalDriverRegistry {
    desktop: Option<PathBuf>,
    fault: Option<PathBuf>,
    /// When drivers are registered, this file's parent is the only allowed evidence root.
    config: Option<PathBuf>,
    identity_sha256: String,
}

impl ExternalDriverRegistry {
    pub fn new(
        desktop: Option<PathBuf>,
        fault: Option<PathBuf>,
        config: Option<PathBuf>,
    ) -> Result<Self, String> {
        for (kind, path) in [("desktop", &desktop), ("fault", &fault)] {
            if let Some(path) = path {
                let metadata = std::fs::metadata(path)
                    .map_err(|error| format!("{kind} 驱动不可读：{}：{error}", path.display()))?;
                if !metadata.is_file() || metadata.len() == 0 {
                    return Err(format!("{kind} 驱动不是非空文件：{}", path.display()));
                }
            }
        }
        if let Some(config) = &config {
            let metadata = std::fs::metadata(config)
                .map_err(|error| format!("驱动配置不可读：{}：{error}", config.display()))?;
            if !metadata.is_file() {
                return Err(format!("驱动配置不是文件：{}", config.display()));
            }
        }
        let mut hasher = Sha256::new();
        for path in [&desktop, &fault] {
            match path {
                Some(path) => {
                    let bytes = std::fs::read(path)
                        .map_err(|error| format!("读取驱动 {} 失败：{error}", path.display()))?;
                    hasher.update(b"driver");
                    hasher.update(&bytes);
                }
                None => hasher.update(b"unregistered"),
            }
        }
        if let Some(config) = &config {
            let bytes =
                std::fs::read(config).map_err(|error| format!("读取驱动配置失败：{error}"))?;
            hasher.update(b"config");
            hasher.update(&bytes);
            // A registered executable may be a wrapper around helper scripts.
            // Bind every helper declared in the config so changing real driver
            // behavior invalidates the out-dir identity fingerprint.
            let manifest: serde_json::Value = serde_json::from_slice(&bytes)
                .map_err(|error| format!("解析驱动配置失败：{error}"))?;
            let mut helpers = Vec::new();
            if let Some(obj) = manifest.as_object() {
                for (key, value) in obj {
                    if key.ends_with("_sha256") {
                        continue;
                    }
                    match value {
                        serde_json::Value::String(name)
                            if name.ends_with(".py") || name.ends_with(".cmd") =>
                        {
                            helpers.push(name.clone());
                        }
                        serde_json::Value::Array(values) => {
                            for value in values {
                                if let Some(name) = value.as_str() {
                                    if name.ends_with(".py") || name.ends_with(".cmd") {
                                        helpers.push(name.to_string());
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            helpers.sort();
            helpers.dedup();
            for name in helpers {
                let helper = config
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .join(&name);
                let bytes = std::fs::read(&helper).map_err(|error| {
                    format!("读取驱动辅助文件 {} 失败：{error}", helper.display())
                })?;
                hasher.update(b"helper-path");
                hasher.update(name.as_bytes());
                hasher.update(b"helper-bytes");
                hasher.update(&bytes);
            }
        }
        Ok(Self {
            desktop,
            fault,
            config,
            identity_sha256: format!("{:x}", hasher.finalize()),
        })
    }

    pub fn identity_sha256(&self) -> String {
        self.identity_sha256.clone()
    }

    pub(crate) fn program(&self, kind: ExternalDriverKind) -> Option<&Path> {
        match kind {
            ExternalDriverKind::RealDesktop => self.desktop.as_deref(),
            ExternalDriverKind::FaultInjection => self.fault.as_deref(),
        }
    }

    pub(crate) fn evidence_root(&self) -> Option<PathBuf> {
        self.config
            .as_ref()
            .and_then(|path| path.parent())
            .map(Path::to_path_buf)
    }

    pub fn validate(
        &self,
        case: &ProductEvalCase,
        key: &MatrixKey,
        execution: &str,
        batch_label: Option<&str>,
        repair_limit: u32,
        value: &serde_json::Value,
    ) -> Result<(), String> {
        let kind = required_external_driver(case)
            .ok_or_else(|| "非外部驱动题不允许携带正式外部证据".to_string())?;
        let program = self.program(kind).ok_or_else(|| match kind {
            ExternalDriverKind::RealDesktop => "EVAL_REAL_DESKTOP_DRIVER_REQUIRED".to_string(),
            ExternalDriverKind::FaultInjection => {
                "EVAL_FAULT_INJECTION_DRIVER_REQUIRED".to_string()
            }
        })?;
        let _ = program;
        let evidence: DriverEvidence = serde_json::from_value(value.clone())
            .map_err(|error| format!("EVAL_EXTERNAL_EVIDENCE_INVALID：{error}"))?;
        if evidence.schema_version != EVIDENCE_SCHEMA_VERSION
            || evidence.status != "passed"
            || !evidence.real_execution
        {
            return Err("EVAL_EXTERNAL_EVIDENCE_INVALID：驱动未报告真实通过".to_string());
        }
        if evidence.driver_kind != kind.as_str()
            || evidence.driver_identity_sha256 != self.identity_sha256
            || evidence.case_id != key.case_id
            || evidence.case_contract_sha256 != case_contract_sha256(case)
            || evidence.execution != execution
            || evidence.agent_mode != key.agent_mode.as_str()
            || evidence.repetition != key.repetition
            || evidence.batch_label.as_deref() != batch_label
            || evidence.nonce.is_empty()
            || evidence.repair_index > repair_limit
            || evidence.repair_limit != repair_limit
        {
            return Err(
                "EVAL_EXTERNAL_EVIDENCE_MISMATCH：证据与注册驱动或矩阵单元不匹配".to_string(),
            );
        }
        let fault = case.fault_injection.as_ref();
        if evidence.fault_kind.as_deref() != fault.map(|f| f.kind.as_str())
            || evidence.fault_after_step != fault.and_then(|f| f.after_step)
        {
            return Err("EVAL_EXTERNAL_EVIDENCE_MISMATCH：故障注入声明不一致".to_string());
        }
        match kind {
            ExternalDriverKind::RealDesktop => {
                if evidence.action_receipt_count == 0 || evidence.independent_observation_count == 0
                {
                    return Err(
                        "EVAL_EXTERNAL_EVIDENCE_INCOMPLETE：缺少真实动作回执或独立观察".to_string(),
                    );
                }
            }
            ExternalDriverKind::FaultInjection => {
                if fault.is_none()
                    || evidence.action_receipt_count == 0
                    || evidence.independent_observation_count == 0
                {
                    return Err(
                        "EVAL_EXTERNAL_EVIDENCE_INCOMPLETE：缺少故障观察或检查点审计".to_string(),
                    );
                }
            }
        }
        let expected = expected_evidence_files(case, kind);
        if evidence.artifacts.is_empty() {
            return Err("EVAL_EXTERNAL_EVIDENCE_INCOMPLETE：驱动未产出本格任务产物".to_string());
        }
        for artifact in &evidence.artifacts {
            if !case
                .expected_artifacts
                .iter()
                .any(|wanted| wanted == &artifact.path)
            {
                return Err(
                    "EVAL_EXTERNAL_EVIDENCE_MISMATCH：驱动产物不是本题声明的产物".to_string(),
                );
            }
        }
        if evidence.files.len() != expected.len() {
            return Err("EVAL_EXTERNAL_EVIDENCE_INCOMPLETE：证据文件清单不完整".to_string());
        }
        let evidence_dir = PathBuf::from(&evidence.evidence_dir);
        if evidence_dir
            != value
                .get("evidence_dir")
                .and_then(serde_json::Value::as_str)
                .map(PathBuf::from)
                .unwrap_or_default()
        {
            return Err("EVAL_EXTERNAL_EVIDENCE_MISMATCH：证据目录与报告绑定不一致".to_string());
        }
        let canonical_evidence_dir = std::fs::canonicalize(&evidence_dir)
            .map_err(|error| format!("EVAL_EXTERNAL_EVIDENCE_INVALID：证据根不可解析：{error}"))?;
        let configured_root = self
            .evidence_root()
            .ok_or_else(|| "EVAL_EXTERNAL_DRIVER_EVIDENCE_ROOT_REQUIRED".to_string())?;
        let canonical_root = std::fs::canonicalize(&configured_root).map_err(|error| {
            format!("EVAL_EXTERNAL_EVIDENCE_INVALID：驱动配置根不可解析：{error}")
        })?;
        if !canonical_evidence_dir.starts_with(&canonical_root) {
            return Err("EVAL_EXTERNAL_EVIDENCE_MISMATCH：证据目录越出注册配置范围".to_string());
        }
        let mut total = 0_u64;
        for (file, wanted) in evidence.files.iter().zip(expected.iter()) {
            if file.path != *wanted {
                return Err("EVAL_EXTERNAL_EVIDENCE_MISMATCH：证据文件名不合法".to_string());
            }
            if file.size_bytes > MAX_EVIDENCE_FILE_BYTES {
                return Err("EVAL_EXTERNAL_EVIDENCE_INCOMPLETE：证据文件过大".to_string());
            }
            total = total.saturating_add(file.size_bytes);
            let path = evidence_dir.join(Path::new(&file.path));
            if !path.is_file() {
                return Err("EVAL_EXTERNAL_EVIDENCE_INCOMPLETE：证据文件缺失".to_string());
            }
            let bytes = std::fs::read(&path).map_err(|error| {
                format!("EVAL_EXTERNAL_EVIDENCE_INCOMPLETE：读取证据失败：{error}")
            })?;
            if bytes.len() as u64 != file.size_bytes
                || format!("{:x}", Sha256::digest(&bytes)) != file.sha256
            {
                return Err("EVAL_EXTERNAL_EVIDENCE_MISMATCH：证据文件哈希不一致".to_string());
            }
        }
        if total > MAX_EVIDENCE_TOTAL_BYTES {
            return Err("EVAL_EXTERNAL_EVIDENCE_INCOMPLETE：证据总量过大".to_string());
        }
        Ok(())
    }
}

/// Fail-closed selection of the driver required by a case. This is source of
/// truth for both the built-in executors' deny messages and runner validation.
pub fn required_external_driver(case: &ProductEvalCase) -> Option<ExternalDriverKind> {
    match case.category {
        EvalCategory::Desktop => Some(ExternalDriverKind::RealDesktop),
        EvalCategory::LongHorizon => Some(ExternalDriverKind::FaultInjection),
        _ if case.fault_injection.is_some() => Some(ExternalDriverKind::FaultInjection),
        _ => None,
    }
}

pub fn case_contract_sha256(case: &ProductEvalCase) -> String {
    let canonical = serde_json::to_vec(case).unwrap_or_else(|_| case.id.as_bytes().to_vec());
    format!("{:x}", Sha256::digest(canonical))
}

fn expected_evidence_files(case: &ProductEvalCase, kind: ExternalDriverKind) -> Vec<String> {
    let mut files = vec![
        "driver-request.json".to_string(),
        "driver-stdout.json".to_string(),
    ];
    match kind {
        ExternalDriverKind::RealDesktop => {
            files.extend([
                "windows-actions.json".to_string(),
                "independent-observations.json".to_string(),
                "screenshots/manifest.json".to_string(),
            ]);
        }
        ExternalDriverKind::FaultInjection => {
            let fault_kind = case
                .fault_injection
                .as_ref()
                .map(|f| f.kind.as_str())
                .unwrap_or("unknown");
            files.extend([
                "fault-observation.json".to_string(),
                format!("checkpoint-before-{fault_kind}.json"),
                format!("checkpoint-after-{fault_kind}.json"),
                "side-effect-replay-audit.json".to_string(),
            ]);
        }
    }
    files
}

fn safe_evidence_rel(raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    if raw.is_empty() || raw.starts_with('/') || raw.starts_with('\\') || raw.contains(':') {
        return Err(format!("非法证据相对路径：{raw}"));
    }
    let normalized = raw.replace('\\', "/");
    if normalized
        .split('/')
        .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(format!("证据路径含越界段：{raw}"));
    }
    Ok(normalized)
}

fn hash_file(path: &Path) -> Result<(u64, String), String> {
    let bytes =
        std::fs::read(path).map_err(|error| format!("读取 {} 失败：{error}", path.display()))?;
    Ok((bytes.len() as u64, format!("{:x}", Sha256::digest(&bytes))))
}

fn canonical_json(value: &serde_json::Value) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

/// Runner-side driver invocation. It returns a bound evidence record only when
/// the external process exited 0, wrote every required file, and emitted a
/// matching report. Any failure returns an error in RawExecOutcome.
pub fn run_registered_driver(
    registry: &ExternalDriverRegistry,
    ctx: &mut ExecContext<'_>,
    opts: &RunOptions,
    outcome: &mut RawExecOutcome,
) {
    let Some(kind) = required_external_driver(ctx.case) else {
        return;
    };
    let Some(attempt) = ctx.matrix_attempt() else {
        outcome.error = Some("EVAL_EXTERNAL_DRIVER_NO_MATRIX_ATTEMPT".to_string());
        return;
    };
    let required_code = match kind {
        ExternalDriverKind::RealDesktop => "EVAL_REAL_DESKTOP_DRIVER_REQUIRED",
        ExternalDriverKind::FaultInjection => "EVAL_FAULT_INJECTION_DRIVER_REQUIRED",
    };
    let Some(program) = registry.program(kind) else {
        outcome.error = Some(required_code.to_string());
        return;
    };
    let driver_path = program.to_path_buf();
    let Some(evidence_root) = registry.evidence_root() else {
        outcome.error = Some("EVAL_EXTERNAL_DRIVER_EVIDENCE_ROOT_REQUIRED".to_string());
        return;
    };
    let cell = format!(
        "{}-{}-{}-{}-{}",
        attempt
            .key
            .case_id
            .replace(|c: char| !c.is_ascii_alphanumeric(), "_"),
        attempt.key.agent_mode.as_str(),
        attempt.key.repetition,
        attempt.repair_index,
        uuid::Uuid::new_v4().simple()
    );
    let evidence_dir = evidence_root.join("external-driver-evidence").join(&cell);
    if let Err(error) = std::fs::create_dir_all(evidence_dir.join("screenshots")) {
        outcome.error = Some(format!("EVAL_EXTERNAL_DRIVER_SETUP_FAILED：{error}"));
        return;
    }
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    let fault = ctx.case.fault_injection.as_ref();
    let request = DriverRequest {
        command: DRIVER_COMMAND.to_string(),
        schema_version: EVIDENCE_SCHEMA_VERSION,
        driver_kind: kind.as_str().to_string(),
        driver_id: driver_path.to_string_lossy().into_owned(),
        driver_identity_sha256: registry.identity_sha256(),
        case_id: attempt.key.case_id.clone(),
        case_contract_sha256: case_contract_sha256(ctx.case),
        category: ctx.case.category.as_str().to_string(),
        execution: attempt.execution.to_string(),
        agent_mode: attempt.key.agent_mode.as_str().to_string(),
        repetition: attempt.key.repetition,
        batch_label: attempt.batch_label.map(str::to_string),
        repair_index: attempt.repair_index,
        repair_limit: attempt.repair_limit,
        fault_kind: fault.map(|f| f.kind.clone()),
        fault_after_step: fault.and_then(|f| f.after_step),
        sandbox: ctx.sandbox.to_string_lossy().into_owned(),
        evidence_dir: evidence_dir.to_string_lossy().into_owned(),
        config: opts
            .driver_config
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned()),
        allow_write: ctx.case.allow_write.clone(),
        expected_artifacts: ctx.case.expected_artifacts.clone(),
        nonce: nonce.clone(),
    };
    let request_json = serde_json::to_vec_pretty(&request).unwrap_or_default();
    if let Err(error) = std::fs::write(evidence_dir.join("driver-request.json"), &request_json) {
        outcome.error = Some(format!("EVAL_EXTERNAL_DRIVER_SETUP_FAILED：{error}"));
        return;
    }
    let output = Command::new(&driver_path)
        .arg(DRIVER_COMMAND)
        .arg(evidence_dir.join("driver-request.json"))
        .output();
    let output = match output {
        Ok(output) => output,
        Err(error) => {
            outcome.error = Some(format!("EVAL_EXTERNAL_DRIVER_RUN_FAILED：{error}"));
            return;
        }
    };
    let stdout_bytes = output.stdout;
    let stdout_path = evidence_dir.join("driver-stdout.json");
    let _ = std::fs::write(&stdout_path, &stdout_bytes);
    if stdout_bytes.len() as u64 > MAX_DRIVER_STDOUT_BYTES || !output.status.success() {
        outcome.error = Some(format!(
            "EVAL_EXTERNAL_DRIVER_RUN_FAILED：exit={:?} stdout_bytes={}",
            output.status.code(),
            stdout_bytes.len()
        ));
        return;
    }
    let mut report: DriverEvidence = match serde_json::from_slice(&stdout_bytes) {
        Ok(report) => report,
        Err(error) => {
            outcome.error = Some(format!("EVAL_EXTERNAL_EVIDENCE_INVALID：{error}"));
            return;
        }
    };
    if report.driver_kind != kind.as_str()
        || report.driver_identity_sha256 != registry.identity_sha256()
        || report.case_id != attempt.key.case_id
        || report.execution != attempt.execution
        || report.agent_mode != attempt.key.agent_mode.as_str()
        || report.repetition != attempt.key.repetition
        || report.batch_label.as_deref() != opts.batch_label.as_deref()
        || report.nonce != nonce
        || report.repair_index != attempt.repair_index
        || report.repair_limit != opts.repair_attempts
        || report.evidence_dir != evidence_dir.to_string_lossy()
    {
        outcome.error =
            Some("EVAL_EXTERNAL_EVIDENCE_MISMATCH：驱动报告与本次矩阵尝试不一致".to_string());
        return;
    }
    // Runner-mediated sandbox writes: external drivers never receive a raw
    // sandbox path and cannot bypass the case's allow_write policy.
    let mut bound_artifacts: Vec<DriverArtifact> = Vec::new();
    for artifact in report.artifacts.drain(..) {
        if let Err(error) = ctx.write_driver_artifact(&artifact.path, &artifact.content) {
            outcome.error = Some(format!(
                "EVAL_EXTERNAL_DRIVER_ARTIFACT_WRITE_FAILED：{error}"
            ));
            return;
        }
        bound_artifacts.push(DriverArtifact {
            path: artifact.path.clone(),
            content: String::new(),
        });
    }
    report.artifacts = bound_artifacts.clone();
    let mut files = Vec::new();
    for wanted in expected_evidence_files(ctx.case, kind) {
        let safe = match safe_evidence_rel(&wanted) {
            Ok(safe) => safe,
            Err(error) => {
                outcome.error = Some(format!("EVAL_EXTERNAL_EVIDENCE_INVALID：{error}"));
                return;
            }
        };
        let path = evidence_dir.join(Path::new(&safe));
        match hash_file(&path) {
            Ok((size_bytes, sha256)) => files.push(EvidenceFile {
                path: wanted,
                size_bytes,
                sha256,
            }),
            Err(error) => {
                outcome.error = Some(format!("EVAL_EXTERNAL_EVIDENCE_INCOMPLETE：{error}"));
                return;
            }
        }
    }
    // The runner is the sole authority for file sizes and hashes. In
    // particular, driver-stdout.json cannot contain its own hash; requiring a
    // driver to predict that hash would make every real driver impossible.
    report.files = files.clone();
    report.evidence_dir = evidence_dir.to_string_lossy().into_owned();
    // The journal keeps bound artifact paths; the sandbox/artifact snapshot
    // holds the bytes, so contents are not duplicated into external_evidence.
    let mut value = match serde_json::to_value(&report) {
        Ok(value) => value,
        Err(error) => {
            outcome.error = Some(format!("EVAL_EXTERNAL_EVIDENCE_INVALID：{error}"));
            return;
        }
    };
    if let Some(map) = value.as_object_mut() {
        map.insert(
            "evidence_dir".to_string(),
            serde_json::Value::String(evidence_dir.to_string_lossy().into_owned()),
        );
        let _ = &bound_artifacts;
    }
    if let Err(error) = registry.validate(
        ctx.case,
        attempt.key,
        attempt.execution,
        opts.batch_label.as_deref(),
        opts.repair_attempts,
        &value,
    ) {
        outcome.error = Some(error);
        return;
    }
    outcome.tool_log.push(format!(
        "external_driver:{}:{}:{}",
        kind.as_str(),
        canonical_json(&serde_json::json!({"case": attempt.key.case_id, "nonce": nonce})),
        evidence_dir.to_string_lossy()
    ));
    outcome.external_evidence = Some(value);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product_eval::{AgentMode, FaultInjection, PRODUCT_EVAL_SCHEMA_VERSION};
    use std::collections::BTreeMap;

    fn case(category: EvalCategory, fault: Option<FaultInjection>) -> ProductEvalCase {
        ProductEvalCase {
            schema_version: PRODUCT_EVAL_SCHEMA_VERSION,
            id: "external-case".to_string(),
            category,
            title: "external".to_string(),
            instruction: "real".to_string(),
            inputs: vec![],
            allow_read: vec!["inputs/**".to_string()],
            allow_write: vec!["out/**".to_string()],
            expected_artifacts: vec!["out/report.md".to_string()],
            checkers: vec![],
            reference_outputs: BTreeMap::new(),
            timeout_secs: Some(30),
            max_model_calls: Some(4),
            repetitions: Some(1),
            allow_commands: vec![],
            failure_classes: vec![],
            expected_side_effects: vec![],
            fault_injection: fault,
        }
    }

    #[test]
    fn third_arm_fault_injection_requires_fault_driver() {
        let mut code = case(EvalCategory::Code, None);
        assert!(required_external_driver(&code).is_none());
        code.fault_injection = Some(FaultInjection {
            kind: "restart".to_string(),
            after_step: Some(2),
            note: String::new(),
        });
        assert_eq!(
            required_external_driver(&code),
            Some(ExternalDriverKind::FaultInjection)
        );
    }

    #[test]
    fn missing_registration_fails_closed() {
        let registry = ExternalDriverRegistry::new(None, None, None).unwrap();
        let desktop = case(EvalCategory::Desktop, None);
        let key = MatrixKey::new("external-case", AgentMode::Single, 0);
        let evidence = serde_json::json!({"schema_version": 1});
        let error = registry
            .validate(&desktop, &key, "live-agent", None, 0, &evidence)
            .unwrap_err();
        assert_eq!(error, "EVAL_REAL_DESKTOP_DRIVER_REQUIRED");
    }
}
