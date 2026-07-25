//! Read-only activity review commands.

use pw_contracts::{ACTIVITY_SESSION_SCHEMA_VERSION, ActivitySessionDto, ActivitySessionPageDto};
use pw_platform::activity::{DataProtector, DpapiProtector};
use pw_platform::paths::AppDataLayout;
use pw_storage::activity::{ActivityDatabase, StoredActivitySession};
use serde::Deserialize;
use tauri::State;

const MAX_PAGE_SIZE: u32 = 100;

#[derive(Deserialize)]
struct ProtectedContextPayload {
    version: u16,
    protected_app_id: Vec<u8>,
    protected_title: Vec<u8>,
}

/// Converts one encrypted activity row into its display-safe IPC shape.
///
/// # Errors
///
/// Returns an error when the payload is malformed, uses an unsupported
/// version, cannot be decrypted, or contains invalid text.
pub fn map_activity_session(
    session: &StoredActivitySession,
    protector: &impl DataProtector,
) -> Result<ActivitySessionDto, String> {
    let payload: ProtectedContextPayload = serde_json::from_slice(&session.protected_context)
        .map_err(|_| "activity context is unavailable".to_owned())?;
    if payload.version != ACTIVITY_SESSION_SCHEMA_VERSION {
        return Err("activity context version is unsupported".to_owned());
    }
    let app = protector
        .unprotect(&payload.protected_app_id)
        .map_err(|_| "activity context is unavailable".to_owned())?;
    let title = protector
        .unprotect(&payload.protected_title)
        .map_err(|_| "activity context is unavailable".to_owned())?;
    Ok(ActivitySessionDto {
        schema_version: ACTIVITY_SESSION_SCHEMA_VERSION,
        id: session.id,
        started_at: session.started_at,
        ended_at: session.ended_at,
        duration_seconds: u64::try_from(session.duration_seconds)
            .map_err(|_| "activity duration is invalid".to_owned())?,
        category: session.category.clone(),
        display_app: String::from_utf8(app)
            .map_err(|_| "activity context is unavailable".to_owned())?,
        display_title: String::from_utf8(title)
            .map_err(|_| "activity context is unavailable".to_owned())?,
    })
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
/// Returns one bounded page of activity sessions for local review.
///
/// # Errors
///
/// Returns an error for an invalid page size or when the local activity store
/// cannot be opened, read, or decrypted.
pub fn list_activity_sessions(
    layout: State<'_, AppDataLayout>,
    limit: u32,
    before_id: Option<i64>,
) -> Result<ActivitySessionPageDto, String> {
    if !(1..=MAX_PAGE_SIZE).contains(&limit) {
        return Err("activity page size must be between 1 and 100".to_owned());
    }
    let database = ActivityDatabase::open(layout.activity_database())
        .map_err(|_| "activity history is unavailable".to_owned())?;
    let page = database
        .page_sessions(limit, before_id)
        .map_err(|_| "activity history is unavailable".to_owned())?;
    let sessions = page
        .sessions
        .iter()
        .map(|session| map_activity_session(session, &DpapiProtector))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ActivitySessionPageDto {
        schema_version: ACTIVITY_SESSION_SCHEMA_VERSION,
        sessions,
        next_before_id: page.next_before_id,
    })
}
