//! 全文索引：内存分词 + SQLite FTS5（零新增依赖，从 notes/mod.rs 拆出）。

use super::*;
use std::collections::BTreeMap;
use std::path::Path;

// ----------------------------------------------------------------------------
// 全文索引：内存分词 + SQLite FTS5（零新增依赖）
// ----------------------------------------------------------------------------

/// 标题在索引里的伪块 id：标题不属于任何真实块，命中时用它标出来源。
pub const TITLE_BLOCK_ID: &str = "__title__";

/// 检索命中。
#[derive(Debug, Clone, PartialEq)]
pub struct SearchHit {
    pub doc_id: String,
    pub block_id: BlockId,
    /// 命中词（FTS 片段或内存命中的原词）。
    pub snippet: String,
}

/// 从块提取可检索/可展示文本（递归含子块；供索引、渲染器、测试使用）。
pub fn block_text(doc: &NoteDoc, block: &Block) -> String {
    let mut text = String::new();
    match &block.kind {
        BlockKind::Paragraph { text: t } => text.push_str(t),
        BlockKind::Heading { text: t, .. } => text.push_str(t),
        BlockKind::ListItem { text: t } => text.push_str(t),
        BlockKind::Code { text: t, .. } => text.push_str(t),
        BlockKind::Table { rows } => {
            for row in rows {
                text.push_str(&row.join(" "));
            }
        }
        BlockKind::Image { alt, .. } => text.push_str(alt),
        BlockKind::File { path, .. } => text.push_str(path),
        BlockKind::Quote { text: t } => text.push_str(t),
        BlockKind::HtmlEmbed { html } => text.push_str(html),
        BlockKind::Canvas { data } => {
            for note in &data.notes {
                text.push_str(&note.text);
            }
        }
        BlockKind::AiGenerated {
            prompt, text: t, ..
        } => {
            text.push_str(prompt);
            text.push(' ');
            text.push_str(t);
        }
        BlockKind::List { .. } => {}
    }
    // 递归子块
    for child_id in &block.children {
        if let Some(child) = doc.blocks.get(child_id) {
            text.push(' ');
            text.push_str(&block_text(doc, child));
        }
    }
    text
}

/// 简单分词：按非字母数字切分 + 小写。
fn tokenize(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            current.push(ch.to_lowercase().next().unwrap_or(ch));
        } else if !current.is_empty() {
            out.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// 索引 trait：内存实现与 FTS5 实现共用。
pub trait NoteIndex: Send {
    /// 全量重建文档索引（幂等）。
    fn index_doc(&mut self, doc: &NoteDoc) -> Result<(), String>;
    /// 检索，返回命中的块（按文档内顺序）。
    fn search(&self, query: &str) -> Vec<SearchHit>;
}

/// 内存分词索引（简单、无文件依赖）。
#[derive(Default)]
pub struct InMemoryNoteIndex {
    /// 词 → (doc_id, block_id, snippet)
    map: BTreeMap<String, Vec<(String, String, String)>>,
}

impl InMemoryNoteIndex {
    pub fn new() -> Self {
        Self::default()
    }
}

impl NoteIndex for InMemoryNoteIndex {
    fn index_doc(&mut self, doc: &NoteDoc) -> Result<(), String> {
        self.map.clear();
        // 标题也进索引：笔记列表按标题展示，用户常按标题找回笔记，而标题只存在
        // NoteDoc.title，不在任何块里，只索引块正文会搜不到。
        if !doc.title.trim().is_empty() {
            let snippet: String = doc.title.chars().take(60).collect();
            for word in tokenize(&doc.title) {
                if word.len() < 2 {
                    continue;
                }
                self.map.entry(word).or_default().push((
                    doc.id.clone(),
                    TITLE_BLOCK_ID.to_string(),
                    snippet.clone(),
                ));
            }
        }
        for block in walk(doc, &doc.root) {
            let text = block_text(doc, block);
            if text.trim().is_empty() {
                continue;
            }
            let snippet: String = text.chars().take(60).collect();
            for word in tokenize(&text) {
                if word.len() < 2 {
                    continue;
                }
                self.map.entry(word).or_default().push((
                    doc.id.clone(),
                    block.id.clone(),
                    snippet.clone(),
                ));
            }
        }
        Ok(())
    }

    fn search(&self, query: &str) -> Vec<SearchHit> {
        let words: Vec<String> = tokenize(query)
            .into_iter()
            .filter(|w| w.len() >= 2)
            .collect();
        if words.is_empty() {
            return Vec::new();
        }
        let mut hits = Vec::new();
        for (word, entries) in &self.map {
            if words
                .iter()
                .any(|w| word.starts_with(w) || w.starts_with(word))
            {
                for (doc_id, block_id, snippet) in entries {
                    hits.push(SearchHit {
                        doc_id: doc_id.clone(),
                        block_id: block_id.clone(),
                        snippet: snippet.clone(),
                    });
                }
            }
        }
        hits.sort_by(|a, b| a.doc_id.cmp(&b.doc_id).then(a.block_id.cmp(&b.block_id)));
        hits.dedup();
        hits
    }
}

/// SQLite FTS5 索引（`<db_path>` 单文件；重建时清空重插）。
/// tokenizer 用 trigram：对中文/无空格语言的子串检索友好；<3 字符查询回退 LIKE。
pub struct FtsNoteIndex {
    conn: std::sync::Mutex<rusqlite::Connection>,
}

impl FtsNoteIndex {
    pub fn open(db_path: &Path) -> Result<Self, String> {
        let conn = rusqlite::Connection::open(db_path).map_err(|e| e.to_string())?;
        conn.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS note_fts USING fts5(doc_id, block_id UNINDEXED, text, tokenize='trigram');",
        )
        .map_err(|e| e.to_string())?;
        Ok(Self {
            conn: std::sync::Mutex::new(conn),
        })
    }
}

impl NoteIndex for FtsNoteIndex {
    fn index_doc(&mut self, doc: &NoteDoc) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch("DELETE FROM note_fts;")
            .map_err(|e| e.to_string())?;
        // 标题行：FTS 只按块建索引时，标题不在任何块里，按标题搜会漏（见内存索引注释）。
        if !doc.title.trim().is_empty() {
            conn.execute(
                "INSERT INTO note_fts(doc_id, block_id, text) VALUES (?1, ?2, ?3)",
                rusqlite::params![doc.id, TITLE_BLOCK_ID, doc.title],
            )
            .map_err(|e| e.to_string())?;
        }
        for block in walk(doc, &doc.root) {
            let text = block_text(doc, block);
            if text.trim().is_empty() {
                continue;
            }
            conn.execute(
                "INSERT INTO note_fts(doc_id, block_id, text) VALUES (?1, ?2, ?3)",
                rusqlite::params![doc.id, block.id, text],
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    fn search(&self, query: &str) -> Vec<SearchHit> {
        let query = query.trim();
        if query.is_empty() {
            return Vec::new();
        }
        let conn = self.conn.lock().unwrap();
        // 短查询（trigram 要求 ≥3 字符）：LIKE 回退
        let chars: usize = query.chars().count();
        if chars < 3 {
            let like = format!("%{}%", query.replace('%', "\\%").replace('_', "\\_"));
            let mut stmt = match conn.prepare(
                "SELECT doc_id, block_id, substr(text, 1, 60) FROM note_fts WHERE text LIKE ?1 ESCAPE '\\'",
            ) {
                Ok(stmt) => stmt,
                Err(_) => return Vec::new(),
            };
            let rows = stmt.query_map(rusqlite::params![like], |row| {
                Ok(SearchHit {
                    doc_id: row.get(0)?,
                    block_id: row.get(1)?,
                    snippet: row.get(2)?,
                })
            });
            return match rows {
                Ok(rows) => rows.flatten().collect(),
                Err(_) => Vec::new(),
            };
        }
        // 查询词转 FTS 短语（trigram 子串匹配）
        let phrase = format!("\"{}\"", query.replace('"', "\"\""));
        let mut stmt = match conn.prepare(
            "SELECT doc_id, block_id, snippet(note_fts, 2, '[', ']', '...', 12) FROM note_fts WHERE note_fts MATCH ?1",
        ) {
            Ok(stmt) => stmt,
            Err(_) => return Vec::new(),
        };
        let rows = stmt.query_map(rusqlite::params![phrase], |row| {
            Ok(SearchHit {
                doc_id: row.get(0)?,
                block_id: row.get(1)?,
                snippet: row.get(2)?,
            })
        });
        match rows {
            Ok(rows) => rows.flatten().collect(),
            Err(_) => Vec::new(),
        }
    }
}

/// 便捷入口：索引器（内存为主，可选 FTS）。
pub struct NoteIndexer {
    inner: Box<dyn NoteIndex>,
}

impl NoteIndexer {
    pub fn in_memory() -> Self {
        Self {
            inner: Box::new(InMemoryNoteIndex::new()),
        }
    }

    pub fn fts(db_path: &Path) -> Result<Self, String> {
        Ok(Self {
            inner: Box::new(FtsNoteIndex::open(db_path)?),
        })
    }

    pub fn index_doc(&mut self, doc: &NoteDoc) -> Result<(), String> {
        self.inner.index_doc(doc)
    }
}

impl NoteIndex for NoteIndexer {
    fn index_doc(&mut self, doc: &NoteDoc) -> Result<(), String> {
        self.inner.index_doc(doc)
    }

    fn search(&self, query: &str) -> Vec<SearchHit> {
        self.inner.search(query)
    }
}

/// 便捷函数：内存索引检索。
pub fn search_notes(index: &NoteIndexer, query: &str) -> Vec<SearchHit> {
    index.search(query)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc_with_body(id: &str, title: &str, body: &str) -> NoteDoc {
        let mut doc = new_doc(id, title);
        let root = doc.root.clone();
        add_block(
            &mut doc,
            &root,
            BlockKind::Paragraph {
                text: body.into(),
            },
            BTreeMap::new(),
        )
        .unwrap();
        doc
    }

    fn hit_blocks(index: &mut NoteIndexer, doc: &NoteDoc, query: &str) -> Vec<String> {
        index.index_doc(doc).unwrap();
        let mut blocks: Vec<String> = search_notes(index, query)
            .into_iter()
            .filter(|hit| hit.doc_id == doc.id)
            .map(|hit| hit.block_id)
            .collect();
        blocks.sort();
        blocks.dedup();
        blocks
    }

    #[test]
    fn in_memory_index_matches_title_even_when_body_differs() {
        let mut index = NoteIndexer::in_memory();
        // 内存索引按词前缀匹配，标题用前缀词验证（FTS 走子串，见下一条用例）。
        let doc = doc_with_body("d-title", "自检清单周报", "正文只有正文两个字");
        // 标题独有的词应命中，并且来源标成标题伪块。
        let blocks = hit_blocks(&mut index, &doc, "自检");
        assert_eq!(blocks, vec![TITLE_BLOCK_ID.to_string()]);
        // 正文词仍命中正文块，不被标题行干扰。
        let body_blocks = hit_blocks(&mut index, &doc, "正文");
        assert!(!body_blocks.contains(&TITLE_BLOCK_ID.to_string()));
        assert!(!body_blocks.is_empty());
    }

    #[test]
    fn fts_index_matches_title_with_short_and_long_queries() {
        let db = std::env::temp_dir().join(format!("owo-notes-fts-{}.db", uuid::Uuid::new_v4()));
        let mut index = NoteIndexer::fts(&db).unwrap();
        let doc = doc_with_body("d-fts", "季度复盘摘要", "正文提到 alpha 和 beta");
        // 两字查询走 LIKE 回退，三字以上走 trigram MATCH，两条路径都要能按标题找到。
        assert!(hit_blocks(&mut index, &doc, "复盘").contains(&TITLE_BLOCK_ID.to_string()));
        assert!(hit_blocks(&mut index, &doc, "季度复").contains(&TITLE_BLOCK_ID.to_string()));
        // 正文命中不应被标题行取代。
        let body_blocks = hit_blocks(&mut index, &doc, "alpha");
        assert!(!body_blocks.is_empty());
        assert!(!body_blocks.contains(&TITLE_BLOCK_ID.to_string()));
        drop(index);
        let _ = std::fs::remove_file(&db);
    }
}
