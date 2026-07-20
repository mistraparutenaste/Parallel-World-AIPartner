//! Privacy-safe Memory Center IPC.  All SQLite work stays in blocking workers;
//! the renderer only receives bounded summaries.

use pw_contracts::{
    CommitmentSummaryDto, DialogueSummaryDto, MemoryCenterDto, MemoryDomainControlDto,
    PendingMemoryCandidateDto, SCHEMA_VERSION,
};
use pw_platform::paths::AppDataLayout;
use pw_storage::Database;
use rusqlite::{OptionalExtension, params};
use tauri::State;

const CONVERSATION: &str = "default";
const PREVIEW_LIMIT: usize = 160;

fn database_path(layout: &AppDataLayout) -> std::path::PathBuf {
    layout.data.join("parallel-world.sqlite3")
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| i64::try_from(value.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn bounded_preview(value: &str) -> String {
    let trimmed = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut preview = trimmed.chars().take(PREVIEW_LIMIT).collect::<String>();
    if trimmed.chars().count() > PREVIEW_LIMIT {
        preview.push('…');
    }
    preview
}

fn memory_center_at(path: &std::path::Path, now: i64) -> Result<MemoryCenterDto, String> {
    let database = Database::open(path).map_err(|error| error.to_string())?;
    let connection = database.connection();
    let domains = connection
        .prepare("SELECT domain,consent,retention_seconds,revision FROM memory_domain_controls ORDER BY domain")
        .map_err(|error| error.to_string())?
        .query_map([], |row| Ok(MemoryDomainControlDto {
            domain: row.get(0)?, consent: row.get(1)?, retention_seconds: row.get(2)?, revision: row.get(3)?,
        }))
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>().map_err(|error| error.to_string())?;
    let pending = connection
        .prepare("SELECT id,memory_domain,content,created_at FROM memory_candidates WHERE candidate_state='pending' AND policy_state!='rejected' ORDER BY created_at DESC,id DESC LIMIT 50")
        .map_err(|error| error.to_string())?
        .query_map([], |row| {
            let content: String = row.get(2)?;
            Ok(PendingMemoryCandidateDto { id: row.get(0)?, domain: row.get(1)?, preview: bounded_preview(&content), created_at: row.get(3)? })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>().map_err(|error| error.to_string())?;
    let commitments = connection
        .prepare("SELECT id,status,due_at,revision FROM commitments WHERE status='open' ORDER BY COALESCE(due_at,9223372036854775807),id LIMIT 50")
        .map_err(|error| error.to_string())?
        .query_map([], |row| Ok(CommitmentSummaryDto { id: row.get(0)?, status: row.get(1)?, due_at: row.get(2)?, revision: row.get(3)? }))
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>().map_err(|error| error.to_string())?;
    let dialogue = connection.query_row(
        "SELECT mood,relationship_summary,relationship_score,expires_at,revision FROM dialogue_states WHERE conversation_id=?1 AND expires_at>?2",
        params![CONVERSATION, now],
        |row| Ok(DialogueSummaryDto { mood: row.get(0)?, relationship_summary: row.get(1)?, relationship_score: row.get(2)?, expires_at: row.get(3)?, revision: row.get(4)? }),
    ).optional().map_err(|error| error.to_string())?;
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
        pending,
        commitments,
        dialogue,
        temporary,
        temporary_revision,
    })
}

#[tauri::command]
pub async fn get_memory_center(
    layout: State<'_, AppDataLayout>,
) -> Result<MemoryCenterDto, String> {
    let path = database_path(&layout);
    tauri::async_runtime::spawn_blocking(move || memory_center_at(&path, now()))
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
    if !matches!(consent, "allowed" | "pending_approval" | "never_store") || expected_revision < 0 {
        return Err("invalid memory control".into());
    }
    let database = Database::open(path).map_err(|error| error.to_string())?;
    let changed = database.connection().execute(
        "UPDATE memory_domain_controls SET consent=?1,revision=revision+1,updated_at=?2 WHERE domain=?3 AND revision=?4",
        params![consent, now, domain, expected_revision],
    ).map_err(|error| error.to_string())?;
    if changed == 0 {
        return Err("MEMORY_CONTROL_CONFLICT".into());
    }
    database.connection().query_row(
        "SELECT domain,consent,retention_seconds,revision FROM memory_domain_controls WHERE domain=?1", [domain],
        |row| Ok(MemoryDomainControlDto { domain: row.get(0)?, consent: row.get(1)?, retention_seconds: row.get(2)?, revision: row.get(3)? }),
    ).map_err(|error| error.to_string())
}

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
            transaction.execute("INSERT INTO temporary_conversations(conversation_id,temporary,revision,updated_at) VALUES(?1,?2,1,?3)", params![CONVERSATION, temporary, now]).map_err(|error| error.to_string())?;
            1
        }
        Some(value) if value == expected_revision => {
            transaction.execute("UPDATE temporary_conversations SET temporary=?1,revision=revision+1,updated_at=?2 WHERE conversation_id=?3 AND revision=?4", params![temporary, now, CONVERSATION, value]).map_err(|error| error.to_string())?;
            value + 1
        }
        _ => return Err("TEMPORARY_MODE_CONFLICT".into()),
    };
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(next)
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn center_snapshot_is_bounded_and_uses_compare_and_set() {
        let path =
            std::env::temp_dir().join(format!("pw-memory-center-{}.sqlite3", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let database = Database::open(&path).unwrap();
        database.connection().execute("INSERT INTO memory_observations(id,conversation_id,turn_id,user_text,input_hash,observed_at,created_at,updated_at) VALUES(1,'default',1,'[redacted]','hash',1,1,1)", []).unwrap();
        database.connection().execute("INSERT INTO memory_classification_runs(id,observation_id,classifier_version,schema_version,input_hash,lease_attempt_token,transport_outcome,created_at) VALUES(1,1,'test',1,'hash','lease','pending',1)", []).unwrap();
        database.connection().execute("INSERT INTO memory_candidates(observation_id,classification_run_id,candidate_ordinal,content,subject_scope,epistemic_form,attribution,speech_act,source_mode,polarity,conditionality,fictionality,verification_status,temporal_scope,proposed_operation,proposed_relation,source_start,source_end,memory_domain,write_class,candidate_state,policy_state,created_at,updated_at) VALUES(1,1,0,?1,'user_self','fact_claim','user','asserted','direct','affirmed','actual','real_world','user_reported','stable','add','originated',0,1,'semantic_user','personal','pending','pending_approval',1,1)", ["x".repeat(300)]).unwrap();
        drop(database);
        let center = memory_center_at(&path, 1).unwrap();
        assert_eq!(center.domains.len(), 8);
        assert_eq!(center.pending[0].preview.chars().count(), PREVIEW_LIMIT + 1);
        assert_eq!(
            set_domain_control_at(&path, "semantic_user", "never_store", 0, 2)
                .unwrap()
                .revision,
            1
        );
        assert_eq!(set_temporary_at(&path, true, 0, 2).unwrap(), 1);
        assert!(set_temporary_at(&path, false, 0, 3).is_err());
        let _ = std::fs::remove_file(path);
    }
}
