use crate::Database;
use pw_application::PortError;
use pw_application::memory::{MemoryRecord, MemoryStore, StoredSummary};
use rusqlite::{OptionalExtension, params};

pub struct SqliteMemoryStore {
    database: Database,
}
impl SqliteMemoryStore {
    #[must_use]
    pub const fn new(database: Database) -> Self {
        Self { database }
    }
}

impl MemoryStore for SqliteMemoryStore {
    fn load_summary(&self, conversation_id: &str) -> Result<Option<StoredSummary>, PortError> {
        self.database.connection().query_row("SELECT content, through_message_id FROM conversation_summaries WHERE conversation_id=?1", [conversation_id], |row| Ok(StoredSummary { content: row.get(0)?, through_message_id: row.get(1)? })).optional().map_err(|e| PortError(e.to_string()))
    }
    fn upsert_summary(
        &mut self,
        conversation_id: &str,
        content: &str,
        through_message_id: i64,
        updated_at: i64,
    ) -> Result<(), PortError> {
        self.database.connection().execute("INSERT INTO conversation_summaries(conversation_id,content,through_message_id,updated_at) VALUES(?1,?2,?3,?4) ON CONFLICT(conversation_id) DO UPDATE SET content=excluded.content,through_message_id=excluded.through_message_id,updated_at=excluded.updated_at", params![conversation_id,content,through_message_id,updated_at]).map(|_| ()).map_err(|e| PortError(e.to_string()))
    }
    fn upsert_memory(
        &mut self,
        source: Option<&str>,
        content: &str,
        updated_at: i64,
    ) -> Result<i64, PortError> {
        if let Some(id) = self.database.connection().query_row("SELECT id FROM memories WHERE content=?1 AND source_conversation_id IS ?2 ORDER BY id LIMIT 1", params![content, source], |row| row.get(0)).optional().map_err(|e| PortError(e.to_string()))? {
            self.database.connection().execute("UPDATE memories SET updated_at=?1 WHERE id=?2", params![updated_at,id]).map_err(|e| PortError(e.to_string()))?;
            return Ok(id);
        }
        self.database.connection().execute("INSERT INTO memories(content,source_conversation_id,created_at,updated_at) VALUES(?1,?2,?3,?3)", params![content,source,updated_at]).map_err(|e| PortError(e.to_string()))?;
        Ok(self.database.connection().last_insert_rowid())
    }
    fn update_memory(&mut self, id: i64, content: &str, updated_at: i64) -> Result<(), PortError> {
        self.database
            .connection()
            .execute(
                "UPDATE memories SET content=?1,updated_at=?2 WHERE id=?3",
                params![content, updated_at, id],
            )
            .map(|_| ())
            .map_err(|e| PortError(e.to_string()))
    }
    fn delete_memory(&mut self, id: i64) -> Result<(), PortError> {
        self.database
            .connection()
            .execute("DELETE FROM memories WHERE id=?1", [id])
            .map(|_| ())
            .map_err(|e| PortError(e.to_string()))
    }
    fn delete_summary(&mut self, conversation_id: &str) -> Result<(), PortError> {
        self.database
            .connection()
            .execute(
                "DELETE FROM conversation_summaries WHERE conversation_id=?1",
                [conversation_id],
            )
            .map(|_| ())
            .map_err(|e| PortError(e.to_string()))
    }
    fn search(&self, query: &str, limit: usize) -> Result<Vec<MemoryRecord>, PortError> {
        if query.trim().is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        if query.trim().chars().count() < 3 {
            let escaped = query
                .trim()
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_");
            let pattern = format!("%{escaped}%");
            let mut statement = self.database.connection().prepare("SELECT id,content FROM memories WHERE content LIKE ?1 ESCAPE '\\' ORDER BY updated_at DESC,id DESC LIMIT ?2").map_err(|e| PortError(e.to_string()))?;
            return statement
                .query_map(
                    params![pattern, i64::try_from(limit).unwrap_or(i64::MAX)],
                    |row| {
                        Ok(MemoryRecord {
                            id: row.get(0)?,
                            content: row.get(1)?,
                        })
                    },
                )
                .map_err(|e| PortError(e.to_string()))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| PortError(e.to_string()));
        }
        let phrase = format!("\"{}\"", query.trim().replace('"', "\"\""));
        let mut statement=self.database.connection().prepare("SELECT m.id,m.content FROM memories_fts f JOIN memories m ON m.id=f.rowid WHERE memories_fts MATCH ?1 ORDER BY bm25(memories_fts),m.updated_at DESC LIMIT ?2").map_err(|e| PortError(e.to_string()))?;
        statement
            .query_map(
                params![phrase, i64::try_from(limit).unwrap_or(i64::MAX)],
                |row| {
                    Ok(MemoryRecord {
                        id: row.get(0)?,
                        content: row.get(1)?,
                    })
                },
            )
            .map_err(|e| PortError(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| PortError(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::SqliteMemoryStore;
    use crate::Database;
    use pw_application::memory::MemoryStore;

    #[test]
    fn summary_and_memory_survive_reopen_and_search_by_relevance() {
        let path = std::env::temp_dir().join(format!("pw-memory-{}.sqlite3", std::process::id()));
        let _ = std::fs::remove_file(&path);
        {
            let mut store = SqliteMemoryStore::new(Database::open(&path).unwrap());
            store.database.connection().execute_batch("INSERT INTO conversations(id,created_at,updated_at) VALUES('chat',1,1); INSERT INTO messages(conversation_id,turn_id,role,content,created_at) VALUES('chat',1,'user','x',1);").unwrap();
            store.upsert_summary("chat", "旅行の要約", 1, 10).unwrap();
            store.upsert_memory(None, "猫が好き", 10).unwrap();
            store.upsert_memory(None, "犬の散歩", 11).unwrap();
        }
        let store = SqliteMemoryStore::new(Database::open(&path).unwrap());
        assert_eq!(
            store.load_summary("chat").unwrap().unwrap().content,
            "旅行の要約"
        );
        assert_eq!(store.search("猫が好き", 1).unwrap()[0].content, "猫が好き");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn update_refreshes_the_fts_index() {
        let mut store = SqliteMemoryStore::new(Database::open_in_memory().unwrap());
        let id = store.upsert_memory(None, "紅茶が好き", 1).unwrap();
        store.update_memory(id, "コーヒーが好き", 2).unwrap();
        assert!(store.search("紅茶", 10).unwrap().is_empty());
        assert_eq!(store.search("コーヒー", 10).unwrap().len(), 1);
        store.delete_memory(id).unwrap();
        assert!(store.search("コーヒー", 10).unwrap().is_empty());
    }

    #[test]
    fn japanese_short_queries_use_escaped_like_fallback() {
        let mut store = SqliteMemoryStore::new(Database::open_in_memory().unwrap());
        store.upsert_memory(None, "猫が好き", 1).unwrap();
        store.upsert_memory(None, "100%確実_です", 2).unwrap();
        assert_eq!(store.search("猫", 10).unwrap()[0].content, "猫が好き");
        assert_eq!(store.search("%", 10).unwrap()[0].content, "100%確実_です");
        assert_eq!(store.search("_", 10).unwrap()[0].content, "100%確実_です");
    }

    #[test]
    fn fact_upsert_does_not_duplicate_identical_source_content() {
        let mut store = SqliteMemoryStore::new(Database::open_in_memory().unwrap());
        let first = store.upsert_memory(None, "猫が好き", 1).unwrap();
        let second = store.upsert_memory(None, "猫が好き", 2).unwrap();
        assert_eq!(first, second);
        assert_eq!(store.search("猫", 10).unwrap().len(), 1);
    }
}
