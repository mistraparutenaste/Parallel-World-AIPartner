use pw_platform::activity::{DataProtector, DpapiProtector};

#[cfg(windows)]
#[test]
fn activity_dpapi_round_trips_empty_and_nonempty_payloads() {
    let protector = DpapiProtector;

    for plaintext in [b"".as_slice(), b"private activity context".as_slice()] {
        let protected = protector.protect(plaintext).expect("payload is protected");
        assert!(!protected.is_empty());
        assert_ne!(protected, plaintext);
        assert_eq!(
            protector
                .unprotect(&protected)
                .expect("payload is unprotected"),
            plaintext
        );
    }
}

#[cfg(windows)]
#[test]
fn activity_dpapi_rejects_tampered_ciphertext_without_plaintext_in_error() {
    const SENTINEL: &[u8] = b"ACTIVITY-DPAPI-PLAINTEXT-SENTINEL";
    let protector = DpapiProtector;
    let mut protected = protector.protect(SENTINEL).expect("sentinel is protected");
    let last = protected.last_mut().expect("DPAPI ciphertext is nonempty");
    *last ^= 0x80;

    let error = protector
        .unprotect(&protected)
        .expect_err("tampered ciphertext is rejected");
    assert!(
        !error
            .to_string()
            .contains("ACTIVITY-DPAPI-PLAINTEXT-SENTINEL")
    );
}

#[cfg(not(windows))]
#[test]
fn activity_dpapi_is_explicitly_unsupported_off_windows() {
    let protector = DpapiProtector;
    assert!(protector.protect(b"private").is_err());
    assert!(protector.unprotect(b"protected").is_err());
}
