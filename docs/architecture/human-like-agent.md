# 人間らしい対話エージェントの境界

この実装は、通常の発話を遅くせずに、状態を小さく分離して扱う。

## 状態モデル

メモリのドメインは次の8つに固定する。

`working`、`episode`、`semantic_user`、`relationship`、`ai_self`、`procedural`、`commitment`、`reflection`

Rust側の typed state（`MemoryDomain`、`DomainControl`、`DialogueState`、
`Commitment`、`TemporaryConversationSettings`）を唯一の書込み境界とし、
WebView/TypeScriptはDTO経由で読み書きする。観測ログ、候補、承認済みメモリ、
関係/コミットメント状態は別テーブルで管理し、DTOや診断ログへ生の会話履歴を出さない。

## プライバシーと削除

一時会話は観測・候補・承認済みメモリ・コミットメント・対話状態への書込みを
同一トランザクションの条件付きSQLで拒否する。ドメイン同意は
`allowed` / `pending_approval` / `never_store` の3値で、候補の昇格時にも再検査する。
個別削除・全削除・履歴削除は `memory_tombstones` と世代(CAS)を先に更新し、
遅れて到着した昇格やリンクが削除済み行を復活させない。Memory Centerのプレビューは
機密パターンを `[REDACTED]` に置換し、文字数と件数を制限する。

## 応答ルーティングと遅延

語彙ルーターが `Simple` と、memory/commitment/correction/decision-support/tool/
proactive の計7種を分ける。Simpleは既存のLLMストリームを1本だけ実行し、追加の
planner・検索・埋め込み・TTS呼出しを行わない。計画対象でもplanner/retriever/realizer
は30msの準備予算内でのみ実行し、タイムアウト・異常・不正な計画は元のプロンプトへ
fail-openするため、返信とストリーミングTTSは継続する。目標はモデル時間を除く
Simpleターンp95 200ms以下、計画検索は100ms以下である。

## 任意の埋め込みとFTSフォールバック

`pw-application::memory::MemoryEmbedder` はローカル実装を明示的に渡した場合だけ使う
ポートである。既定値は `LexicalFallback::disabled()` で、SQLite FTS/LIKEの結果を
そのまま使う。埋め込みへ渡すクエリは240文字、候補は16件、各スニペットは512文字、
待ち時間は最大100msに制限する。adapterの未設定、無効化、遅延、panic、エラー、
範囲外/重複/未知IDのスコアはすべて語彙順へ戻る。ベクトルをSQLiteへ永続化せず、
このポートからネットワークや有料APIを呼ばない。したがって普通のchat/reply/TTSに
追加呼出しは発生しない。

## 検証と環境ゲート

Rustのmigration/reopen/FTS、削除フェンス、temporary/CAS、完了ターンの状態更新、
proactiveのcancel/rate-limit、response routing、Memory Centerのredaction/boundsを
focused testで検証する。`cargo fmt --all -- --check`、workspace cargo test、
契約・capabilityのdiff-checkに加え、利用可能な環境ではdesktop Vitest/typecheck/build
と起動スモークを別々に実行する。DesktopのLive2D runtime alias/dist、Sherpaモデル、
外部LLM/TTS（loopbackを含む）は環境依存ゲートであり、未配置・未起動でもテキスト会話の
fail-open動作を壊さない。結果と未達ゲートは`.superpowers/sdd/task-7-report.md`へ記録する。
