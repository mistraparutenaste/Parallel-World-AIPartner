//! Keeps the official updater fixture visible to the integration-test target.
//! The real `Update::download` network test lives beside the backend so Windows
//! does not classify this executable as an elevated updater process.

#[test]
fn signed_updater_fixture_is_present() {
    assert!(!include_bytes!("../../../../tools/fixtures/updater/fixture-update.bin").is_empty());
    assert!(
        !include_str!("../../../../tools/fixtures/updater/fixture-update.bin.sig")
            .trim()
            .is_empty()
    );
    assert!(
        !include_str!("../../../../tools/fixtures/updater/test-public.key")
            .trim()
            .is_empty()
    );
}
