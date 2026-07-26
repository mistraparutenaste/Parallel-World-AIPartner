//! Privacy-safe Memory Center IPC. All `SQLite` work stays in blocking workers;
//! the renderer only receives bounded summaries.

use pw_application::memory::{MemoryDomain, is_safe_persistent_content, redact_persistent_content};
use pw_contracts::{
    CommitmentSummaryDto, DialogueSummaryDto, MemoryCenterDto, MemoryDomainControlDto,
    MemorySummaryDto, PendingMemoryCandidateDto, SCHEMA_VERSION,
};
use pw_platform::paths::AppDataLayout;
use pw_storage::{Database, ImportedMemoryRecord, SqliteMemoryStore};
use rusqlite::{OptionalExtension, params};
use std::path::Path;
use tauri::{AppHandle, Manager, Runtime, State};

use crate::chat::ChatService;
use crate::commands::data::validated_export_path;

const CONVERSATION: &str = "default";
const PREVIEW_LIMIT: usize = 160;
const MAX_MEMORY_CHARS: usize = 2_000;
const MEMORY_CSV_HEADER: [&str; 7] = [
    "id",
    "domain",
    "content",
    "state",
    "pinned",
    "created_at",
    "updated_at",
];

fn database_path(layout: &AppDataLayout) -> std::path::PathBuf {
    layout.main_database()
}

pub(crate) fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |value| {
            i64::try_from(value.as_secs()).unwrap_or(i64::MAX)
        })
}

/// Redacts persistent secrets before normalising and bounding a renderer
/// preview. The second safety check is deliberate: a future redactor change
/// must fail closed rather than return an unredacted value to the webview.
fn bounded_preview(value: &str) -> String {
    let redacted = if is_safe_persistent_content(value) {
        value.to_owned()
    } else {
        redact_persistent_content(value)
    };
    let redacted = if is_safe_persistent_content(&redacted) {
        redacted
    } else {
        "[REDACTED]".to_owned()
    };
    let trimmed = redacted.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut preview = trimmed.chars().take(PREVIEW_LIMIT).collect::<String>();
    if trimmed.chars().count() > PREVIEW_LIMIT {
        preview.push('…');
    }
    preview
}

fn bounded_optional_preview(value: Option<String>) -> Option<String> {
    value.map(|value| bounded_preview(&value))
}

#[allow(clippy::too_many_lines)]
fn memory_center_at(path: &std::path::Path, now: i64) -> Result<MemoryCenterDto, String> {
    let database = Database::open(path).map_err(|error| error.to_string())?;
    let connection = database.connection();
    let domains = connection
        .prepare(
            "SELECT domain,consent,retention_seconds,revision FROM memory_domain_controls ORDER BY domain",
        )
        .map_err(|error| error.to_string())?
        .query_map([], |row| {
            Ok(MemoryDomainControlDto {
                domain: row.get(0)?,
                consent: row.get(1)?,
                retention_seconds: row.get(2)?,
                revision: row.get(3)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let memories = connection
        .prepare(
            "SELECT id,content,state,pinned,created_at,updated_at,revision FROM memories ORDER BY updated_at DESC,id DESC LIMIT 50",
        )
        .map_err(|error| error.to_string())?
        .query_map([], |row| {
            let content: String = row.get(1)?;
            Ok(MemorySummaryDto {
                id: row.get(0)?,
                preview: bounded_preview(&content),
                state: row.get(2)?,
                pinned: row.get(3)?,
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
                revision: row.get(6)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let pending = connection
        .prepare("SELECT id,memory_domain,content,created_at FROM memory_candidates WHERE candidate_state='pending' AND policy_state!='rejected' ORDER BY created_at DESC,id DESC LIMIT 50")
        .map_err(|error| error.to_string())?
        .query_map([], |row| {
            let content: String = row.get(2)?;
            Ok(PendingMemoryCandidateDto {
                id: row.get(0)?,
                domain: row.get(1)?,
                preview: bounded_preview(&content),
                created_at: row.get(3)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let commitments = connection
        .prepare("SELECT id,content,status,due_at,revision FROM commitments WHERE status='open' ORDER BY COALESCE(due_at,9223372036854775807),id LIMIT 50")
        .map_err(|error| error.to_string())?
        .query_map([], |row| {
            Ok(CommitmentSummaryDto {
                id: row.get(0)?,
                content: row.get(1)?,
                status: row.get(2)?,
                due_at: row.get(3)?,
                revision: row.get(4)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let dialogue = connection
        .query_row(
            "SELECT mood,relationship_summary,relationship_score,expires_at,revision FROM dialogue_states WHERE conversation_id=?1 AND expires_at>?2",
            params![CONVERSATION, now],
            |row| {
                Ok(DialogueSummaryDto {
                    mood: bounded_optional_preview(row.get(0)?),
                    relationship_summary: bounded_optional_preview(row.get(1)?),
                    relationship_score: row.get(2)?,
                    expires_at: row.get(3)?,
                    revision: row.get(4)?,
                })
            },
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let (temporary, temporary_revision) = connection
        .query_row(
            "SELECT temporary,revision FROM temporary_conversations WHERE conversation_id=?1",
            [CONVERSATION],
            |row| Ok((row.get::<_, bool>(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .unwrap_or((false, 0));
    Ok(MemoryCenterDto {
        schema_version: SCHEMA_VERSION,
        domains,
        memories,
        pending,
        commitments,
        dialogue,
        temporary,
        temporary_revision,
    })
}

/// # Errors
///
/// Returns an error message when the database cannot be read or the worker
/// cannot be spawned.
#[tauri::command]
pub async fn get_memory_center(
    layout: State<'_, AppDataLayout>,
) -> Result<MemoryCenterDto, String> {
    let path = database_path(&layout);
    tauri::async_runtime::spawn_blocking(move || memory_center_at(&path, now()))
        .await
        .map_err(|error| error.to_string())?
}

fn memory_content_at(path: &Path, memory_id: i64) -> Result<String, String> {
    if memory_id <= 0 {
        return Err("invalid memory id".into());
    }
    let database = Database::open(path).map_err(|error| error.to_string())?;
    SqliteMemoryStore::new(database)
        .memory_content(memory_id)
        .map_err(|error| error.to_string())?
        .map(|(content, _)| content)
        .ok_or_else(|| "MEMORY_NOT_FOUND".to_owned())
}

/// Returns the full content of one explicitly selected memory. List responses
/// remain preview-only.
///
/// # Errors
/// Returns an error for an invalid or missing id, or a database/worker failure.
#[tauri::command]
pub async fn get_memory_content(
    layout: State<'_, AppDataLayout>,
    memory_id: i64,
) -> Result<String, String> {
    let path = database_path(&layout);
    tauri::async_runtime::spawn_blocking(move || memory_content_at(&path, memory_id))
        .await
        .map_err(|error| error.to_string())?
}

fn update_memory_at(
    path: &Path,
    memory_id: i64,
    content: &str,
    expected_revision: i64,
) -> Result<(), String> {
    if memory_id <= 0
        || expected_revision <= 0
        || content.trim().is_empty()
        || content.chars().count() > MAX_MEMORY_CHARS
        || !is_safe_persistent_content(content)
    {
        return Err("INVALID_MEMORY_CONTENT".into());
    }
    let database = Database::open(path).map_err(|error| error.to_string())?;
    SqliteMemoryStore::new(database)
        .update_memory_fenced(memory_id, content, expected_revision)
        .map(|_| ())
        .map_err(|error| {
            if error.to_string().contains("MEMORY_CONFLICT") {
                "MEMORY_CONFLICT".to_owned()
            } else {
                error.to_string()
            }
        })
}

/// Updates one memory through compare-and-set and returns a fresh preview-only
/// Memory Center snapshot.
///
/// # Errors
/// Returns `MEMORY_CONFLICT` for a stale revision, or an error for unsafe,
/// empty, over-limit content and database/worker failures.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub async fn update_memory<R: Runtime>(
    app: AppHandle<R>,
    memory_id: i64,
    content: String,
    expected_revision: i64,
) -> Result<MemoryCenterDto, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let layout = app.state::<AppDataLayout>();
        let path = database_path(&layout);
        let chat = app.state::<ChatService>();
        chat.with_exclusive_reset(|| {
            update_memory_at(&path, memory_id, &content, expected_revision)?;
            memory_center_at(&path, now())
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

fn export_memories_csv_at(
    database_path: &Path,
    destination: &Path,
    allow_overwrite: bool,
) -> Result<(), String> {
    let destination = validated_export_path(database_path, destination, allow_overwrite)?;
    let database = Database::open(database_path).map_err(|error| error.to_string())?;
    let mut statement = database
        .connection()
        .prepare(
            "SELECT m.id,COALESCE(
               (SELECT d.domain FROM imported_memory_domains d WHERE d.memory_id=m.id),
               (SELECT c.memory_domain FROM memory_provenance p JOIN memory_candidates c ON c.id=p.candidate_id WHERE p.memory_id=m.id ORDER BY p.created_at DESC,p.candidate_id DESC LIMIT 1),
               'semantic_user'
             ),m.content,m.state,m.pinned,m.created_at,m.updated_at FROM memories m ORDER BY m.id",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok([
                row.get::<_, i64>(0)?.to_string(),
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, bool>(4)?.to_string(),
                row.get::<_, i64>(5)?.to_string(),
                row.get::<_, i64>(6)?.to_string(),
            ])
        })
        .map_err(|error| error.to_string())?;
    let mut writer = csv::WriterBuilder::new()
        .terminator(csv::Terminator::CRLF)
        .from_path(destination)
        .map_err(|error| error.to_string())?;
    writer
        .write_record(MEMORY_CSV_HEADER)
        .map_err(|error| error.to_string())?;
    for row in rows {
        writer
            .write_record(row.map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
    }
    writer.flush().map_err(|error| error.to_string())
}

/// Exports durable memories as RFC 4180 CSV.
///
/// # Errors
/// Returns an error for unsafe destinations or database/file failures.
#[tauri::command]
pub async fn export_memories_csv(
    layout: State<'_, AppDataLayout>,
    destination: String,
    allow_overwrite: bool,
) -> Result<(), String> {
    let database_path = database_path(&layout);
    tauri::async_runtime::spawn_blocking(move || {
        export_memories_csv_at(&database_path, Path::new(&destination), allow_overwrite)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[derive(Debug)]
struct ImportedMemory {
    id: Option<i64>,
    domain: String,
    content: String,
    state: String,
    pinned: bool,
    created_at: i64,
    updated_at: i64,
}

fn parse_memory_csv(source: &Path) -> Result<Vec<ImportedMemory>, String> {
    let mut reader = csv::ReaderBuilder::new()
        .from_path(source)
        .map_err(|error| error.to_string())?;
    if reader
        .headers()
        .map_err(|error| error.to_string())?
        .iter()
        .ne(MEMORY_CSV_HEADER)
    {
        return Err("INVALID_MEMORY_CSV_HEADER".into());
    }
    let mut imported = Vec::new();
    for record in reader.records() {
        let record = record.map_err(|_| "INVALID_MEMORY_CSV_ROW".to_owned())?;
        let field = |index: usize| {
            record
                .get(index)
                .ok_or_else(|| "INVALID_MEMORY_CSV_ROW".to_owned())
        };
        let id = match field(0)?.trim() {
            "" => None,
            value => Some(
                value
                    .parse::<i64>()
                    .map_err(|_| "INVALID_MEMORY_CSV_ROW".to_owned())?,
            ),
        };
        let domain = field(1)?.to_owned();
        MemoryDomain::parse(&domain).map_err(|_| "INVALID_MEMORY_CSV_ROW".to_owned())?;
        let content = field(2)?.to_owned();
        let state = field(3)?.to_owned();
        let pinned = field(4)?
            .parse::<bool>()
            .map_err(|_| "INVALID_MEMORY_CSV_ROW".to_owned())?;
        let created_at = field(5)?
            .parse::<i64>()
            .map_err(|_| "INVALID_MEMORY_CSV_ROW".to_owned())?;
        let updated_at = field(6)?
            .parse::<i64>()
            .map_err(|_| "INVALID_MEMORY_CSV_ROW".to_owned())?;
        if id.is_some_and(|value| value <= 0)
            || content.trim().is_empty()
            || content.chars().count() > MAX_MEMORY_CHARS
            || !is_safe_persistent_content(&content)
            || !matches!(state.as_str(), "active" | "dormant" | "superseded")
            || created_at < 0
            || updated_at < 0
        {
            return Err("INVALID_MEMORY_CSV_ROW".into());
        }
        imported.push(ImportedMemory {
            id,
            domain,
            content,
            state,
            pinned,
            created_at,
            updated_at,
        });
    }
    Ok(imported)
}

fn import_memories_csv_at(database_path: &Path, source: &Path) -> Result<(), String> {
    let source = source.canonicalize().map_err(|error| error.to_string())?;
    if !source.is_file() {
        return Err("インポート元は通常ファイルである必要があります".into());
    }
    if same_file::is_same_file(database_path, &source).map_err(|error| error.to_string())? {
        return Err("使用中のデータベースはCSVとして読み込めません".into());
    }
    let imported = parse_memory_csv(&source)?;
    let database = Database::open(database_path).map_err(|error| error.to_string())?;
    let mut store = SqliteMemoryStore::new(database);
    for memory in imported {
        store
            .insert_memory_direct(ImportedMemoryRecord {
                id: memory.id,
                domain: &memory.domain,
                content: &memory.content,
                state: &memory.state,
                pinned: memory.pinned,
                created_at: memory.created_at,
                updated_at: memory.updated_at,
            })
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// Imports RFC 4180 memory CSV. Existing ids are updated; blank ids insert.
///
/// # Errors
/// Returns an error for malformed rows, unsafe content, or database/file
/// failures.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub async fn import_memories_csv<R: Runtime>(
    app: AppHandle<R>,
    source: String,
) -> Result<MemoryCenterDto, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let layout = app.state::<AppDataLayout>();
        let path = database_path(&layout);
        let chat = app.state::<ChatService>();
        chat.with_exclusive_reset(|| {
            import_memories_csv_at(&path, Path::new(&source))?;
            memory_center_at(&path, now())
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

fn validate_commitment(content: &str, status: &str, due_at: Option<i64>) -> Result<(), String> {
    if content.trim().is_empty()
        || content.chars().count() > MAX_MEMORY_CHARS
        || !is_safe_persistent_content(content)
        || !matches!(status, "open" | "completed" | "cancelled")
        || due_at.is_some_and(|value| value < 0)
    {
        return Err("INVALID_COMMITMENT".into());
    }
    Ok(())
}

fn create_commitment_at(
    path: &Path,
    content: &str,
    due_at: Option<i64>,
    now: i64,
) -> Result<(), String> {
    validate_commitment(content, "open", due_at)?;
    let database = Database::open(path).map_err(|error| error.to_string())?;
    database
        .connection()
        .execute(
            "INSERT INTO commitments(conversation_id,content,status,due_at,revision,created_at,updated_at) VALUES(?1,?2,'open',?3,1,?4,?4)",
            params![CONVERSATION, content.trim(), due_at, now],
        )
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn update_commitment_at(
    path: &Path,
    id: i64,
    content: &str,
    status: &str,
    due_at: Option<i64>,
    expected_revision: i64,
    now: i64,
) -> Result<(), String> {
    validate_commitment(content, status, due_at)?;
    if id <= 0 || expected_revision <= 0 {
        return Err("INVALID_COMMITMENT".into());
    }
    let database = Database::open(path).map_err(|error| error.to_string())?;
    let changed = database
        .connection()
        .execute(
            "UPDATE commitments SET content=?1,status=?2,due_at=?3,revision=revision+1,updated_at=?4 WHERE id=?5 AND conversation_id=?6 AND revision=?7",
            params![content.trim(), status, due_at, now, id, CONVERSATION, expected_revision],
        )
        .map_err(|error| error.to_string())?;
    if changed == 1 {
        Ok(())
    } else {
        Err("COMMITMENT_CONFLICT".into())
    }
}

fn delete_commitment_at(path: &Path, id: i64) -> Result<(), String> {
    if id <= 0 {
        return Err("INVALID_COMMITMENT".into());
    }
    let database = Database::open(path).map_err(|error| error.to_string())?;
    let changed = database
        .connection()
        .execute(
            "DELETE FROM commitments WHERE id=?1 AND conversation_id=?2",
            params![id, CONVERSATION],
        )
        .map_err(|error| error.to_string())?;
    if changed == 1 {
        Ok(())
    } else {
        Err("COMMITMENT_NOT_FOUND".into())
    }
}

/// Creates a manually entered task for the durable default conversation.
///
/// # Errors
/// Returns an error for unsafe/over-limit content or a database failure.
#[tauri::command]
pub async fn create_commitment(
    layout: State<'_, AppDataLayout>,
    content: String,
    due_at: Option<i64>,
) -> Result<MemoryCenterDto, String> {
    let path = database_path(&layout);
    tauri::async_runtime::spawn_blocking(move || {
        create_commitment_at(&path, &content, due_at, now())?;
        memory_center_at(&path, now())
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Updates task content, status, and due date through compare-and-set.
///
/// # Errors
/// Returns `COMMITMENT_CONFLICT` for a stale revision, or a validation,
/// database, or worker error.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn update_commitment(
    layout: State<'_, AppDataLayout>,
    id: i64,
    content: String,
    status: String,
    due_at: Option<i64>,
    expected_revision: i64,
) -> Result<MemoryCenterDto, String> {
    let path = database_path(&layout);
    tauri::async_runtime::spawn_blocking(move || {
        update_commitment_at(
            &path,
            id,
            &content,
            &status,
            due_at,
            expected_revision,
            now(),
        )?;
        memory_center_at(&path, now())
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Deletes a manually managed task.
///
/// # Errors
/// Returns an error for an invalid/missing id or a database/worker failure.
#[tauri::command]
pub async fn delete_commitment(
    layout: State<'_, AppDataLayout>,
    id: i64,
) -> Result<MemoryCenterDto, String> {
    let path = database_path(&layout);
    tauri::async_runtime::spawn_blocking(move || {
        delete_commitment_at(&path, id)?;
        memory_center_at(&path, now())
    })
    .await
    .map_err(|error| error.to_string())?
}

fn set_domain_control_at(
    path: &std::path::Path,
    domain: &str,
    consent: &str,
    expected_revision: i64,
    now: i64,
) -> Result<MemoryDomainControlDto, String> {
    MemoryDomain::parse(domain).map_err(|_| "invalid memory domain".to_owned())?;
    if !matches!(consent, "allowed" | "pending_approval" | "never_store") || expected_revision < 0 {
        return Err("invalid memory control".into());
    }
    let database = Database::open(path).map_err(|error| error.to_string())?;
    let changed = database
        .connection()
        .execute(
            "UPDATE memory_domain_controls SET consent=?1,revision=revision+1,updated_at=?2 WHERE domain=?3 AND revision=?4",
            params![consent, now, domain, expected_revision],
        )
        .map_err(|error| error.to_string())?;
    if changed == 0 {
        return Err("MEMORY_CONTROL_CONFLICT".into());
    }
    database
        .connection()
        .query_row(
            "SELECT domain,consent,retention_seconds,revision FROM memory_domain_controls WHERE domain=?1",
            [domain],
            |row| {
                Ok(MemoryDomainControlDto {
                    domain: row.get(0)?,
                    consent: row.get(1)?,
                    retention_seconds: row.get(2)?,
                    revision: row.get(3)?,
                })
            },
        )
        .map_err(|error| error.to_string())
}

/// # Errors
///
/// Returns an error message for an invalid domain/consent, a stale
/// `expected_revision`, or a database/worker failure.
#[tauri::command]
pub async fn set_memory_domain_control(
    layout: State<'_, AppDataLayout>,
    domain: String,
    consent: String,
    expected_revision: i64,
) -> Result<MemoryDomainControlDto, String> {
    let path = database_path(&layout);
    tauri::async_runtime::spawn_blocking(move || {
        set_domain_control_at(&path, &domain, &consent, expected_revision, now())
    })
    .await
    .map_err(|error| error.to_string())?
}

fn set_temporary_at(
    path: &std::path::Path,
    temporary: bool,
    expected_revision: i64,
    now: i64,
) -> Result<i64, String> {
    let mut database = Database::open(path).map_err(|error| error.to_string())?;
    let transaction = database
        .connection_mut()
        .transaction()
        .map_err(|error| error.to_string())?;
    let revision = transaction
        .query_row(
            "SELECT revision FROM temporary_conversations WHERE conversation_id=?1",
            [CONVERSATION],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let next = match revision {
        None if expected_revision == 0 => {
            transaction
                .execute(
                    "INSERT INTO temporary_conversations(conversation_id,temporary,revision,updated_at) VALUES(?1,?2,1,?3)",
                    params![CONVERSATION, temporary, now],
                )
                .map_err(|error| error.to_string())?;
            1
        }
        Some(value) if value == expected_revision => {
            transaction
                .execute(
                    "UPDATE temporary_conversations SET temporary=?1,revision=revision+1,updated_at=?2 WHERE conversation_id=?3 AND revision=?4",
                    params![temporary, now, CONVERSATION, value],
                )
                .map_err(|error| error.to_string())?;
            value + 1
        }
        _ => return Err("TEMPORARY_MODE_CONFLICT".into()),
    };
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(next)
}

/// # Errors
///
/// Returns an error message for a stale `expected_revision` or a
/// database/worker failure.
#[tauri::command]
pub async fn set_temporary_conversation(
    layout: State<'_, AppDataLayout>,
    temporary: bool,
    expected_revision: i64,
) -> Result<MemoryCenterDto, String> {
    let path = database_path(&layout);
    tauri::async_runtime::spawn_blocking(move || {
        set_temporary_at(&path, temporary, expected_revision, now())?;
        memory_center_at(&path, now())
    })
    .await
    .map_err(|error| error.to_string())?
}

fn delete_memory_at(path: &std::path::Path, memory_id: i64) -> Result<(), String> {
    if memory_id <= 0 {
        return Err("invalid memory id".into());
    }
    let database = Database::open(path).map_err(|error| error.to_string())?;
    let mut store = SqliteMemoryStore::new(database);
    store
        .delete_memory_fenced(memory_id)
        .map_err(|error| error.to_string())
}

/// Deletes one durable memory behind the same reset and tombstone fences as
/// the existing "delete all" command.
///
/// # Errors
///
/// Returns an error message for an invalid memory id or a database/worker
/// failure.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub async fn delete_memory<R: Runtime>(
    app: AppHandle<R>,
    memory_id: i64,
) -> Result<MemoryCenterDto, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let layout = app.state::<AppDataLayout>();
        let path = database_path(&layout);
        let chat = app.state::<ChatService>();
        chat.with_exclusive_reset(|| {
            delete_memory_at(&path, memory_id)?;
            memory_center_at(&path, now())
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn center_snapshot_is_bounded_redacted_and_compare_and_set() {
        let path = std::env::temp_dir().join(format!(
            "pw-memory-center-{}-{}.sqlite3",
            std::process::id(),
            now()
        ));
        let _ = std::fs::remove_file(&path);
        let database = Database::open(&path).unwrap();
        database
            .connection()
            .execute("INSERT INTO memory_observations(id,conversation_id,turn_id,user_text,input_hash,observed_at,created_at,updated_at) VALUES(1,'default',1,'[redacted]','hash',1,1,1)", [])
            .unwrap();
        database
            .connection()
            .execute("INSERT INTO memory_classification_runs(id,observation_id,classifier_version,schema_version,input_hash,lease_attempt_token,transport_outcome,created_at) VALUES(1,1,'test',1,'hash','lease','pending',1)", [])
            .unwrap();
        database
            .connection()
            .execute("INSERT INTO memory_candidates(observation_id,classification_run_id,candidate_ordinal,content,subject_scope,epistemic_form,attribution,speech_act,source_mode,polarity,conditionality,fictionality,verification_status,temporal_scope,proposed_operation,proposed_relation,source_start,source_end,memory_domain,write_class,candidate_state,policy_state,created_at,updated_at) VALUES(1,1,0,?1,'user_self','fact_claim','user','asserted','direct','affirmed','actual','real_world','user_reported','stable','add','originated',0,1,'semantic_user','personal','pending','pending_approval',1,1)", ["token=legacy-secret-value-1234567890abcdef {}".replace("{}", &"x".repeat(200))])
            .unwrap();
        database
            .connection()
            .execute(
                "INSERT INTO memories(content,created_at,updated_at) VALUES(?1,1,1)",
                ["authorization: Bearer legacy-secret-value-1234567890abcdef"],
            )
            .unwrap();
        database
            .connection()
            .execute("INSERT INTO dialogue_states(conversation_id,mood,relationship_summary,expires_at,revision,updated_at) VALUES('default',?1,?2,100,1,1)", ["mood=legacy-secret-value-1234567890abcdef", "authorization: Bearer legacy-secret-value-1234567890abcdef"])
            .unwrap();
        drop(database);

        let center = memory_center_at(&path, 1).unwrap();
        assert_eq!(center.domains.len(), 8);
        assert_eq!(center.pending[0].preview.chars().count(), PREVIEW_LIMIT + 1);
        assert!(!center.pending[0].preview.contains("legacy-secret"));
        assert_eq!(center.memories.len(), 1);
        assert!(!center.memories[0].preview.contains("legacy-secret"));
        let dialogue = center.dialogue.expect("dialogue summary");
        assert!(!dialogue.mood.unwrap().contains("legacy-secret"));
        assert!(
            !dialogue
                .relationship_summary
                .unwrap()
                .contains("legacy-secret")
        );
        assert!(set_domain_control_at(&path, "not-a-domain", "allowed", 0, 2).is_err());
        assert_eq!(
            set_domain_control_at(&path, "semantic_user", "never_store", 0, 2)
                .unwrap()
                .revision,
            1
        );
        delete_memory_at(&path, center.memories[0].id).unwrap();
        let database = Database::open(&path).unwrap();
        assert_eq!(
            database
                .connection()
                .query_row("SELECT COUNT(*) FROM memories", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            database
                .connection()
                .query_row("SELECT COUNT(*) FROM memory_tombstones", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        drop(database);
        assert_eq!(set_temporary_at(&path, true, 0, 2).unwrap(), 1);
        assert!(set_temporary_at(&path, false, 0, 3).is_err());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn memory_edit_and_csv_paths_enforce_bounds_cas_and_valid_rows() {
        let root =
            std::env::temp_dir().join(format!("pw-memory-edit-{}-{}", std::process::id(), now()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("main.sqlite3");
        let database = Database::open(&path).unwrap();
        database
            .connection()
            .execute(
                "INSERT INTO memories(content,created_at,updated_at) VALUES('before',1,1)",
                [],
            )
            .unwrap();
        let memory_id = database.connection().last_insert_rowid();
        drop(database);

        assert_eq!(memory_content_at(&path, memory_id).unwrap(), "before");
        update_memory_at(&path, memory_id, "after", 1).unwrap();
        assert_eq!(memory_content_at(&path, memory_id).unwrap(), "after");
        assert_eq!(
            update_memory_at(&path, memory_id, "stale", 1).unwrap_err(),
            "MEMORY_CONFLICT"
        );
        assert_eq!(
            update_memory_at(&path, memory_id, &"x".repeat(MAX_MEMORY_CHARS + 1), 2).unwrap_err(),
            "INVALID_MEMORY_CONTENT"
        );

        let exported = root.join("memories.csv");
        export_memories_csv_at(&path, &exported, false).unwrap();
        let exported_text = std::fs::read_to_string(&exported).unwrap();
        assert!(exported_text.starts_with("id,domain,content,state,pinned,created_at,updated_at"));
        assert!(exported_text.contains(",semantic_user,after,"));

        let imported = root.join("import.csv");
        std::fs::write(
            &imported,
            "id,domain,content,state,pinned,created_at,updated_at\r\n,relationship,imported,active,false,2,2\r\n",
        )
        .unwrap();
        import_memories_csv_at(&path, &imported).unwrap();
        assert_eq!(memory_center_at(&path, now()).unwrap().memories.len(), 2);
        let reexported = root.join("memories-reexported.csv");
        export_memories_csv_at(&path, &reexported, false).unwrap();
        assert!(
            std::fs::read_to_string(&reexported)
                .unwrap()
                .contains(",relationship,imported,")
        );

        let invalid = root.join("invalid.csv");
        std::fs::write(
            &invalid,
            "id,domain,content,state,pinned,created_at,updated_at\r\n,invalid,bad,active,false,2,2\r\n",
        )
        .unwrap();
        assert_eq!(
            import_memories_csv_at(&path, &invalid).unwrap_err(),
            "INVALID_MEMORY_CSV_ROW"
        );

        create_commitment_at(&path, "first task", None, 3).unwrap();
        let commitment = memory_center_at(&path, now()).unwrap().commitments[0].clone();
        assert_eq!(commitment.content, "first task");
        update_commitment_at(
            &path,
            commitment.id,
            "renamed task",
            "open",
            Some(10),
            commitment.revision,
            4,
        )
        .unwrap();
        assert_eq!(
            update_commitment_at(
                &path,
                commitment.id,
                "stale task",
                "open",
                None,
                commitment.revision,
                5,
            )
            .unwrap_err(),
            "COMMITMENT_CONFLICT"
        );
        delete_commitment_at(&path, commitment.id).unwrap();
        assert!(
            memory_center_at(&path, now())
                .unwrap()
                .commitments
                .is_empty()
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
