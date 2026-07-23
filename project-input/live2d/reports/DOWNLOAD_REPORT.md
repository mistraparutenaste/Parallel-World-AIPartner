# Live2D Asset Download Report

生成日: 2026-07-11（最終更新: MotionSync for Web / Epsilon / Simple model / Haru受付版 取得反映）

## 完了

| 取得物 | バージョン/タグ | 保存先 | SHA-256検証 | ファイル検証結果 |
|---|---|---|---|---|
| CubismWebSamples (Haru/Hiyori/Mao/Mark/Ren/Rice/Wanko) | 5-r.5 (commit ed1e0b7) | third_party/live2d/CubismWebSamples/ (原本), selected/{model}/ | 実施済み | 全モデル参照整合性OK（reports/FILE_VERIFICATION.md） |
| CubismWebFramework (サブモジュール) | commit d4da0aa | third_party/live2d/CubismWebSamples/Framework/ | — | — |
| CubismWebMotionSyncComponents | 5-r.2 (commit b16600a) | third_party/live2d/CubismWebMotionSyncComponents/ | — | — |
| Cubism SDK for Web（Core含む） | 5-r.5 | downloads/sdk/, originals/web-samples/, Core→third_party/live2d/CubismWebSamples/Core/ | 実施済み | Core 9ファイル配置確認 |
| **Kei**（kei_vowels_pro / kei_basic_free） | kei_ja.zip | downloads/models/kei/, originals/models/kei/, selected/kei/ | 実施済み | 両バリアントとも.motionsync3.json・4言語WAV含め参照整合性OK |
| **MotionSync Plugin for Web**（Core(CRI)含む） | 5-r.2（for Web） | downloads/motionsync/, originals/motionsync/, Core→third_party/live2d/CubismWebMotionSyncComponents/Core/ | 実施済み | Core(CRI) 11ファイル配置確認 |
| **Epsilon**（epsilon_pro / epsilon_free） | epsilon_ja.zip | downloads/models/epsilon/, originals/models/epsilon/, selected/epsilon/ | 実施済み | 両バリアントとも表情8種・多数モーション含め参照整合性OK |
| **Simple model** | simple_ja.zip | downloads/models/simple-model/, originals/models/simple-model/, selected/simple-model/ | 実施済み | .model3.json/.moc3/テクスチャ/モーション参照整合性OK |
| **Haru（受付バージョン）PRO版**（追加取り込み、ユーザー承認済み） | haru_greeter_ja.zip | downloads/models/haru-receptionist/, originals/models/haru-receptionist/, selected/haru-receptionist/ | 実施済み | モーション27種含め参照整合性OK |
| ライセンス/NOTICE/README/CHANGELOG一式 | — | project-input/live2d/licenses/（25ファイル） | 実施済み（manifests/licenses_sha256.txt） | — |
| manifests/assets.json | — | project-input/live2d/manifests/assets.json | 19アセットのSHA-256を記録 | — |

**最小テストセット（Mark/Haru/Mao/Ren/Kei）・補助セット（Hiyori/Rice/Wanko/Simple model/Epsilon）ともに全て取得完了しました。**

不一致だった2件も解消済みです。
- MotionSync: ユーザーが正しい「for Web」版を再ダウンロードし、格納済み。誤っていた「for Native」版（`CubismSdkMotionSyncPluginForNative-5-r.2.zip`）はDownloadsフォルダに残っていますが、project-input配下へは配置していません。
- Haru受付版: ユーザーが取り込みを承認したため、`project-input/live2d/selected/haru-receptionist/` として追加しました（Phase 3の指定外の追加アセットとして manifests/SOURCE_URLS.md に明記）。

## ユーザー確認待ち

現時点でなし。すべてのPhase 3-6対象および追加承認分の取得・格納・検証が完了しています。

## 未完了

- Phase 4の追加候補モデル（Chitose等9件）は方針通り未取得（reports/OPTIONAL_MODELS.md参照、いずれも現行検証範囲では非推奨判定のため）。
- それ以外の未完了項目はありません。

## ライセンス上の注意

- **Live2D Original Characters区分**（Haru/Hiyori/Mao/Mark/Ren/Rice/Wanko/Kei/Epsilon/Simple model/Haru受付版）: 著作権表示 "This content uses sample data owned and copyrighted by Live2D Inc." が必要。個別制限あり（例: Shizukuは改名不可、Mark-kunは「イケメン化」改変不可 — いずれも今回未取得）。Kei/Epsilonは「一般ユーザー・小規模事業者（年商1000万円未満）は規約同意により商用利用可、中・大規模事業者は非公開テスト用途のみ」という利用条件。
- **Collaboration Characters区分**（Natori、Tsumiki Harugasa等）: 商用利用・改変・再配布を禁止。今回、Natoriはリポジトリ内に存在するが選択セットへコピーしていません。
- **外部ライセンスキャラクター区分**（Hatsune Miku、Unity-chan等）: 第三者ライセンス（Crypton Future Media、Unity Technologies Japan）への準拠が別途必要。今回は使用対象外。
- Cubism Core（live2dcubismcore.js等）およびMotionSync Core（live2dcubismmotionsynccore.js等）はいずれもプロプライエタリソフトウェア。third_party/live2d/ 全体を.gitignoreで除外済み。
- MotionSync Core配下の `CRIWARELOGO_1.png` および CRI社関連のライセンス表記あり（Core/CRI/LICENSE.md, RedistributableFiles.txt保存済み）。CRI・ミドルウェア社との第三者ライセンスが絡む可能性があるため、実際にMotionSync機能を製品へ組み込む際は追加確認を推奨。
- `redistributionApproved` は manifests/assets.json 内すべて `false`（法的確認未了のため）。
- Haru受付版PRO（haru_greeter_t05）は「デジタルサイネージ・受付・ガイド」用途として案内されており、標準のLive2Dオリジナルキャラクター条件が適用される（個別追加制限の記載なし、ReadMe.txt確認済み）。
- ライセンス文書自体の再配布可否は個別に未確認（各配布物に同梱されているLICENSE.md/NOTICE.mdのコピー保管は許容されると考えられるが、公開リポジトリへの再配布可否は別途確認を推奨）。

## 推奨テストモデル

- 初期描画: Mark（最小モデル構造）、Simple model（さらに最小）
- 表情: Mao（表情・Blend Shape）、Ren（高度な表情）、Epsilon（表情8種・感情マッピング・表情フェード）
- リップシンク: Haru（WAV音量ベース）
- MotionSync: Kei（kei_vowels_pro / kei_basic_free、.motionsync3.json・4言語WAV検証済み）、MotionSync Plugin for Web（Core含め配置済み）
- 高度描画: Ren（Cubism 5.3・Offscreen描画・半透明・Blend Mode）、Rice（Reversed Mask・Extended Interpolation）
- 追加検証用: Haru（受付バージョン）PRO版（モーション27種、UI/シナリオ検証向け）

**最小セット・補助セットともに全モデル取得完了。**

## 除外した素材

| 素材名 | 除外理由 |
|---|---|
| Natori | コラボレーションキャラクター。商用利用・改変・再配布に制限があるため通常のテスト用セットから除外（リポジトリ内には存在） |
| Hatsune Miku | 外部ライセンスキャラクター。Crypton Future Media社の個別ガイドライン準拠が必要で、汎用テスト素材として扱うと管理が複雑になるため |
| Unity-chan | 外部ライセンスキャラクター。Unity Technologies Japan社の個別ライセンス準拠が必要なため |
| Chitose / Koharu&Haruto / Tororo&Hijiki / Gantzert&Felixander / Izumi / Miara / Nito / Hibiki / Shizuku | Phase 4候補調査の結果、現行の検証範囲（最小セット・補助セット）に含まれないため自動ダウンロード対象外と判定。詳細は reports/OPTIONAL_MODELS.md |
| CubismSdkMotionSyncPluginForNative-5-r.2.zip | docs/research/サンプルデータ.mdが要求する「for Web」ではなく「for Native」のため未配置のまま。正しい「for Web」版で代替済み |

---

## 検証プロセスについての補足

ダウンロード完了の報告に留まらず、以下を実施済みです。

1. 実ファイル存在確認: project-input/live2d/selected/ 配下全ファイルの存在をfind/lsコマンドで確認
2. 参照整合性確認: 全11アセット（Haru/Hiyori/Mao/Mark/Ren/Rice/Wanko/Kei×2/Epsilon×2/Simple model/Haru受付版）で.model3.jsonのJSON妥当性、.moc3/テクスチャ/物理演算/ポーズ/表情/モーション/音声/MotionSyncファイルの参照先実在、大文字小文字一致、パストラバーサルなしを検証（reports/FILE_VERIFICATION.md、NG件数0）
3. SHA-256記録: 選択済みモデル・ダウンロードZIP原本・third_partyへ配置したCore一式を含む19アセット分をmanifests/assets.jsonへ記録、ライセンス文書25ファイル分をmanifests/licenses_sha256.txtへ記録
4. ライセンス保存状況確認: project-input/live2d/licenses/ にライセンス/NOTICE/README/CHANGELOG計25ファイルが存在することを確認
5. Downloadsフォルダの全ファイルを棚卸しし、要求と一致しないもの（旧Native版MotionSync）は無断で流用せず、ユーザー確認・再ダウンロードを経て解消
