//! Durable delivery records keyed by a verified project room and task thread.

mod files;
pub(super) mod tools;

use owo_agent_core::CasStore;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use files::{load_content, snapshot, target_path, write_atomic, LIMIT};

const MAX_FILES: usize = 12;

#[derive(Debug)]
pub(super) enum DeliveryError {
    Invalid(&'static str),
    Conflict(&'static str),
    Missing,
    Unavailable,
}

impl From<io::Error> for DeliveryError {
    fn from(error: io::Error) -> Self {
        tracing::warn!(kind = ?error.kind(), "OwOBot delivery storage error");
        Self::Unavailable
    }
}

impl DeliveryError {
    pub(super) fn code(&self) -> &'static str {
        match self {
            Self::Invalid(code) | Self::Conflict(code) => code,
            Self::Missing => "delivery_not_found",
            Self::Unavailable => "delivery_storage_unavailable",
        }
    }
}

type DeliveryResult<T> = Result<T, DeliveryError>;

#[derive(Clone)]
pub(super) struct Store {
    directory: PathBuf,
    pub(super) workspace: PathBuf,
    group_id: String,
    cas: CasStore,
    gate: Arc<Mutex<()>>,
}

#[derive(Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Actor {
    pub(super) bot_id: String,
    pub(super) role: String,
    pub(super) session_id: String,
    pub(super) turn_id: String,
    pub(super) assignment_id: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Task {
    task_id: String,
    pub(super) thread_id: String,
    group_id: String,
    turns: Vec<Actor>,
    revisions: Vec<Revision>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Revision {
    number: u64,
    files: Vec<Change>,
    producer: Actor,
    review: Option<Review>,
    decision: Option<Decision>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Change {
    path: String,
    base: Option<String>,
    result: String,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Review {
    reviewed_at: String,
    approved: bool,
    note: String,
    actor: Actor,
}

#[derive(Clone, Copy, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum Action {
    Apply,
    Reject,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Decision {
    decided_at: String,
    action: Action,
    key: String,
    finished: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct StageRequest {
    pub(super) base_revision: u64,
    pub(super) files: Vec<ProposedFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProposedFile {
    pub(super) path: String,
    pub(super) text: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ReviewRequest {
    pub(super) revision: u64,
    pub(super) approved: bool,
    pub(super) note: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct DecisionRequest {
    pub(super) revision: u64,
    pub(super) action: Action,
    pub(super) idempotency_key: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct View {
    pub(super) task_id: String,
    thread_id: String,
    turns: Vec<Actor>,
    pub(super) revision: u64,
    revisions: Vec<u64>,
    status: &'static str,
    files: Vec<Preview>,
    review: Option<Review>,
    pub(super) replayed: bool,
    decided_at: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Preview {
    path: String,
    before: Option<String>,
    after: String,
    sha256: String,
}

impl Store {
    pub(super) fn new(
        project: &Path,
        workspace: PathBuf,
        group_id: String,
        gate: Arc<Mutex<()>>,
    ) -> DeliveryResult<Self> {
        let directory = project.join("deliveries");
        let cas =
            CasStore::new(directory.join("contents")).map_err(|_| DeliveryError::Unavailable)?;
        Ok(Self {
            directory,
            workspace,
            group_id,
            cas,
            gate,
        })
    }

    fn ledger(&self, thread: &str) -> DeliveryResult<PathBuf> {
        if thread.is_empty()
            || thread.len() > 128
            || !thread
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        {
            return Err(DeliveryError::Invalid("invalid_task"));
        }
        Ok(self.directory.join(format!("{thread}.json")))
    }

    fn read(&self, thread: &str) -> DeliveryResult<Task> {
        let raw = match fs::read(self.ledger(thread)?) {
            Ok(raw) => raw,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(DeliveryError::Missing)
            }
            Err(error) => return Err(error.into()),
        };
        let task: Task = serde_json::from_slice(&raw).map_err(|_| DeliveryError::Unavailable)?;
        if task.thread_id != thread || task.group_id != self.group_id {
            return Err(DeliveryError::Missing);
        }
        Ok(task)
    }

    fn save(&self, task: &Task) -> DeliveryResult<()> {
        let bytes = serde_json::to_vec(task).map_err(|_| DeliveryError::Unavailable)?;
        write_atomic(&self.ledger(&task.thread_id)?, &bytes, true)?;
        Ok(())
    }

    pub(super) fn bind(&self, thread: &str, actor: Actor) -> DeliveryResult<()> {
        let _guard = self.gate.lock().map_err(|_| DeliveryError::Unavailable)?;
        let mut task = match self.read(thread) {
            Ok(task) => task,
            Err(DeliveryError::Missing) => Task {
                task_id: format!("owobot-{thread}"),
                thread_id: thread.to_owned(),
                group_id: self.group_id.clone(),
                turns: Vec::new(),
                revisions: Vec::new(),
            },
            Err(error) => return Err(error),
        };
        if let Some(previous) = task
            .turns
            .iter()
            .find(|candidate| candidate.turn_id == actor.turn_id)
        {
            return if previous == &actor {
                Ok(())
            } else {
                Err(DeliveryError::Conflict("turn_already_bound"))
            };
        }
        task.turns.push(actor);
        self.save(&task)
    }

    pub(super) fn stage(
        &self,
        thread: &str,
        actor: Actor,
        request: StageRequest,
    ) -> DeliveryResult<View> {
        let _guard = self.gate.lock().map_err(|_| DeliveryError::Unavailable)?;
        let mut task = self.read(thread)?;
        if actor.role != "worker" || !task.turns.contains(&actor) {
            return Err(DeliveryError::Invalid("delivery_requires_worker"));
        }
        let base_revision = task.revisions.last().map_or(0, |revision| revision.number);
        if base_revision != request.base_revision {
            return Err(DeliveryError::Conflict("delivery_revision_changed"));
        }
        if task
            .revisions
            .last()
            .and_then(|revision| revision.decision.as_ref())
            .is_some_and(|decision| !decision.finished)
        {
            return Err(DeliveryError::Conflict("delivery_apply_in_progress"));
        }
        let total = request
            .files
            .iter()
            .try_fold(0usize, |sum, file| sum.checked_add(file.text.len()));
        if request.files.is_empty()
            || request.files.len() > MAX_FILES
            || total.is_none_or(|bytes| bytes > LIMIT)
        {
            return Err(DeliveryError::Invalid("invalid_delivery_size"));
        }
        let mut seen = HashSet::new();
        let mut files = Vec::new();
        for proposed in request.files {
            let safe = target_path(&self.workspace, &proposed.path)?;
            let identity = safe.to_string_lossy();
            let identity = if cfg!(windows) {
                identity.to_lowercase()
            } else {
                identity.into_owned()
            };
            if !seen.insert(identity) {
                return Err(DeliveryError::Invalid("duplicate_delivery_path"));
            }
            let before = snapshot(&safe)?;
            if before.as_deref() == Some(proposed.text.as_bytes()) {
                continue;
            }
            let base = before
                .map(|bytes| self.cas.put(&bytes))
                .transpose()
                .map_err(|_| DeliveryError::Unavailable)?;
            let result = self
                .cas
                .put(proposed.text.as_bytes())
                .map_err(|_| DeliveryError::Unavailable)?;
            files.push(Change {
                path: proposed.path,
                base,
                result,
            });
        }
        if files.is_empty() {
            return Err(DeliveryError::Invalid("delivery_has_no_changes"));
        }
        let number = base_revision + 1;
        self.cas
            .save_refs()
            .map_err(|_| DeliveryError::Unavailable)?;
        task.revisions.push(Revision {
            number,
            files,
            producer: actor,
            review: None,
            decision: None,
        });
        self.save(&task)?;
        self.view(task, Some(number), false)
    }

    pub(super) fn review(
        &self,
        thread: &str,
        actor: Actor,
        request: ReviewRequest,
    ) -> DeliveryResult<View> {
        let _guard = self.gate.lock().map_err(|_| DeliveryError::Unavailable)?;
        let mut task = self.read(thread)?;
        if actor.role != "reviewer" || !task.turns.contains(&actor) {
            return Err(DeliveryError::Invalid("delivery_requires_reviewer"));
        }
        let note = request.note.trim();
        if note.is_empty() || note.len() > 4096 {
            return Err(DeliveryError::Invalid("invalid_review_note"));
        }
        let revision = current(&mut task, request.revision)?;
        if revision.producer.bot_id == actor.bot_id
            || revision.decision.is_some()
            || revision.review.is_some()
        {
            return Err(DeliveryError::Conflict("delivery_cannot_be_reviewed"));
        }
        revision.review = Some(Review {
            reviewed_at: chrono::Utc::now().to_rfc3339(),
            approved: request.approved,
            note: note.into(),
            actor,
        });
        self.save(&task)?;
        self.view(task, Some(request.revision), false)
    }

    pub(super) fn inspect(&self, thread: &str, revision: Option<u64>) -> DeliveryResult<View> {
        let _guard = self.gate.lock().map_err(|_| DeliveryError::Unavailable)?;
        let task = self.read(thread)?;
        self.view(task, revision, false)
    }

    pub(super) fn decide(&self, thread: &str, request: DecisionRequest) -> DeliveryResult<View> {
        let _guard = self.gate.lock().map_err(|_| DeliveryError::Unavailable)?;
        if request.idempotency_key.is_empty()
            || request.idempotency_key.len() > 128
            || request.idempotency_key.chars().any(char::is_control)
        {
            return Err(DeliveryError::Invalid("invalid_decision_key"));
        }
        let mut task = self.read(thread)?;
        let revision = task
            .revisions
            .iter()
            .find(|revision| revision.number == request.revision)
            .ok_or(DeliveryError::Conflict("delivery_revision_changed"))?;
        if let Some(decision) = &revision.decision {
            if decision.action != request.action || decision.key != request.idempotency_key {
                return Err(DeliveryError::Conflict("delivery_already_decided"));
            }
            if decision.finished {
                return self.view(task, Some(request.revision), true);
            }
        }
        let revision = current(&mut task, request.revision)?;
        if request.action == Action::Reject {
            revision.decision = Some(Decision {
                decided_at: chrono::Utc::now().to_rfc3339(),
                action: request.action,
                key: request.idempotency_key,
                finished: true,
            });
            self.save(&task)?;
            return self.view(task, Some(request.revision), false);
        }
        if !revision
            .review
            .as_ref()
            .is_some_and(|review| review.approved)
        {
            return Err(DeliveryError::Conflict("delivery_requires_review"));
        }
        let resuming = revision.decision.is_some();
        let changes = self.prepare_apply(revision, resuming)?;
        if !resuming {
            revision.decision = Some(Decision {
                decided_at: chrono::Utc::now().to_rfc3339(),
                action: request.action,
                key: request.idempotency_key,
                finished: false,
            });
            self.save(&task)?;
        }
        for (change, text) in changes {
            let path = target_path(&self.workspace, &change.path)?;
            let actual = snapshot(&path)?.map(|bytes| CasStore::hash_of(&bytes));
            if actual.as_deref() == Some(change.result.as_str()) {
                continue;
            }
            if actual != change.base {
                return Err(DeliveryError::Conflict("delivery_workspace_changed"));
            }
            write_atomic(&path, &text, change.base.is_some())?;
        }
        let revision = current(&mut task, request.revision)?;
        if let Some(decision) = revision.decision.as_mut() {
            decision.finished = true;
        }
        self.save(&task)?;
        self.view(task, Some(request.revision), false)
    }

    fn prepare_apply(
        &self,
        revision: &Revision,
        resuming: bool,
    ) -> DeliveryResult<Vec<(Change, Vec<u8>)>> {
        revision
            .files
            .iter()
            .map(|change| {
                let path = target_path(&self.workspace, &change.path)?;
                let actual = snapshot(&path)?.map(|bytes| CasStore::hash_of(&bytes));
                if !(actual == change.base || resuming && actual.as_deref() == Some(&change.result))
                {
                    return Err(DeliveryError::Conflict("delivery_workspace_changed"));
                }
                Ok((
                    Change {
                        path: change.path.clone(),
                        base: change.base.clone(),
                        result: change.result.clone(),
                    },
                    load_content(&self.cas, &change.result)?,
                ))
            })
            .collect()
    }

    fn view(&self, task: Task, number: Option<u64>, replayed: bool) -> DeliveryResult<View> {
        let mut view = View {
            task_id: task.task_id,
            thread_id: task.thread_id,
            turns: task.turns,
            revision: 0,
            revisions: task
                .revisions
                .iter()
                .map(|revision| revision.number)
                .collect(),
            status: "working",
            files: Vec::new(),
            review: None,
            replayed,
            decided_at: None,
        };
        let revision = match number {
            Some(number) => task
                .revisions
                .into_iter()
                .find(|revision| revision.number == number),
            None => task.revisions.into_iter().last(),
        };
        if number.is_some() && revision.is_none() {
            return Err(DeliveryError::Missing);
        }
        if let Some(revision) = revision {
            view.revision = revision.number;
            view.status = match (&revision.decision, &revision.review) {
                (
                    Some(Decision {
                        action: Action::Reject,
                        ..
                    }),
                    _,
                ) => "rejected",
                (Some(Decision { finished: true, .. }), _) => "applied",
                (Some(_), _) => "applying",
                (None, Some(Review { approved: true, .. })) => "reviewed",
                (None, Some(_)) => "changesRequested",
                _ => "staged",
            };
            view.decided_at = revision
                .decision
                .as_ref()
                .map(|decision| decision.decided_at.clone());
            view.review = revision.review;
            for change in revision.files {
                let as_text = |hash: &str| -> DeliveryResult<String> {
                    String::from_utf8(load_content(&self.cas, hash)?)
                        .map_err(|_| DeliveryError::Unavailable)
                };
                view.files.push(Preview {
                    path: change.path,
                    before: change.base.as_deref().map(as_text).transpose()?,
                    after: as_text(&change.result)?,
                    sha256: change.result,
                });
            }
        }
        Ok(view)
    }
}

fn current(task: &mut Task, number: u64) -> DeliveryResult<&mut Revision> {
    task.revisions
        .last_mut()
        .filter(|revision| revision.number == number)
        .ok_or(DeliveryError::Conflict("delivery_revision_changed"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const THREAD: &str = "thread-fixture";

    fn actor(role: &str) -> Actor {
        Actor {
            bot_id: format!("bot-{role}"),
            role: role.into(),
            session_id: format!("session-{role}"),
            turn_id: format!("turn-{role}"),
            assignment_id: None,
        }
    }

    fn store() -> (tempfile::TempDir, Store) {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        fs::create_dir(&workspace).unwrap();
        let store = Store::new(
            root.path(),
            workspace,
            "room-x".into(),
            Arc::new(Mutex::new(())),
        )
        .unwrap();
        for role in ["lead", "worker", "reviewer"] {
            store.bind(THREAD, actor(role)).unwrap();
        }
        (root, store)
    }

    fn stage_two(store: &Store) {
        fs::write(store.workspace.join("existing.md"), "base").unwrap();
        let proposal = StageRequest {
            base_revision: 0,
            files: vec![
                ProposedFile {
                    path: "existing.md".into(),
                    text: "edited".into(),
                },
                ProposedFile {
                    path: "new.csv".into(),
                    text: "created".into(),
                },
            ],
        };
        let view = store.stage(THREAD, actor("worker"), proposal).unwrap();
        assert_eq!(view.revision, 1);
        assert_eq!(view.files[0].before.as_deref(), Some("base"));
        assert_eq!(view.files[1].before, None);
    }

    fn apply() -> DecisionRequest {
        DecisionRequest {
            revision: 1,
            action: Action::Apply,
            idempotency_key: "approve-1".into(),
        }
    }

    fn review() -> ReviewRequest {
        ReviewRequest {
            revision: 1,
            approved: true,
            note: "Verified both files".into(),
        }
    }

    #[test]
    fn only_the_bound_reviewer_can_unlock_application_and_replay_is_readonly() {
        let (root, store) = store();
        stage_two(&store);
        assert!(matches!(
            store.review(THREAD, actor("worker"), review()),
            Err(DeliveryError::Invalid(_))
        ));
        assert!(matches!(
            store.decide(THREAD, apply()),
            Err(DeliveryError::Conflict("delivery_requires_review"))
        ));
        assert_eq!(
            fs::read_to_string(store.workspace.join("existing.md")).unwrap(),
            "base"
        );
        assert!(!store.workspace.join("new.csv").exists());
        store.review(THREAD, actor("reviewer"), review()).unwrap();
        assert_eq!(store.decide(THREAD, apply()).unwrap().status, "applied");
        assert_eq!(
            fs::read_to_string(store.workspace.join("existing.md")).unwrap(),
            "edited"
        );
        assert_eq!(
            fs::read_to_string(store.workspace.join("new.csv")).unwrap(),
            "created"
        );
        let restarted = Store::new(
            root.path(),
            store.workspace.clone(),
            "room-x".into(),
            Arc::new(Mutex::new(())),
        )
        .unwrap();
        fs::write(store.workspace.join("new.csv"), "subsequent user edit").unwrap();
        assert!(restarted.decide(THREAD, apply()).unwrap().replayed);
        assert_eq!(
            fs::read_to_string(store.workspace.join("new.csv")).unwrap(),
            "subsequent user edit"
        );
    }

    #[test]
    fn whole_revision_is_checked_before_writes_and_reject_never_touches_the_workspace() {
        let (_root, store) = store();
        stage_two(&store);
        store.review(THREAD, actor("reviewer"), review()).unwrap();
        fs::write(store.workspace.join("existing.md"), "user edit").unwrap();
        assert!(matches!(
            store.decide(THREAD, apply()),
            Err(DeliveryError::Conflict("delivery_workspace_changed"))
        ));
        assert_eq!(
            fs::read_to_string(store.workspace.join("existing.md")).unwrap(),
            "user edit"
        );
        assert!(!store.workspace.join("new.csv").exists());
        let reject = DecisionRequest {
            revision: 1,
            action: Action::Reject,
            idempotency_key: "reject-1".into(),
        };
        assert_eq!(store.decide(THREAD, reject).unwrap().status, "rejected");
        assert!(matches!(
            store.decide(THREAD, apply()),
            Err(DeliveryError::Conflict("delivery_already_decided"))
        ));
        assert!(!store.workspace.join("new.csv").exists());
    }

    #[test]
    fn resumes_a_journalled_apply_after_one_file_was_written() {
        let (root, store) = store();
        stage_two(&store);
        store.review(THREAD, actor("reviewer"), review()).unwrap();
        let mut task = store.read(THREAD).unwrap();
        let revision = task.revisions.last_mut().unwrap();
        revision.decision = Some(Decision {
            decided_at: "2026-11-16T00:00:00Z".into(),
            action: Action::Apply,
            key: "approve-1".into(),
            finished: false,
        });
        store.save(&task).unwrap();
        fs::write(store.workspace.join("existing.md"), "edited").unwrap();

        let resumed = Store::new(
            root.path(),
            store.workspace.clone(),
            "room-x".into(),
            Arc::new(Mutex::new(())),
        )
        .unwrap();
        assert_eq!(resumed.inspect(THREAD, None).unwrap().status, "applying");
        assert_eq!(resumed.decide(THREAD, apply()).unwrap().status, "applied");
        assert_eq!(
            fs::read_to_string(store.workspace.join("existing.md")).unwrap(),
            "edited"
        );
        assert_eq!(
            fs::read_to_string(store.workspace.join("new.csv")).unwrap(),
            "created"
        );
    }

    #[test]
    fn a_later_revision_retains_earlier_previews_and_decisions() {
        let (_root, store) = store();
        stage_two(&store);
        store.review(THREAD, actor("reviewer"), review()).unwrap();
        store.decide(THREAD, apply()).unwrap();
        let revision = store
            .stage(
                THREAD,
                actor("worker"),
                StageRequest {
                    base_revision: 1,
                    files: vec![ProposedFile {
                        path: "new.csv".into(),
                        text: "second revision".into(),
                    }],
                },
            )
            .unwrap();
        assert_eq!(revision.revisions, vec![1, 2]);
        assert_eq!(revision.status, "staged");
        let history = store.inspect(THREAD, Some(1)).unwrap();
        assert_eq!(history.files[1].after, "created");
        assert_eq!(history.status, "applied");
        assert!(matches!(
            store.inspect(THREAD, Some(3)),
            Err(DeliveryError::Missing)
        ));
        assert!(store.decide(THREAD, apply()).unwrap().replayed);
        assert_eq!(store.inspect(THREAD, None).unwrap().revision, 2);
    }

    #[test]
    fn forbids_worktree_control_paths_and_other_roles_from_staging() {
        let (_root, store) = store();
        for path in ["../escaped.txt", ".git/config", "new.csv:stream"] {
            let request = StageRequest {
                base_revision: 0,
                files: vec![ProposedFile {
                    path: path.into(),
                    text: "invalid".into(),
                }],
            };
            assert!(matches!(
                store.stage(THREAD, actor("worker"), request),
                Err(DeliveryError::Invalid(_))
            ));
        }
        let request = StageRequest {
            base_revision: 0,
            files: vec![ProposedFile {
                path: "safe.txt".into(),
                text: "unverified".into(),
            }],
        };
        assert!(matches!(
            store.stage(THREAD, actor("reviewer"), request),
            Err(DeliveryError::Invalid("delivery_requires_worker"))
        ));
        assert_eq!(store.inspect(THREAD, None).unwrap().revision, 0);
    }
}
