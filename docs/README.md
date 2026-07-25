# Documentation

このディレクトリには、共有・保守するプロジェクト文書を用途別に格納します。

## Product

- [アイデア](product/IDEA.md)
- [基本設計](product/基本設計.md)

## Development

- [作業ログ](development/worklogs/README.md)
- [Design QA](development/design-qa.md)
- [実機テスト結果](development/test-results/)
- [開発・引継ぎ資料](development/)

ソークテストは `tools/scripts/soak-test.ps1` による手動運用です（CI・npm scriptsからは呼ばれません）。手順は [ソークテスト手順](development/soak-test.md) を参照してください。

## Reference

- [調査資料](research/)
- [実装計画](plans/)（フェーズ別の計画・仕様は [superpowers/plans](superpowers/plans/)・[superpowers/specs](superpowers/specs/)）
- [検証資料](verification/)
- [アーキテクチャ](architecture/)
- [ADR](adr/)
- [セットアップ](setup/)

動作確認用のUIモックは [`tools/ui-mocks/`](../tools/ui-mocks/) に格納します。
