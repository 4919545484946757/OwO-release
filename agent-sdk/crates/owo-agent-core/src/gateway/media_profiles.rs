//! Host-owned media profiles. Only credential references are persisted.
//! The data root comes from the Daemon, never from a workspace or model argument.
use super::media_config::{MediaConnection, MediaService};
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    time::Duration,
};

#[path = "media_profiles_checks.rs"]
mod checks;
pub use checks::{check_profile_tcp, checked_status, MediaCheckRequest, MediaConnectionCheck};

const MAX_BYTES: usize = 256 * 1024;
const MAX_REVISION: u64 = 9_007_199_254_740_991; // Exact in JavaScript and SQLite.
const FILE: &str = "media-connections.sqlite3";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MediaProfile {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub model: String,
    pub credential_env: Option<String>,
    pub services: Vec<MediaService>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MediaProfileCatalog {
    pub schema_version: u32,
    /// Compare-and-swap revision. A save submits the revision it read.
    pub revision: u64,
    pub profiles: Vec<MediaProfile>,
    pub assignments: BTreeMap<MediaService, String>,
}
impl Default for MediaProfileCatalog {
    fn default() -> Self {
        Self {
            schema_version: 1,
            revision: 0,
            profiles: Vec::new(),
            assignments: BTreeMap::new(),
        }
    }
}
impl MediaProfileCatalog {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 || self.revision > MAX_REVISION || self.profiles.len() > 64 {
            return Err("media_profiles/invalid_catalog: 版本、修订号或连接数量无效".into());
        }
        let encoded = serde_json::to_vec(self).map_err(|_| "media_profiles/invalid_catalog")?;
        if encoded.len() > MAX_BYTES {
            return Err("media_profiles/too_large: 配置超过 256 KiB".into());
        }
        let mut ids = BTreeSet::new();
        for profile in &self.profiles {
            if profile.id.is_empty()
                || profile.id.len() > 64
                || !profile
                    .id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
                || !ids.insert(&profile.id)
            {
                return Err(
                    "media_profiles/invalid_id: 连接 ID 必须唯一且仅含字母、数字、下划线或短横线"
                        .into(),
                );
            }
            if profile.name.trim().is_empty()
                || profile.name.len() > 128
                || profile.name.chars().any(char::is_control)
                || profile.model.trim().is_empty()
                || profile.model.len() > 256
                || profile.model.chars().any(char::is_control)
                || profile.base_url.trim().is_empty()
                || profile.base_url.len() > 2048
                || profile.services.is_empty()
                || profile.services.len() > 2
                || profile.services.iter().collect::<BTreeSet<_>>().len() != profile.services.len()
            {
                return Err(
                    "media_profiles/invalid_profile: 名称、模型、端点或服务声明无效".into(),
                );
            }
            if let Some(name) = &profile.credential_env {
                // Desktop vault references are bound to a specific chat connection.
                // Do not reinterpret them as generic media credentials or bypass that binding.
                if name
                    .to_ascii_uppercase()
                    .starts_with("OWO_CUSTOM_MODEL_SECRET_")
                {
                    return Err("media_profiles/bound_credential: 桌面保险库引用尚未接入媒体连接，不能重绑定端点或模型".into());
                }
                if name.is_empty()
                    || name.len() > 128
                    || !name.bytes().enumerate().all(|(i, c)| {
                        c == b'_' || c.is_ascii_alphabetic() || i > 0 && c.is_ascii_digit()
                    })
                {
                    return Err(
                        "media_profiles/invalid_credential_reference: 只能保存环境变量名称".into(),
                    );
                }
            }
            // Validate URL/model/reference without reading credentials or probing a service.
            profile.resolve_with(profile.services[0], |_| {
                Some("validation-placeholder".into())
            })?;
        }
        for (service, id) in &self.assignments {
            if !self
                .profiles
                .iter()
                .any(|p| &p.id == id && p.services.contains(service))
            {
                return Err(
                    "media_profiles/invalid_assignment: 服务指向不存在或不兼容的连接".into(),
                );
            }
        }
        Ok(())
    }
    pub fn resolve(&self, service: MediaService) -> Result<MediaConnection, String> {
        self.resolve_with(service, |name| std::env::var(name).ok())
    }
    pub fn status(&self, service: MediaService) -> Value {
        let mut view = match self.resolve(service) {
            Ok(connection) => connection.status(),
            Err(reason) => invalid_status(service, reason),
        };
        view["catalog_revision"] = self.revision.into();
        view["profile_id"] = self
            .assignments
            .get(&service)
            .cloned()
            .map(Value::String)
            .unwrap_or(Value::Null);
        view
    }

    pub fn resolve_with(
        &self,
        service: MediaService,
        lookup: impl Fn(&str) -> Option<String>,
    ) -> Result<MediaConnection, String> {
        let Some(id) = self.assignments.get(&service) else {
            return MediaConnection::from_lookup(service, lookup);
        };
        let profile = self
            .profiles
            .iter()
            .find(|p| &p.id == id && p.services.contains(&service))
            .ok_or_else(|| "media_profiles/invalid_assignment: 服务连接不存在".to_string())?;
        profile.resolve_with(service, lookup)
    }
}
impl MediaProfile {
    fn resolve_with(
        &self,
        service: MediaService,
        lookup: impl Fn(&str) -> Option<String>,
    ) -> Result<MediaConnection, String> {
        let (base, model) = match service {
            MediaService::ImageGeneration => ("OWO_IMAGE_BASE_URL", "OWO_IMAGE_MODEL"),
            MediaService::TextToSpeech => ("OWO_TTS_BASE_URL", "OWO_TTS_MODEL"),
        };
        let key = match &self.credential_env {
            Some(name) => Some(
                lookup(name)
                    .map(|v| v.trim().to_string())
                    .filter(|v| !v.is_empty())
                    .ok_or_else(|| {
                        "media/credential_reference_missing: 指定的凭据环境变量未设置".to_string()
                    })?,
            ),
            None => None,
        };
        // Build endpoint metadata with no ambient credentials, then attach the explicit reference.
        // Reading the reference independently also permits names matching an endpoint/model variable.
        MediaConnection::from_lookup(service, |name| {
            if name == base {
                Some(self.base_url.clone())
            } else if name == model {
                Some(self.model.clone())
            } else {
                None
            }
        })
        .map(|connection| connection.with_named_profile(self.credential_env.clone(), key))
    }
}

#[derive(Clone)]
pub struct MediaProfileStore {
    path: PathBuf,
}
impl MediaProfileStore {
    pub fn new(data_root: &Path) -> Self {
        Self {
            path: data_root.join(FILE),
        }
    }
    fn open(&self, write: bool) -> Result<Connection, String> {
        let flags = if write {
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE
        } else {
            OpenFlags::SQLITE_OPEN_READ_ONLY
        };
        let db = Connection::open_with_flags(&self.path, flags).map_err(|_| storage_error())?;
        db.busy_timeout(Duration::from_secs(2))
            .map_err(|_| storage_error())?;
        Ok(db)
    }
    pub fn load(&self) -> Result<MediaProfileCatalog, String> {
        match self.path.try_exists() {
            Ok(false) => return Ok(MediaProfileCatalog::default()),
            Ok(true) => {}
            Err(_) => return Err(storage_error()),
        }
        Self::read(&self.open(false)?)
    }
    fn read(db: &Connection) -> Result<MediaProfileCatalog, String> {
        let row: Option<(i64, String)> = db.query_row("SELECT revision, CASE WHEN length(document) <= 262144 THEN document ELSE NULL END FROM media_catalog WHERE singleton=1", [], |row| Ok((row.get(0)?, row.get(1)?))).optional().map_err(|_| storage_error())?;
        let Some((revision, document)) = row else {
            return Ok(MediaProfileCatalog::default());
        };
        let catalog: MediaProfileCatalog =
            serde_json::from_str(&document).map_err(|_| storage_error())?;
        catalog.validate().map_err(|_| storage_error())?;
        if revision < 0 || catalog.revision != revision as u64 {
            return Err(storage_error());
        }
        Ok(catalog)
    }
    pub fn save(&self, mut proposed: MediaProfileCatalog) -> Result<MediaProfileCatalog, String> {
        proposed.validate()?;
        if proposed.revision == MAX_REVISION {
            return Err("media_profiles/invalid_catalog: 修订号已耗尽".into());
        }
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|_| storage_error())?;
        }
        let mut db = self.open(true)?;
        db.execute_batch("CREATE TABLE IF NOT EXISTS media_catalog (singleton INTEGER PRIMARY KEY CHECK(singleton=1), revision INTEGER NOT NULL, document TEXT NOT NULL)").map_err(|_| storage_error())?;
        let transaction = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| storage_error())?;
        let current = Self::read(&transaction)?;
        if proposed.revision != current.revision {
            return Err("media_profiles/conflict: 配置已被修改，请重新读取后合并".into());
        }
        proposed.revision += 1;
        let document = serde_json::to_string(&proposed).map_err(|_| storage_error())?;
        if document.len() > MAX_BYTES {
            return Err("media_profiles/too_large: 配置超过 256 KiB".into());
        }
        transaction.execute("INSERT INTO media_catalog(singleton, revision, document) VALUES(1, ?1, ?2) ON CONFLICT(singleton) DO UPDATE SET revision=excluded.revision, document=excluded.document", rusqlite::params![proposed.revision as i64, document]).map_err(|_| storage_error())?;
        transaction.commit().map_err(|_| storage_error())?;
        Ok(proposed)
    }
    pub fn resolve(&self, service: MediaService) -> Result<MediaConnection, String> {
        self.load()?.resolve(service)
    }
    pub fn status(&self, service: MediaService) -> Value {
        match self.load() {
            Ok(catalog) => match self.checks(catalog.revision) {
                Ok(checks) => checked_status(&catalog, service, &checks),
                Err(reason) => invalid_status(service, reason),
            },
            Err(reason) => invalid_status(service, reason),
        }
    }
}
fn storage_error() -> String {
    "media_profiles/storage_error: 无法读取或保存宿主媒体配置；未回退到其他连接".into()
}
pub fn invalid_status(service: MediaService, reason: String) -> Value {
    serde_json::json!({"service":service.name(), "status":"invalid_configuration", "configured":false, "connected":null, "verified":null, "last_verified_at":null, "endpoint_origin":null, "model":null, "configuration_source":null, "credential_source":null, "profile_id":null, "catalog_revision":null, "reason":reason})
}

#[cfg(test)]
mod tests {
    use super::*;
    fn catalog() -> MediaProfileCatalog {
        MediaProfileCatalog {
            profiles: vec![MediaProfile {
                id: "speech".into(),
                name: "本地语音".into(),
                base_url: "http://127.0.0.1:9000/v1".into(),
                model: "voice-fixture".into(),
                credential_env: None,
                services: vec![MediaService::TextToSpeech],
            }],
            assignments: BTreeMap::from([(MediaService::TextToSpeech, "speech".into())]),
            ..Default::default()
        }
    }
    #[test]
    fn saves_reload_and_rejects_stale_writes_without_losing_the_current_selection() {
        let temp = tempfile::tempdir().unwrap();
        let store = MediaProfileStore::new(temp.path());
        assert_eq!(store.load().unwrap(), MediaProfileCatalog::default());
        assert!(!temp.path().join(FILE).exists());
        let saved = store.save(catalog()).unwrap();
        assert_eq!(saved.revision, 1);
        assert_eq!(MediaProfileStore::new(temp.path()).load().unwrap(), saved);
        assert!(store
            .save(catalog())
            .unwrap_err()
            .starts_with("media_profiles/conflict"));
        assert_eq!(store.load().unwrap(), saved);
        let status = store.status(MediaService::TextToSpeech);
        assert_eq!(status["profile_id"], "speech");
        assert_eq!(status["model"], "voice-fixture");
        assert!(status["verified"].is_null());
    }
    #[test]
    fn assigned_profiles_never_fall_back_to_ambient_credentials_and_missing_refs_fail() {
        let mut catalog = catalog();
        catalog.profiles[0].base_url = "https://speech.example/v1".into();
        let connection = catalog
            .resolve_with(MediaService::TextToSpeech, |_| {
                Some("ambient-fixture".into())
            })
            .unwrap();
        assert!(!connection.configured());
        assert!(connection
            .require_configured()
            .unwrap_err()
            .contains("命名连接"));
        assert!(connection.api_key().is_none());
        catalog.profiles[0].credential_env = Some("SPEECH_FIXTURE_KEY".into());
        assert!(catalog
            .resolve_with(MediaService::TextToSpeech, |_| None)
            .is_err());
        let connection = catalog
            .resolve_with(MediaService::TextToSpeech, |name| {
                (name == "SPEECH_FIXTURE_KEY").then(|| "reference-fixture".into())
            })
            .unwrap();
        assert_eq!(connection.api_key(), Some("reference-fixture"));
        assert!(!connection
            .status()
            .to_string()
            .contains("reference-fixture"));
    }
    #[test]
    fn rejects_unknown_secret_fields_dangling_or_incompatible_assignments_and_unsafe_urls() {
        let mut json = serde_json::to_value(catalog()).unwrap();
        json["profiles"][0]["api_key"] = "fixture-secret".into();
        assert!(serde_json::from_value::<MediaProfileCatalog>(json).is_err());
        let mut c = catalog();
        c.assignments
            .insert(MediaService::ImageGeneration, "speech".into());
        assert!(c.validate().is_err());
        c.assignments.remove(&MediaService::ImageGeneration);
        c.profiles[0].base_url = "https://user:fixture-secret@example.com/v1".into();
        let error = c.validate().unwrap_err();
        assert!(!error.contains("fixture-secret"));
        c.profiles[0].base_url = "http://localhost/v1".into();
        c.profiles[0].base_url = "   ".into();
        assert!(
            c.validate().is_err(),
            "An empty endpoint must not select a remote default"
        );
        c.profiles[0].base_url = "http://localhost/v1".into();
        c.profiles[0].credential_env = Some("OWO_CUSTOM_MODEL_SECRET_fixture".into());
        assert!(c
            .validate()
            .unwrap_err()
            .starts_with("media_profiles/bound_credential"));
        c.profiles[0].credential_env = Some("owo_custom_model_secret_fixture".into());
        assert!(c
            .validate()
            .unwrap_err()
            .starts_with("media_profiles/bound_credential"));
        c.profiles[0].credential_env = None;
        c.profiles.push(c.profiles[0].clone());
        assert!(c.validate().is_err());
    }
    #[test]
    fn unassigned_services_preserve_environment_compatibility_but_corrupt_store_never_falls_back() {
        let c = catalog();
        assert_eq!(
            c.resolve_with(MediaService::ImageGeneration, |name| (name
                == "OWO_IMAGE_BASE_URL")
                .then(|| "http://localhost:8000/v1".into()))
                .unwrap()
                .base_url(),
            "http://localhost:8000/v1"
        );
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join(FILE), b"corrupt database").unwrap();
        let store = MediaProfileStore::new(temp.path());
        assert!(store.resolve(MediaService::TextToSpeech).is_err());
        assert_eq!(
            store.status(MediaService::TextToSpeech)["status"],
            "invalid_configuration"
        );
    }
    #[test]
    fn concurrent_saves_use_transactional_compare_and_swap() {
        let temp = tempfile::tempdir().unwrap();
        let store = MediaProfileStore::new(temp.path());
        let initial = store.save(catalog()).unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let handles = (0..2)
            .map(|i| {
                let store = store.clone();
                let barrier = barrier.clone();
                let mut proposed = initial.clone();
                proposed.profiles[0].name = format!("writer-{i}");
                std::thread::spawn(move || {
                    barrier.wait();
                    store.save(proposed)
                })
            })
            .collect::<Vec<_>>();
        let results = handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
        assert!(results
            .iter()
            .filter_map(|r| r.as_ref().err())
            .all(|e| e.starts_with("media_profiles/conflict")));
        assert_eq!(store.load().unwrap().revision, 2);
    }
}
