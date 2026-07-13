# Phase 6 Task 6 work log

## 実装

- `pw-platform::diagnostics`: schema付きJSON、panic payload分類、thread/location/backtrace、秘密情報除去、temp+rename、20件/20 MiB保持。
- panic hookはmain先頭で導入し、thread-local再入guard、hook内保持整理禁止、任意backtrace上限を実装。panic処理中の再panicはabortとなるため、任意の既存hookは呼び出さず、安全な独自hookへ置換する。
- Windows exportは監査可能な単一unsafe境界で `MoveFileExW(REPLACE_EXISTING | WRITE_THROUGH)` を使い、timestamp/pid/atomic sequence + create-newの競合耐性tempから原子的に置換。
- bootstrap: panic hookを一度だけ導入し、early-crash storeも起動時に回収。ログにも同じ保持上限を適用。
- frontend: `error` / `unhandledrejection` をmetadataだけに変換するtyped IPC bridge。
- Settings: 診断一覧と明示パスexport。list/exportはSettings Capability限定。
- WER LocalDumps: 任意手動診断として、機密性・ACL・保持・削除の注意事項を文書化。

## TDD / 検証

- RED: 未定義diagnostics module、未実装DiagnosticsPanel、Capability不一致を確認。
- GREEN: `pw-platform`診断テスト2件、Capability 8件、desktop Rust 72件、frontend 49件。
- レビュー追加: concurrent write/retention、atomic no-overwrite競合、旧ファイル保持、Windows atomic overwrite、panic hook隔離、typed report検証、起動時のorphan temp回収を含む診断テスト8件。
- Settings exportの上書き再試行が失敗した場合も未処理Promiseにせず、画面上のstatusへエラーを表示する回帰テストを追加（DiagnosticsPanel 2件）。
- tempはRAII guardでwrite/sync/rename失敗時も削除し、通常storeとearly-crash storeの両方を次回起動時に回収する。
- list/exportは所有ファイル名・schema・許可フィールド・typed reportを検証し、外部・旧式・unknown-field JSONを無視してtyped再serializeする。
- panic後の保持整理は容量1のcoalescing signalから専用workerがhook外で実施し、受信競合時も再度maintainする。
- ログwriterは5 MiB単位で巨大writeも分割し、各segment後にactiveを除外して総量20 MiBを即時維持する。rotationテスト2件。
- `cargo fmt --all`、bindings生成、frontend typecheck/build成功。
