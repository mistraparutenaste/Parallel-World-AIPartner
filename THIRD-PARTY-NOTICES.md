# サードパーティ表記 / Third-Party Notices

Parallel World本体のライセンスは[LICENSE](LICENSE)（PolyForm Noncommercial License 1.0.0）および[LICENSE-COMMERCIAL.md](LICENSE-COMMERCIAL.md)に定めます。本ファイルに記載する構成要素は**その対象外**であり、それぞれの提供元が定める条件に従います。当方の商用ライセンスは、これらについて何の権利も許諾しません。

## Live2D Cubism SDK

`third_party/live2d/` に配置されます（Gitでは追跡せず、セットアップ時に取得します）。

- Cubism Web Samples — `third_party/live2d/CubismWebSamples/LICENSE.md`
- Cubism Web MotionSync Components — `third_party/live2d/CubismWebMotionSyncComponents/LICENSE.md`

**重要**: Live2Dの規定では、直近会計年度の売上高が1000万円以上の事業者が利用する場合、[Cubism SDK リリースライセンス（出版許諾契約）](https://www.live2d.com/ja/download/cubism-sdk/release-license/)への同意が別途必要です。この義務はParallel Worldの商用ライセンスとは独立しており、当方が代替することはできません。

## 音声モデル・エンジン

いずれも同梱せず、利用者の環境で取得または起動します。各提供元の条件を確認してください。

| 構成要素 | 用途 | 参照 |
| --- | --- | --- |
| Silero VAD | 音声区間検出 | 配布元のライセンス |
| ReazonSpeech | 音声認識モデル | 配布元のライセンス |
| sherpa-onnx | 音声認識ランタイム | crate同梱ライセンス |
| AivisSpeech | 音声合成エンジン | 各エンジンのライセンス |
| Irodori-TTS | 音声合成エンジン・基礎モデル | [docs/setup/irodori-tts.md](docs/setup/irodori-tts.md) |

## フォント

ラノベPOP v2（<https://flopdesign.booth.pm/>）。同梱の説明書は[こちら](apps/desktop/src/assets/fonts/lanobe-pop-v2/ReadMe.html)です。

- Copyright (C) 2002-2019 M+ FONTS PROJECT
- Copyright (C) 2020 flopdesign.com
- Copyright (C) 2020 Kato Masashi

## キャラクターアセット

キャラクターのモデル、画像、音声はGitリポジトリにも配布bundleにも含みません。利用者が用意する各アセットの利用条件に従ってください。

## 依存OSS

Rust crateおよびnpmパッケージの一覧は次のコマンドで生成できます。生成結果はこのファイルの下に追記するか、`THIRD-PARTY-DEPENDENCIES.md`として出力してください。

```bash
cargo about generate --workspace about.hbs -o THIRD-PARTY-DEPENDENCIES.md
```

```bash
corepack pnpm licenses list --prod --recursive
```

PolyForm Noncommercial LicenseはOSI承認のオープンソースライセンスではないため、コピーレフト系（GPL / AGPL / LGPL）の依存が混入すると矛盾します。依存追加時は`cargo deny check licenses`で検査してください。
