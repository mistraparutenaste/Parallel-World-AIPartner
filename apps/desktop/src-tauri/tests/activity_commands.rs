use parallel_world_desktop::commands::activity::map_activity_session;
use pw_platform::activity::{DataProtectionError, DataProtector};
use pw_storage::activity::StoredActivitySession;

struct PlaintextProtector;

impl DataProtector for PlaintextProtector {
    fn protect(&self, plaintext: &[u8]) -> Result<Vec<u8>, DataProtectionError> {
        Ok(plaintext.to_vec())
    }

    fn unprotect(&self, protected: &[u8]) -> Result<Vec<u8>, DataProtectionError> {
        Ok(protected.to_vec())
    }
}

#[test]
fn activity_mapping_decrypts_only_the_bounded_display_fields() {
    let payload = serde_json::json!({
        "version": 1,
        "protected_app_id": b"code.exe".to_vec(),
        "protected_title": "Parallel World".as_bytes(),
        "idle_seconds": 0,
        "fullscreen": false
    });
    let stored = StoredActivitySession {
        id: 7,
        started_at: 100,
        ended_at: Some(130),
        duration_seconds: 30,
        category: "work".to_owned(),
        payload_version: 1,
        protected_context: serde_json::to_vec(&payload).unwrap(),
    };

    let dto = map_activity_session(&stored, &PlaintextProtector).expect("mapped");

    assert_eq!(dto.id, 7);
    assert_eq!(dto.display_app, "code.exe");
    assert_eq!(dto.display_title, "Parallel World");
}

#[test]
fn activity_mapping_rejects_malformed_or_non_utf8_payloads() {
    let stored = StoredActivitySession {
        id: 1,
        started_at: 0,
        ended_at: Some(0),
        duration_seconds: 0,
        category: "other".to_owned(),
        payload_version: 1,
        protected_context: b"not-json".to_vec(),
    };
    assert!(map_activity_session(&stored, &PlaintextProtector).is_err());
}
