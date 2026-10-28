//! Template provenance is frozen with the actual role read, never inferred from a later head.
use async_trait::async_trait;
use owo_agent_core::cas_store::CasStore;
use owo_agent_core::goal::{Worker, WorkerRegistry};
use owo_agent_core::project_space_store::{ProjectSpaceStoreBackend, SqliteProjectSpaceStore};
use owo_agent_core::team_strategy::TeamSelectionMode;
use owo_agent_core::workswarm::{
    CreateTeamRequest, RoleSpec, RunMeta, TeamCoordinator, TeamTemplateRegistry, WorkSwarmError,
};
use owo_agent_core::workswarm::{PhaseOutcome, RoleWorker};
use owo_agent_protocol::{TeamMode, TeamTemplate};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

struct SnapshotWorker {
    contracts: Arc<Mutex<Vec<String>>>,
}
#[async_trait]
impl Worker for SnapshotWorker {
    fn name(&self) -> &str {
        "snapshot-fixture"
    }
    async fn run(&self, input: &Value) -> Result<String, String> {
        let context: Value = serde_json::from_str(
            input
                .get("text")
                .and_then(Value::as_str)
                .ok_or("missing context")?,
        )
        .map_err(|error| error.to_string())?;
        let contract = context["handoff_contract"]
            .as_str()
            .ok_or("missing role contract")?;
        self.contracts.lock().unwrap().push(contract.to_string());
        Ok("verified snapshot".into())
    }
}

struct Fixture {
    // Close SQLite before TempDir removes the fixture on Windows.
    coordinator: TeamCoordinator,
    temp: tempfile::TempDir,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let coordinator = Self::coordinator_at(temp.path());
        Self { temp, coordinator }
    }
    fn coordinator_at(root: &std::path::Path) -> TeamCoordinator {
        let store: Arc<dyn ProjectSpaceStoreBackend> =
            Arc::new(SqliteProjectSpaceStore::open(&root.join("space.db")).unwrap());
        TeamCoordinator::new(
            store,
            Arc::new(TeamTemplateRegistry::new(root.join("templates"))),
            CasStore::new(root.join("cas")).unwrap(),
            root.join("runs"),
        )
    }
    fn template(&self) -> TeamTemplate {
        serde_json::from_value(json!({
            "template_id":"source-fixture", "name":"Versioned fixture", "mode":"swarmflow",
            "applicability":"audit-source", "origin":"user", "created_at":"2026-10-25",
            "roles":[{"node_id":"maker-node", "role":"maker", "assignee":"worker",
                "worker":"echo", "handoff_contract":"revision-one-contract"}]
        }))
        .unwrap()
    }
    fn request(&self) -> CreateTeamRequest {
        CreateTeamRequest {
            goal_id: None,
            objective: "audit-source request".into(),
            mode: TeamMode::Swarmflow,
            template_id: Some("source-fixture".into()),
            roles: Vec::new(),
            budget: Value::Null,
            human_policy: None,
            strategy: Some(TeamSelectionMode::ForceTeam),
            model: None,
            parallel: false,
            max_agent_members: None,
            parent_context_snapshot: None,
        }
    }
}

#[tokio::test]
async fn template_source_freezes_before_preflight_and_survives_edit_delete_and_restart() {
    let fixture = Fixture::new();
    let template = fixture.template();
    let registry = fixture.coordinator.templates();
    let saved = registry.save_template_checked(&template, Some(0)).unwrap();
    let team = fixture
        .coordinator
        .create_team_run_with_preflight(&fixture.request(), |roles, _, _| {
            assert_eq!(
                roles[0].handoff_contract.as_deref(),
                Some("revision-one-contract")
            );
            let mut newer = template.clone();
            newer.roles[0].handoff_contract = Some("revision-two-contract".into());
            registry.save_template_checked(&newer, Some(1)).unwrap();
            Ok(())
        })
        .await
        .unwrap();
    let meta = fixture.coordinator.load_run_meta(&team.team_id).unwrap();
    let source = meta.template_source.as_ref().unwrap();
    assert_eq!(source.revision, 1);
    assert_eq!(source.content_hash, saved.content_hash.unwrap());
    assert_eq!(source.template_id, "source-fixture");
    assert!(!source.roles_overridden);
    assert_eq!(
        meta.roles[0].handoff_contract.as_deref(),
        Some("revision-one-contract")
    );
    assert_eq!(
        registry
            .get_revision("source-fixture")
            .unwrap()
            .unwrap()
            .revision,
        2
    );
    registry
        .delete_template_checked("source-fixture", Some(2))
        .unwrap();
    let restarted = Fixture::coordinator_at(fixture.temp.path());
    let recovered = restarted.load_run_meta(&team.team_id).unwrap();
    assert_eq!(recovered.template_source, meta.template_source);
    assert_eq!(
        recovered.roles[0].handoff_contract,
        meta.roles[0].handoff_contract
    );
    assert!(restarted.load_run_state(&team.team_id).is_ok());
    let contracts = Arc::new(Mutex::new(Vec::new()));
    let workers = WorkerRegistry::new();
    workers.register(Arc::new(RoleWorker::new(
        Arc::new(restarted.clone()),
        team.team_id.clone(),
        "m-maker".into(),
        "maker".into(),
        Arc::new(SnapshotWorker {
            contracts: Arc::clone(&contracts),
        }),
    )));
    let mut completed = false;
    for _ in 0..3 {
        match restarted.run_phase(&team.team_id, &workers).await.unwrap() {
            PhaseOutcome::Done => {
                completed = true;
                break;
            }
            PhaseOutcome::MoreReady => {}
            other => panic!("snapshot execution did not complete: {other:?}"),
        }
    }
    assert!(completed);
    assert_eq!(
        *contracts.lock().unwrap(),
        vec!["revision-one-contract".to_string()]
    );
    assert_eq!(
        restarted
            .load_run_meta(&team.team_id)
            .unwrap()
            .template_source,
        meta.template_source
    );
    assert!(matches!(
        restarted.create_team_run(&fixture.request()).await,
        Err(WorkSwarmError::NotFound(_))
    ));
}

#[tokio::test]
async fn template_source_auto_match_carries_the_matching_revision_and_hash() {
    let fixture = Fixture::new();
    let saved = fixture
        .coordinator
        .templates()
        .save_template_checked(&fixture.template(), Some(0))
        .unwrap();
    let mut request = fixture.request();
    request.template_id = None;
    let team = fixture.coordinator.create_team_run(&request).await.unwrap();
    let meta = fixture.coordinator.load_run_meta(&team.team_id).unwrap();
    let source = meta.template_source.unwrap();
    assert_eq!(team.template_id.as_deref(), Some("source-fixture"));
    assert_eq!(source.revision, saved.revision);
    assert_eq!(Some(source.content_hash), saved.content_hash);
    assert!(!source.roles_overridden);
}

#[tokio::test]
async fn template_source_marks_explicit_roles_and_request_model_and_single_ignores_template() {
    let fixture = Fixture::new();
    fixture
        .coordinator
        .templates()
        .save_template_checked(&fixture.template(), Some(0))
        .unwrap();
    let mut request = fixture.request();
    let mut role = RoleSpec::agent("replacement");
    role.assignee = "worker".into();
    role.worker = Some("echo".into());
    request.roles = vec![role];
    request.model = Some("request-default-model".into());
    let team = fixture.coordinator.create_team_run(&request).await.unwrap();
    let meta = fixture.coordinator.load_run_meta(&team.team_id).unwrap();
    let source = meta.template_source.unwrap();
    assert!(source.roles_overridden);
    assert_eq!(
        source.requested_model.as_deref(),
        Some("request-default-model")
    );
    assert_eq!(meta.roles[0].role, "replacement");
    request.mode = TeamMode::Single;
    request.strategy = None;
    request.template_id = Some("missing-and-ignored".into());
    let single = fixture.coordinator.create_team_run(&request).await.unwrap();
    let meta = fixture.coordinator.load_run_meta(&single.team_id).unwrap();
    assert!(single.template_id.is_none());
    assert!(meta.template_source.is_none());
}

#[tokio::test]
async fn template_source_legacy_sidecar_stays_unknown_instead_of_borrowing_latest_revision() {
    let fixture = Fixture::new();
    fixture
        .coordinator
        .templates()
        .save_template_checked(&fixture.template(), Some(0))
        .unwrap();
    let team = fixture
        .coordinator
        .create_team_run(&fixture.request())
        .await
        .unwrap();
    let path = fixture
        .coordinator
        .run_dir()
        .join(format!("{}-meta.json", team.team_id));
    let mut old: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    old.as_object_mut().unwrap().remove("template_source");
    std::fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();
    let mut newer = fixture.template();
    newer.name = "Updated after legacy run".into();
    fixture
        .coordinator
        .templates()
        .save_template_checked(&newer, Some(1))
        .unwrap();
    let restarted = Fixture::coordinator_at(fixture.temp.path());
    let meta = restarted.load_run_meta(&team.team_id).unwrap();
    assert!(meta.template_source.is_none());
    assert_eq!(meta.template_id.as_deref(), Some("source-fixture"));
    assert!(serde_json::from_value::<RunMeta>(old)
        .unwrap()
        .template_source
        .is_none());
}

#[tokio::test]
async fn template_source_corrupt_registry_fails_creation_instead_of_running_default_roles() {
    let fixture = Fixture::new();
    let path = fixture
        .temp
        .path()
        .join("templates/workflow-templates.sqlite3");
    std::fs::write(path, b"not a sqlite database").unwrap();
    let mut request = fixture.request();
    request.mode = TeamMode::Team;
    request.template_id = None;
    let error = fixture
        .coordinator
        .create_team_run(&request)
        .await
        .unwrap_err();
    assert!(matches!(error, WorkSwarmError::Run(_)), "{error}");
    assert!(fixture
        .coordinator
        .list_team_runs()
        .await
        .unwrap()
        .is_empty());
}
