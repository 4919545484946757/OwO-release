//! Explicit, bounded TCP checks. Observations are scoped to configuration revisions.
use super::{storage_error, MediaProfileCatalog, MediaProfileStore, MediaService};
use rusqlite::TransactionBehavior;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MediaCheckRequest {
    pub expected_revision: u64,
    pub profile_id: String,
    pub service: MediaService,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MediaConnectionCheck {
    pub profile_id: String,
    pub service: MediaService,
    pub catalog_revision: u64,
    pub kind: String,
    pub connected: bool,
    pub started_at_ms: u64,
    pub checked_at_ms: u64,
    pub latency_ms: u64,
    pub code: String,
}
static CHECK_GATE: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);
fn now_ms() -> Result<u64, String> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|v| v.as_millis() as u64)
        .map_err(|_| "media_profiles/clock_error".into())
}
/// No provider authentication, model call or retry. The three-second timeout includes DNS.
pub async fn check_profile_tcp(
    store: MediaProfileStore,
    request: MediaCheckRequest,
) -> Result<MediaConnectionCheck, String> {
    let _permit = CHECK_GATE
        .try_acquire()
        .map_err(|_| "media_profiles/check_busy: 已有连接测试正在执行".to_string())?;
    let prepared = store.clone();
    let requested = request.clone();
    let profile = tokio::task::spawn_blocking(move || {
        let catalog = prepared.load()?;
        if catalog.revision != requested.expected_revision {
            return Err("media_profiles/conflict: 测试目标配置已变化".into());
        }
        catalog
            .profiles
            .into_iter()
            .find(|p| p.id == requested.profile_id && p.services.contains(&requested.service))
            .ok_or_else(|| {
                "media_profiles/invalid_assignment: 连接不存在或不支持此服务".to_string()
            })
    })
    .await
    .map_err(|_| storage_error())??;
    let url = super::super::media_config::media_network_url(
        &profile.base_url,
        super::super::media_config::media_cloud_enabled(),
    )?;
    let host = url
        .host_str()
        .ok_or("media/invalid_endpoint")?
        .trim_matches(['[', ']'])
        .to_string();
    let port = url
        .port_or_known_default()
        .ok_or("media/invalid_endpoint")?;
    let started_at_ms = now_ms()?;
    let start = std::time::Instant::now();
    let result = tokio::time::timeout(
        Duration::from_secs(3),
        tokio::net::TcpStream::connect((host.as_str(), port)),
    )
    .await;
    let (connected, code) = match result {
        Ok(Ok(stream)) => {
            drop(stream);
            (true, "media_profiles/tcp_reachable")
        }
        Ok(Err(_)) => (false, "media_profiles/tcp_unreachable"),
        Err(_) => (false, "media_profiles/tcp_timeout"),
    };
    let check = MediaConnectionCheck {
        profile_id: request.profile_id,
        service: request.service,
        catalog_revision: request.expected_revision,
        kind: "tcp".into(),
        connected,
        started_at_ms,
        checked_at_ms: now_ms()?,
        latency_ms: start.elapsed().as_millis() as u64,
        code: code.into(),
    };
    let persisted = check.clone();
    tokio::task::spawn_blocking(move || store.record_check(&persisted))
        .await
        .map_err(|_| storage_error())??;
    Ok(check)
}
pub fn checked_status(
    catalog: &MediaProfileCatalog,
    service: MediaService,
    checks: &[MediaConnectionCheck],
) -> Value {
    let mut view = catalog.status(service);
    if let Some(c) = checks.iter().find(|c| {
        c.catalog_revision == catalog.revision
            && catalog.assignments.get(&service) == Some(&c.profile_id)
            && c.service == service
            && c.kind == "tcp"
    }) {
        view["connected"] = c.connected.into();
        view["last_checked_at_ms"] = c.checked_at_ms.into();
        view["connection_check_kind"] = "tcp".into();
        view["connection_check_code"] = c.code.clone().into();
    }
    view
}
impl MediaProfileStore {
    pub fn checks(&self, revision: u64) -> Result<Vec<MediaConnectionCheck>, String> {
        if !self.path.try_exists().map_err(|_| storage_error())? {
            return Ok(Vec::new());
        }
        let db = self.open(false)?;
        let exists:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='media_checks')",[],|row|row.get(0)).map_err(|_|storage_error())?;
        if !exists {
            return Ok(Vec::new());
        }
        let mut statement=db.prepare("SELECT CASE WHEN length(document)<=4096 THEN document ELSE NULL END FROM media_checks WHERE revision=?1 LIMIT 129").map_err(|_|storage_error())?;
        let rows = statement
            .query_map([revision as i64], |row| row.get::<_, String>(0))
            .map_err(|_| storage_error())?;
        let mut checks = Vec::new();
        for row in rows {
            let check: MediaConnectionCheck =
                serde_json::from_str(&row.map_err(|_| storage_error())?)
                    .map_err(|_| storage_error())?;
            if check.kind != "tcp"
                || check.catalog_revision != revision
                || check.checked_at_ms < check.started_at_ms
                || checks.len() >= 128
            {
                return Err(storage_error());
            }
            checks.push(check);
        }
        Ok(checks)
    }
    fn record_check(&self, check: &MediaConnectionCheck) -> Result<(), String> {
        let mut db = self.open(true)?;
        db.execute_batch("CREATE TABLE IF NOT EXISTS media_checks(profile_id TEXT NOT NULL,service TEXT NOT NULL,revision INTEGER NOT NULL,started_at_ms INTEGER NOT NULL,document TEXT NOT NULL,PRIMARY KEY(profile_id,service))").map_err(|_|storage_error())?;
        let transaction = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| storage_error())?;
        let catalog = Self::read(&transaction)?;
        if catalog.revision != check.catalog_revision
            || !catalog
                .profiles
                .iter()
                .any(|p| p.id == check.profile_id && p.services.contains(&check.service))
        {
            return Err("media_profiles/conflict: 测试期间配置已变化，结果未保存".into());
        }
        let document = serde_json::to_string(check).map_err(|_| storage_error())?;
        let count=transaction.execute("INSERT INTO media_checks(profile_id,service,revision,started_at_ms,document) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(profile_id,service) DO UPDATE SET revision=excluded.revision,started_at_ms=excluded.started_at_ms,document=excluded.document WHERE media_checks.started_at_ms<=excluded.started_at_ms",rusqlite::params![check.profile_id,check.service.name(),check.catalog_revision as i64,check.started_at_ms as i64,document]).map_err(|_|storage_error())?;
        if count == 0 {
            return Err("media_profiles/conflict: 较新的测试结果已经保存".into());
        }
        transaction.commit().map_err(|_| storage_error())?;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn real_tcp_observation_survives_reopen_and_old_revisions_cannot_reappear() {
        let temp = tempfile::tempdir().unwrap();
        let store = MediaProfileStore::new(temp.path());
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let catalog = MediaProfileCatalog {
            profiles: vec![super::super::MediaProfile {
                id: "local".into(),
                name: "local".into(),
                base_url: format!("http://{}/v1", listener.local_addr().unwrap()),
                model: "fixture".into(),
                credential_env: None,
                services: vec![MediaService::TextToSpeech],
            }],
            assignments: std::collections::BTreeMap::from([(
                MediaService::TextToSpeech,
                "local".into(),
            )]),
            ..Default::default()
        };
        let saved = store.save(catalog).unwrap();
        assert!(store.checks(saved.revision).unwrap().is_empty());
        let check = check_profile_tcp(
            store.clone(),
            MediaCheckRequest {
                expected_revision: saved.revision,
                profile_id: "local".into(),
                service: MediaService::TextToSpeech,
            },
        )
        .await
        .unwrap();
        assert!(check.connected);
        let reopened = MediaProfileStore::new(temp.path());
        assert_eq!(reopened.checks(saved.revision).unwrap().len(), 1);
        assert_eq!(
            reopened.status(MediaService::TextToSpeech)["connected"],
            true
        );
        assert!(reopened.status(MediaService::TextToSpeech)["verified"].is_null());
        let mut newer = saved;
        newer.profiles[0].model = "changed".into();
        let newer = store.save(newer).unwrap();
        assert!(store.checks(newer.revision).unwrap().is_empty());
        assert!(store
            .record_check(&check)
            .unwrap_err()
            .starts_with("media_profiles/conflict"));
        assert!(store.status(MediaService::TextToSpeech)["connected"].is_null());
        let permit = CHECK_GATE.acquire().await.unwrap();
        assert!(check_profile_tcp(
            store.clone(),
            MediaCheckRequest {
                expected_revision: newer.revision,
                profile_id: "local".into(),
                service: MediaService::TextToSpeech
            }
        )
        .await
        .unwrap_err()
        .starts_with("media_profiles/check_busy"));
        drop(permit);
        assert!(check_profile_tcp(
            store,
            MediaCheckRequest {
                expected_revision: 0,
                profile_id: "local".into(),
                service: MediaService::TextToSpeech
            }
        )
        .await
        .unwrap_err()
        .starts_with("media_profiles/conflict"));
    }
}
