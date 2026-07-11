# Phase 1〜7 外部ゲート資産監査

- 監査日: 2026-07-11
- 対象: `.worktrees/phase-0-foundation` と、同worktreeから参照する親ワークスペースのGit除外領域 `project-input/` / `third_party/`
- 方法: ファイル名・サイズ・公開ライセンス文書・既存manifestを読み取り専用で確認した。秘密鍵、APIキー、証明書の秘密値、環境変数の値は読んでいない。
- 判定語: **開発可** = ローカル開発/結合試験に使える実物がある、**配布不可** = 現在の証跡だけでは配布物へ含められない、**未提供** = 実物または利用条件の証跡がない。

## 結論

| 区分 | 現物 | ライセンス/hash証跡 | 現状の利用可否 |
|---|---|---|---|
| Live2D Cubism SDK for Web | 5-r.5一式、Coreあり | 公式ライセンス文書、archive/file SHA-256あり | **開発可、配布不可**。Release License要否と製品配布条件の確認が必要 |
| Live2D MotionSync for Web | 5-r.2一式、CRI Coreあり | 公式ライセンス文書、再配布可能ファイル一覧、SHA-256あり | **開発可、配布不可**。CRIを含む製品組込み条件を追加確認 |
| Live2Dモデル | 11モデル群、285ファイル、約153.5 MB | 19 asset manifest、全assetの `redistributionApproved=false` | **ローカル試験可、製品同梱不可** |
| STT (ReazonSpeech/sherpa-onnx) | モデル、runtimeともなし | version/hash/license manifestなし | **未提供** |
| VAD (Silero VAD) | ONNX等なし | version/hash/license manifestなし | **未提供** |
| AivisSpeech | Engine実行物、音声モデルともなし | version/hash/license manifestなし | **外部起動前提でも未検証** |
| LLM | `llama-server`、GGUF、外部API設定証跡なし | version/hash/license manifestなし | **未提供**。OpenAI互換mock以外の実接続ゲートが残る |
| 署名/updater | `.sig`、公開鍵、配布artifact、公開endpointなし | 公開用証跡なし | **未提供**。秘密値を使わないmock署名検証のみ実装可能 |

したがって、現時点で外部実物を使って前進できるのはLive2Dのローカル開発・試験だけである。Phase 2以降の音声/LLM実動作とPhase 7の製品配布は、以下の不足資産を取得し、version・SHA-256・licenseをmanifest化するまで受け入れ不可とする。

## 1. Live2D SDK / Core

親ワークスペースのGit除外領域に実物がある。worktree側には大容量/プロプライエタリ実物を置かず、公開ライセンスとmanifestだけが追跡されている。

| 現物 | version | サイズ | SHA-256 | 状態 |
|---|---:|---:|---|---|
| `project-input/live2d/downloads/sdk/CubismSdkForWeb-5-r.5.zip` | 5-r.5 | 20,708,681 B | `67064a7fb1812cf502f5c4a03bfe12cc638c75a621bb4acf06bb28763df06ba0` | あり |
| `third_party/live2d/CubismWebSamples/Core/live2dcubismcore.js` | 5-r.5 | 246,861 B | asset manifestに記録済み | あり |
| `third_party/live2d/CubismWebSamples/Core/live2dcubismcore.min.js` | 5-r.5 | 228,042 B | asset manifestに記録済み | あり |
| `project-input/live2d/downloads/motionsync/CubismSdkMotionSyncPluginForWeb-5-r.2.zip` | 5-r.2 | 14,640,671 B | `74dc9236e28263f9cba7962704ce61ad06677e79ca6e56657f216a098015452f` | あり |
| `third_party/live2d/CubismWebMotionSyncComponents/Core/CRI/live2dcubismmotionsynccore.js` | 5-r.2 | 584,818 B | asset manifestに記録済み | あり |
| 同 `*.min.js` | 5-r.2 | 563,022 B | asset manifestに記録済み | あり |

既存の公開証跡:

- CubismWebSamples: source version `5-r.5`、commit `ed1e0b714826d92469b9e51cacc3346f4e393f03`。
- CubismWebMotionSyncComponents: source version `5-r.2`、commit `b16600aba6f367b50bda8b4cf901725dbc6e3631`。
- `CubismCore-RedistributableFiles.txt` は `live2dcubismcore.d.ts` / `.js` / `.min.js` を再配布可能ファイルとして列挙する。
- `CubismMotionSyncCore-CRI-RedistributableFiles.txt` は `live2dcubismmotionsynccore.d.ts` / `.js` / `.js.map` / `.min.js` を列挙する。
- ローカルのSDKライセンス文書には、直近会計年度売上高1,000万円超の事業者はCubism SDK Release Licenseが必要との記載がある。事業区分・契約締結の証跡はリポジトリにないため、製品配布を許可する根拠にはしない。
- MotionSync CoreにはLive2D Open Software LicenseとCRI Core用Live2D Proprietary Software Licenseの両方が関係する。CRI関連条件を法務/提供元へ確認するまで製品同梱しない。

## 2. Live2Dモデル

`project-input/live2d/selected/` に次の11モデル群、285ファイル、153,501,759 Bがある。

| モデル | ファイル数 | サイズ | 目的 | 配布判定 |
|---|---:|---:|---|---|
| Haru | 47 | 4,277,148 B | 表示/モーション/音量ベースlip sync | 不可 |
| Hiyori | 18 | 4,924,918 B | 補助試験 | 不可 |
| Mao | 22 | 4,321,713 B | 表情/Blend Shape | 不可 |
| Mark | 12 | 691,046 B | 最小モデル | 不可 |
| Ren | 13 | 2,530,402 B | 高度描画 | 不可 |
| Rice | 10 | 3,152,599 B | mask/interpolation | 不可 |
| Wanko | 17 | 765,549 B | 非人型補助試験 | 不可 |
| Kei | 33 | 32,843,704 B | MotionSync/4言語WAV | 不可 |
| Epsilon | 66 | 27,638,441 B | 表情/感情mapping | 不可 |
| Simple model | 8 | 942,871 B | 最小描画試験 | 不可 |
| Haru receptionist | 39 | 71,414,368 B | 追加UI/シナリオ試験 | 不可 |

`assets.json` はSDK/Core/archiveを含む19 assetを記録し、全件 `redistributionApproved=false` である。モデルはFree Material License / Sample Model Terms等の個別条件があるため、ローカル開発と結合試験に限定する。製品同梱モデルは、モデル単位の再配布許諾と必要な著作権表示を別途確定する。

### 既存archive

| archive | サイズ | SHA-256 |
|---|---:|---|
| `kei_ja.zip` | 18,790,626 B | `4a81ae50232170c9556e1c27f64fa383969e85b9d5aa69aa112206dbf6890928` |
| `epsilon_ja.zip` | 26,626,829 B | `a2e4d747bb0fca4f5920637ac8acc350b08c46e3275344a273cb9c37d405f9e1` |
| `simple_ja.zip` | 904,359 B | `a6b19a98a883e455622fa258d99ddb372ed13244ed82520c1fa18b98990e2c2e` |
| `haru_greeter_ja.zip` | 40,966,036 B | `3db5c9180fc8446f7d92eee13ea33f584e5d1308e101d7afa4fe3b037a0ed94e` |

## 3. manifest / ライセンス文書の完全性

小規模な追跡ファイルの監査用SHA-256:

| ファイル | サイズ | SHA-256 |
|---|---:|---|
| `project-input/live2d/manifests/assets.json` | 70,505 B | `d5745acdd27f186476fa0b212bad1cf876333bda55f834e73cc37f5895502b1f` |
| `project-input/live2d/manifests/licenses_sha256.txt` | 2,537 B | `7599c5d8e43fd9e424a6259666db568f32f1b8c68605e43c789cb2dc51f84fdb` |
| `project-input/live2d/licenses/CubismSdkForWeb-LICENSE.md` | 3,109 B | `64c277df4479eeee3652dfb7da5be5dcb4d3e041b0ee7a34d076e79078994ee0` |
| `project-input/live2d/licenses/CubismCore-LICENSE.md` | 527 B | `ab2b9d38b378aa6c3c6fd0e888e5c335ae811d76e4236c913aa4af609c091da8` |

注意点:

1. 親ワークスペース側の25ライセンス文書は、親側 `licenses_sha256.txt` と25/25一致した。
2. worktree側は21文書しかなく、MotionSync Core関係4文書（`CubismMotionSyncCore-LICENSE.md`、README、CRI LICENSE、CRI RedistributableFiles）が追跡されていない。配布物のNOTICE生成時に親の無視領域へ依存してはならないため、公開可能性を確認後に追跡対象へ追加する必要がある。
3. `assets.json` の全19 assetについて実ファイルの有無とサイズを照合した。Haru receptionistの日本語PSD 2件だけmanifest内パスが文字化けし、実ファイル名と一致しない。実ファイル自体は存在する。runtimeに不要なPSDでも、Phase 7のhash検証manifestとしては正規UTF-8名へ修正が必要。
4. `SOURCE_URLS.md` / `DOWNLOAD_REPORT.md` / 一部ライセンスコピーは日本語が文字化けしている。法的表示や監査証跡としてそのまま出荷しない。

## 4. STT / VAD

リポジトリ全体（`node_modules`、`target`、`.git`等を除外）に `.onnx`、`.ort`、`.tflite`、`.mlmodel` は0件で、sherpa-onnx runtimeもない。設計上の名称（ReazonSpeech、sherpa-onnx、Silero VAD）が文書にあるだけで、次が未提供である。

- ReazonSpeech Zipformerモデル一式と固定version
- Silero VAD ONNXと固定version
- sherpa-onnx runtime/ネイティブライブラリの固定version
- 各download URL、配布元、SHA-256、展開後ファイル一覧
- model/runtimeのLICENSE、NOTICE、再配布可否

Phase 2/6/7の実物受け入れには、`models/*.json` のようなmanifestを作り、download前にlicense、download後にSHA-256、展開時にpath traversal、起動時にmodel versionを検証する必要がある。

## 5. AivisSpeech

実行ファイル、ONNX、音声モデル、manifest、LICENSE/NOTICEはいずれもない。基本設計は初期開発でユーザーがAivisSpeech Engineを別途起動する構成としているため、Engineそのものを同梱しない実装は可能だが、実結合試験には以下が必要である。

- 対応AivisSpeech Engine versionと公式配布元
- loopback endpointの契約、および `/speakers` 応答を使う試験環境
- 使用する音声モデルごとの利用規約・クレジット・商用/再配布条件
- Engine/音声モデルをアプリがdownloadする場合のSHA-256 manifest

speaker/styleの固定IDを製品前提にせず、実環境から列挙する設計を維持する。

## 6. LLM

`llama-server`、GGUF、`.safetensors`、version/hash/license manifestは0件である。OpenAI互換HTTP adapterはmockで実装・契約試験できるが、実LLM受け入れは未達である。

- ローカル利用: 固定したllama.cpp releaseと対応 `llama-server`、GGUFごとのmodel card/license/SHA-256が必要。
- 外部API利用: endpoint/model IDの設定と、秘密値をリポジトリへ保存しないcredential storageが必要。
- いずれもストリーム、キャンセル、timeout、429/5xx、互換差分の契約試験対象とする。

## 7. 署名 / updater

秘密値は探索・表示していない。公開成果物だけを確認した結果、`.sig` 0件、updater公開鍵0件、MSI/MSIX/DMG/AppImage 0件で、Tauri設定にもupdater endpoint/public keyはまだない。

Phase 7の製品公開に必要だが未提供の外部ゲート:

- Windowsコード署名証明書とCI署名権限
- Apple Developer ID、notarization資格情報、macOS実機runner
- Tauri updater署名鍵（秘密鍵はCI secretのみ）とアプリへ埋め込む公開鍵
- HTTPS配布endpoint、署名済み更新artifact、更新manifest

外部ゲートがなくても、fixture用テスト鍵をテスト専用ディレクトリに生成し、署名検証を無効化できないmock updater試験までは実装可能である。fixture秘密鍵を製品設定、配布artifact、ログへ含めてはならない。

## 8. Phase 1〜7へのゲート対応

| Phase | 外部資産上の判定 |
|---|---|
| Phase 1 Live2D | SDK/Core/モデル実物あり。ローカル開発可。配布は不可 |
| Phase 2 音声入力/STT/VAD | 実物なし。mockとadapterまで可、実認識受け入れ不可 |
| Phase 3 LLM | 実物なし。OpenAI互換mock契約まで可、実応答受け入れ不可 |
| Phase 4 TTS/lip sync | AivisSpeechなし。mock PCMとLive2Dローカル試験まで可 |
| Phase 5 永続化/記憶 | 外部model資産への依存なし。ただし実LLM統合試験はPhase 3ゲート継承 |
| Phase 6 障害回復 | 各外部サービスのmock fault試験可。実runtime再初期化は未検証 |
| Phase 7 配布/更新 | 署名・notarization・endpoint未提供。開発buildとmock署名検証まで可 |

## 9. 取得時の必須受け入れ項目

新しい外部資産は、少なくとも `id`、`version`、公式URL、取得日、archive SHA-256、展開後重要ファイルSHA-256、license identifier/URL、NOTICE、再配布可否、対象OS/arch、runtime互換versionを記録する。再配布許諾が `true` と証拠付きで確定するまで、Git、installer、updater artifactへ含めない。

この監査は法的助言ではなく、2026-07-11時点のローカル証跡の技術監査である。出荷前に公式の最新版利用規約と契約主体の条件を再確認する。
