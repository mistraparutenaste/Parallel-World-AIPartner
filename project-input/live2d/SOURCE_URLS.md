# Source URLs

## 1. CubismWebSamples（取得済み）

- 取得物: Haru / Hiyori / Mao / Mark / Ren / Rice / Wanko（各モデル一式）
- 公式URL: https://github.com/Live2D/CubismWebSamples
- 取得日: 2026-07-11
- バージョンまたはタグ: 5-r.5
- コミットID: ed1e0b714826d92469b9e51cacc3346f4e393f03
- サブモジュール(Framework): https://github.com/Live2D/CubismWebFramework （コミット d4da0aa07e47d2c1e4f5fa7ea6047861ea5e5d0b）
- ライセンスページ: リポジトリ内 LICENSE.md / NOTICE.md / NOTICE.ja.md
- 個別条件: Natoriはコラボレーションキャラクターのため対象外（商用利用・改変・再配布に制限）
- 使用目的: parallel-worldのLive2D描画・表情・モーション・物理演算検証（local-development / integration-test）
- 保存先: third_party/live2d/CubismWebSamples/（原本）、project-input/live2d/selected/{haru,hiyori,mao,mark,ren,rice,wanko}/（テスト用コピー）

## 2. CubismWebMotionSyncComponents（取得済み・任意機能）

- 取得物: MotionSync公開コンポーネント一式
- 公式URL: https://github.com/Live2D/CubismWebMotionSyncComponents
- 取得日: 2026-07-11
- バージョンまたはタグ: 5-r.2
- コミットID: b16600aba6f367b50bda8b4cf901725dbc6e3631
- ライセンスページ: リポジトリ内 LICENSE.md / NOTICE.md / NOTICE.ja.md
- 個別条件: GitHubリポジトリのみではMotionSyncは動作しない。MotionSync Coreを含む公式配布パッケージ（Phase 6）が別途必要。
- 使用目的: MotionSync機能の検証（任意）
- 保存先: third_party/live2d/CubismWebMotionSyncComponents/（原本）

---

## 3. Kei（取得済み）

- 取得物: Keiモデル一式（kei_vowels_pro / kei_basic_free の2バリアント）
- 公式URL: https://www.live2d.com/en/learn/sample/kei/
- 取得日: 2026-07-11（ユーザーが利用許諾へ同意の上ダウンロード。自動同意はしていない）
- バージョンまたはタグ: kei_ja.zip
- ライセンスページ:
  - Free Material License Agreement（無償提供マテリアルの使用許諾契約書）
  - Live2D Cubism Sample Data Terms of Use: https://www.live2d.com/en/learn/sample/model-terms/
  - 詳細: https://www.live2d.com/download/sample-data/
- 個別条件: 一般ユーザー・小規模事業者（年商1000万円未満）は規約同意により商用利用可。中・大規模事業者は非公開テスト用途のみ。ライセンスタイプ「Live2Dオリジナルキャラクター」。
- 使用目的: MotionSync・音素ベースリップシンク・.motionsync3.json・音声同期・WAVテスト
- 保存先: project-input/live2d/downloads/models/kei/kei_ja.zip（原本ZIP）, originals/models/kei/（展開済み原本）, selected/kei/（テスト用コピー、kei_vowels_pro・kei_basic_free両方）
- 検証結果: 両バリアントとも.model3.json読み込み可、.moc3/テクスチャ/物理演算/表示補助/.motionsync3.json/4言語分のモーション・WAV音声すべて参照整合性OK（reports/FILE_VERIFICATION.md, reports/LIP_SYNC_COMPATIBILITY.md）

## 4. Simple model（取得済み）

- 公式URL: https://www.live2d.com/en/learn/sample/simple-model/
- 取得日: 2026-07-11（ユーザーが利用許諾へ同意の上ダウンロード。自動同意はしていない）
- バージョンまたはタグ: simple_ja.zip（SHA-256: a6b19a98a883e455622fa258d99ddb372ed13244ed82520c1fa18b98990e2c2e）
- ライセンスページ: Free Material License Agreement / Live2D Cubism Sample Data Terms of Use
- 使用目的: 最小構成での描画確認、口開閉、まばたき、角度パラメーター、障害切り分け
- 保存先: project-input/live2d/downloads/models/simple-model/simple_ja.zip（原本ZIP）, originals/models/simple-model/（展開済み原本）, selected/simple-model/（テスト用コピー）
- 検証結果: simple.model3.json読み込み可、.moc3/テクスチャ/.cdi3.json/モーション参照整合性OK。物理演算・表情・MotionSyncは対象モデルでは未提供（reports/FILE_VERIFICATION.md）

## 5. Epsilon（取得済み）

- 公式URL: https://www.live2d.com/en/learn/sample/epsilon/
- 取得日: 2026-07-11（ユーザーが利用許諾へ同意の上ダウンロード。自動同意はしていない）
- バージョンまたはタグ: epsilon_ja.zip（SHA-256: a2e4d747bb0fca4f5920637ac8acc350b08c46e3275344a273cb9c37d405f9e1、epsilon_pro / epsilon_free の2バリアント）
- ライセンスページ: Free Material License Agreement / Live2D Cubism Sample Data Terms of Use
- 使用目的: 標準的なキャラクター表示、複数表情、感情マッピング、表情フェード
- 保存先: project-input/live2d/downloads/models/epsilon/epsilon_ja.zip（原本ZIP）, originals/models/epsilon/（展開済み原本）, selected/epsilon/（テスト用コピー、epsilon_pro・epsilon_free両方）
- 検証結果: 両バリアントとも.model3.json読み込み可、.moc3/テクスチャ3枚/物理演算/表示補助/表情8種/モーション多数、参照整合性OK（reports/FILE_VERIFICATION.md）

## 6. Cubism SDK for Web（取得済み）

- 取得物: Cubism SDK for Web 一式（Samples / Framework / Core / ライセンス / NOTICE）
- 公式URL: https://www.live2d.com/sdk/download/web/
- 取得日: 2026-07-11（ユーザーが利用許諾へ同意の上ダウンロード。自動同意はしていない）
- バージョンまたはタグ: 5-r.5（安定版。CubismWebSamplesリポジトリと同一バージョンで整合）
- ライセンスページ:
  - Live2D Proprietary Software 使用許諾契約書
  - Live2D Open Software 使用許諾契約書
- 使用目的: Cubism Core for Web（GitHub非公開の プロプライエタリファイル）の取得
- 保存先: project-input/live2d/downloads/sdk/CubismSdkForWeb-5-r.5.zip（原本ZIP）, originals/web-samples/CubismSdkForWeb-5-r.5/（展開済み原本）
- Core配置: コピー元 project-input/live2d/originals/web-samples/CubismSdkForWeb-5-r.5/Core/ → コピー先 third_party/live2d/CubismWebSamples/Core/ （live2dcubismcore.js / .min.js / .d.ts / .js.map / README / CHANGELOG / LICENSE / RedistributableFiles.txt の9ファイル）
- ライセンス保存: CubismSdkForWeb-LICENSE.md, CubismSdkForWeb-NOTICE.md, CubismSdkForWeb-NOTICE.ja.md, CubismCore-LICENSE.md, CubismCore-README.md, CubismCore-RedistributableFiles.txt を licenses/ へ保存済み
- 備考: third_party/live2d/CubismWebSamples/Core/ は.gitignoreの `**/Live2DCubismCore.*` 等でカバーされないファイル名（`live2dcubismcore.js`等、小文字）のため、`/third_party/live2d/` 全体除外ルールで保護している

## 7. MotionSync公式パッケージ for Web（取得済み）

- 取得物: Cubism SDK MotionSync Plugin for Web 一式（Samples / Framework / Core(CRI) / TypeScript Demo / ライセンス / NOTICE）
- 公式URL: https://www.live2d.com/en/sdk/download/motionsync/
- 取得日: 2026-07-11（ユーザーが利用許諾へ同意の上、正しい「for Web」版を再ダウンロード。自動同意はしていない）
- バージョンまたはタグ: 5-r.2（for Web）
- ライセンスページ:
  - Live2D Proprietary Software License Agreement
  - Live2D Open Software License Agreement
- 使用目的: MotionSync Core、Framework、TypeScript Demo、サンプル音声の取得（高度なリップシンクのテスト）
- 保存先: project-input/live2d/downloads/motionsync/CubismSdkMotionSyncPluginForWeb-5-r.2.zip（原本ZIP）, originals/motionsync/CubismSdkMotionSyncPluginForWeb-5-r.2/（展開済み原本）
- Core配置: コピー元 project-input/live2d/originals/motionsync/CubismSdkMotionSyncPluginForWeb-5-r.2/Core/ → コピー先 third_party/live2d/CubismWebMotionSyncComponents/Core/ （CRI/live2dcubismmotionsynccore.js 等、README/CHANGELOG/LICENSE含む11ファイル）
- ライセンス保存: CubismSdkMotionSyncPluginForWeb-LICENSE.md, -NOTICE.md, -NOTICE.ja.md, CubismMotionSyncCore-LICENSE.md, CubismMotionSyncCore-README.md, CubismMotionSyncCore-CRI-LICENSE.md, CubismMotionSyncCore-CRI-RedistributableFiles.txt を licenses/ へ保存済み
- 備考: 旧「for Native」版（`CubismSdkMotionSyncPluginForNative-5-r.2.zip`、SHA-256: 59d40e0664a092011b1305f393ad547435f8accd3f12ca0d27e5e842aa6fd5d4）はユーザーのDownloadsフォルダに残っているが、対象違いのためproject-input配下へは配置していない。

## 8. Haru（受付バージョン）PRO版（取得済み・Phase 3外の追加取り込み）

- 取得物: `haru_greeter_t05` モデル一式（PSD素材含む）
- 公式URL: https://www.live2d.com/en/learn/sample/haru-receptionist/ （推定。サンプル一覧の "Haru (receptionist version)" に対応）
- 取得日: 2026-07-11（ユーザーが利用許諾へ同意の上ダウンロード。自動同意はしていない）
- バージョンまたはタグ: haru_greeter_ja.zip（SHA-256: 3db5c9180fc8446f7d92eee13ea33f584e5d1308e101d7afa4fe3b037a0ed94e）
- ライセンスページ: Free Material License Agreement（無償提供マテリアルの使用許諾契約書、ライセンスタイプ「Live2Dオリジナルキャラクター」）+ https://www.live2d.com/download/sample-data/
- 個別条件: ReadMe.txtに個別の禁止事項の記載はなし（標準のLive2Dオリジナルキャラクター条件のみ）。用途は「デジタルサイネージ、受付・ガイド等」と例示。
- 使用目的: docs/research/サンプルデータ.md Phase 3では要求されていないが、ユーザー承認により追加取り込み（2026-07-11）。GitHub版Haruとは別バリエーションとしてUI/シナリオ検証等に利用可能。
- 保存先: project-input/live2d/downloads/models/haru-receptionist/haru_greeter_ja.zip（原本ZIP）, originals/models/haru-receptionist/（展開済み原本）, selected/haru-receptionist/（テスト用コピー）
- 検証結果: haru_greeter_t05.model3.json読み込み可、.moc3/テクスチャ2枚/物理演算/ポーズ/表示補助/モーション27種、参照整合性OK（reports/FILE_VERIFICATION.md）

---

## 著作権表示の候補（アプリへ未挿入、ライセンス画面組み込み候補として保管）

```
This content uses sample data owned and copyrighted by Live2D Inc.
The sample data are utilized in accordance with terms and conditions set by Live2D Inc.
This content itself is created at the author's sole discretion.
```
