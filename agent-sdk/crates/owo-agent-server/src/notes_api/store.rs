use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use owo_agent_core::{load_doc, save_doc, NoteDoc, NoteIndex, NoteIndexer, SearchHit};
use serde_json::{json, Value};
// ----------------------------------------------------------------------------
// 存储：data_root 键控注册表（模块内单例）
// ----------------------------------------------------------------------------

static STORES: OnceLock<std::sync::Mutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<NoteStore>>>>> =
    OnceLock::new();

pub(super) fn stores(
) -> &'static std::sync::Mutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<NoteStore>>>> {
    STORES.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

pub(super) fn store_for(data_root: &Path) -> Arc<tokio::sync::Mutex<NoteStore>> {
    let mut map = stores()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    map.entry(data_root.to_path_buf())
        .or_insert_with(|| {
            Arc::new(tokio::sync::Mutex::new(NoteStore::new(
                data_root.join("notes"),
            )))
        })
        .clone()
}

/// 笔记存储：文档目录 + 清单 + 每文档全文索引。
pub(super) struct NoteStore {
    root: PathBuf,
    /// doc_id → 索引器（FTS5，db 位于 <id>/fts.db）。
    indexers: HashMap<String, NoteIndexer>,
}

impl NoteStore {
    pub(super) fn new(root: PathBuf) -> Self {
        Self {
            root,
            indexers: HashMap::new(),
        }
    }

    pub(super) fn doc_dir(&self, id: &str) -> PathBuf {
        self.root.join(id)
    }

    pub(super) fn ensure_root(&self) -> Result<(), String> {
        std::fs::create_dir_all(&self.root).map_err(|e| format!("创建笔记目录失败：{e}"))
    }

    pub(super) fn list(&mut self) -> Result<Vec<Value>, String> {
        self.ensure_root()?;
        // index.json 清单（损坏/缺失时按目录扫描重建）
        let index_path = self.root.join("index.json");
        if let Ok(content) = std::fs::read_to_string(&index_path) {
            if let Ok(list) = serde_json::from_str::<Vec<Value>>(&content) {
                return Ok(list);
            }
        }
        let mut list = Vec::new();
        for entry in std::fs::read_dir(&self.root).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let dir = entry.path();
            if !dir.is_dir() || dir.file_name().and_then(|n| n.to_str()) == Some("index.json") {
                continue;
            }
            if let Ok(doc) = load_doc(&dir) {
                list.push(json!({
                    "id": doc.id,
                    "title": doc.title,
                    "updated_at": doc.updated_at,
                }));
            }
        }
        list.sort_by(|a, b| {
            a["updated_at"]
                .as_str()
                .unwrap_or("")
                .cmp(b["updated_at"].as_str().unwrap_or(""))
        });
        Ok(list)
    }

    pub(super) fn write_index(&self, list: &[Value]) -> Result<(), String> {
        let content = serde_json::to_string_pretty(list).map_err(|e| e.to_string())?;
        // 原子写（tmp → rename）：崩溃不会留下半截 index.json（损坏虽可重建，
        // 但会丢失排序与额外字段，且重建期间并发读会看到空索引）。
        let path = self.root.join("index.json");
        let tmp = self
            .root
            .join(format!(".index.json.tmp-{}", uuid::Uuid::new_v4()));
        std::fs::write(&tmp, content).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &path).map_err(|error| {
            let _ = std::fs::remove_file(&tmp);
            error.to_string()
        })
    }

    pub(super) fn load(&self, id: &str) -> Result<NoteDoc, String> {
        let dir = self.doc_dir(id);
        if !dir.is_dir() {
            return Err(format!("笔记不存在：{id}"));
        }
        load_doc(&dir)
    }

    /// 持久化 + 更新清单 + 重索引该文档。
    pub(super) fn persist(&mut self, doc: &NoteDoc) -> Result<(), String> {
        self.ensure_root()?;
        save_doc(doc, &self.doc_dir(&doc.id))?;
        let list = self.list()?;
        let list: Vec<Value> = list
            .into_iter()
            .map(|item| {
                if item["id"].as_str() == Some(doc.id.as_str()) {
                    json!({
                        "id": doc.id,
                        "title": doc.title,
                        "updated_at": doc.updated_at,
                    })
                } else {
                    item
                }
            })
            .collect();
        let list = if list
            .iter()
            .any(|item| item["id"].as_str() == Some(doc.id.as_str()))
        {
            list
        } else {
            let mut list = list;
            list.push(json!({
                "id": doc.id,
                "title": doc.title,
                "updated_at": doc.updated_at,
            }));
            list
        };
        self.write_index(&list)?;
        self.reindex(doc)
    }

    /// 重建文档索引（FTS5：db 位于 <id>/fts.db）。
    pub(super) fn reindex(&mut self, doc: &NoteDoc) -> Result<(), String> {
        let db_path = self.doc_dir(&doc.id).join("fts.db");
        let mut indexer = NoteIndexer::fts(&db_path).map_err(|e| e.to_string())?;
        indexer.index_doc(doc)?;
        self.indexers.insert(doc.id.clone(), indexer);
        Ok(())
    }

    /// 跨所有文档检索（遍历每文档索引合并）。
    pub(super) fn search(&mut self, query: &str) -> Result<Vec<SearchHit>, String> {
        let mut hits: Vec<SearchHit> = Vec::new();
        let ids: Vec<String> = self
            .list()?
            .iter()
            .filter_map(|i| i["id"].as_str().map(str::to_string))
            .collect();
        for id in ids {
            let fts_path = self.doc_dir(&id).join("fts.db");
            let indexer = self.indexers.entry(id.clone()).or_insert_with(|| {
                NoteIndexer::fts(&fts_path).unwrap_or_else(|_| NoteIndexer::in_memory())
            });
            hits.extend(indexer.search(query));
        }
        hits.sort_by(|a, b| a.doc_id.cmp(&b.doc_id).then(a.block_id.cmp(&b.block_id)));
        hits.dedup();
        Ok(hits)
    }

    pub(super) fn delete(&mut self, id: &str) -> Result<(), String> {
        let dir = self.doc_dir(id);
        if !dir.is_dir() {
            return Err(format!("笔记不存在：{id}"));
        }
        // 先释放 FTS 索引连接（SQLite 句柄在 Windows 上会阻止删除目录）。
        self.indexers.remove(id);
        std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
        let list = self.list()?;
        let list: Vec<Value> = list
            .into_iter()
            .filter(|item| item["id"].as_str() != Some(id))
            .collect();
        self.write_index(&list)
    }
}
