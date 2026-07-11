# Phase 1 Live2D表示 — 実装前調査・計画案

調査日: 2026-07-11  
対象: `.worktrees/phase-0-foundation`（コード・設計）と、親worktreeのgitignore対象Live2D実資産  
この文書の役割: Phase 1の実装前に、現物・一次情報・外部ゲートを照合し、TDDで実行可能な作業単位と証拠を固定する。実装そのものは含まない。

## 1. Phase 1の権威ある要求

### `基本設計.md` からの要求

- Cubism SDK for Webを使用する。
- TypeScript側でモデル読込、表情、モーション、視線、待機アニメーションを扱う。リップシンクはPhase 4で完成させるため、Phase 1では口制御を実装範囲へ混ぜない。
- Character Windowは、透過Live2D表示、表情・モーション、ドラッグ、クリック判定を担当する。
- ReactからSDKを直接操作せず、`CharacterController -> packages/live2d-runtime -> Cubism SDK` の依存方向を守る。
- Phase 1実装内容はSDK組込、透過ウィンドウ、モデル読込、待機モーション、表情変更、ドラッグ移動、クリック透過、DPI対応。
- 完了条件は、安定したデスクトップ表示、設定変更による表情・モーション切替、再起動後の位置・サイズ復元。

### 製品完成設計からの追加制約

- Character Windowに設定ファイル変更や外部プロセス起動権限を与えない。
- Live2D障害時も通常ウィンドウで会話と状態を表示できる縮退経路を保つ。
- TypeScriptではcomponent、Live2D controller、IPC契約、window別テストを分離する。
- 許諾未確認モデルは配布buildへ含めない。外部ゲートが未提供でも、ローカル開発buildとmock artifactによる検証までは完成させる。

## 2. 現行コードの棚卸し

### 既にある基盤

| 領域 | 現状 | Phase 1で使う境界 |
|---|---|---|
| frontend | React 19、Vite 8、TypeScript、Vitest、3 HTML entry | `character-entry.tsx`からCharacter featureをmountする |
| Character UI | `CharacterWindow.tsx`はSVG silhouette、status、drag regionのみ | canvas host、load/error status、操作可能領域へ置換する |
| Tauri | Tauri 2、Rust生成の3 WebviewWindow | `character`だけ透明・枠なしという既存定義を拡張する |
| capability | Characterは`start-dragging`と`show`だけ | 原則維持。入力透過をRust commandに閉じ込め、広いwindow権限を付与しない |
| data path | `pw-platform::AppDataLayout`と日次log | window-state pluginの保存先と競合しないことを確認する |
| CSP | local scriptのみ、`img-src`にself/asset | bundled local modelなら現状に近いCSPで足りる。CDNは追加しない |
| contracts | Rustからts-rs生成する型パッケージ | Character設定commandを追加する場合も生成型を正本にする |

### 欠けているもの

- `packages/live2d-runtime`自体が未作成。
- Cubism CoreをHTMLより先に読み込む処理、Framework build/import、WebGL renderer、model lifecycleがない。
- `content/characters/default/character.json`と、そのschema/validatorがない。
- Settingsから表情・モーションを選ぶUIと、window間の状態伝播がない。
- Character windowの位置・サイズ復元、DPI変化へのcanvas backing-store追従がない。
- クリック透過の切替commandと、Settings側から復帰できる操作経路がない。
- 実モデルを使う統合テストと、SDK/モデルなしでも走るCIテストの分離がない。

### 先に認識すべきbaseline defect

現行の複数TSX/Rustファイルには日本語文字列のmojibakeが残っている。Phase 1の新規テストが壊れた表示文字列を正として固定しないよう、Phase 1最初の変更で対象Character文字列だけをUTF-8へ正常化し、既存UI全体の一括修正は別タスクにしない。無関係なChat/Settings文言まで広げる場合は独立レビュー単位にする。

## 3. Live2D実資産の現物確認

### worktree間の重要な差

- feature worktreeには、`project-input/live2d/licenses/`、`manifests/`、`reports/`、`SOURCE_URLS.md`だけが見える。
- 実際のSDK・Core・モデルは親worktreeの以下に存在し、`.gitignore`対象である。
  - `third_party/live2d/`: 475 files / 約84.4 MB
  - `project-input/live2d/selected/`: 285 files / 約153.5 MB
  - `project-input/live2d/originals/`: 531 files / 約188.5 MB
  - `project-input/live2d/downloads/`: 6 files / 約122.6 MB
- よって実装は、絶対パスや「親worktreeにたまたま存在する」状態へ依存してはならない。明示的なdev asset staging commandで、検証済み入力からworktree内のignored staging directoryへコピーする。

### SDK

- Cubism SDK for Web `5-r.5`、CubismWebSamples commit `ed1e0b7`、Framework commit `d4da0aa`を取得済み。
- Coreには `live2dcubismcore.js` / `.min.js` / `.d.ts` があり、`CubismCore-RedistributableFiles.txt`はこの3ファイルをLive2D Proprietary Software License Agreementの条件下でredistributableと列挙している。
- FrameworkはTypeScript sourceとして存在し、公式packageは`tsc`で`dist`を生成する構造。
- MotionSync `5-r.2`も取得済みだが、Phase 1へは入れない。Phase 4のリップシンク責務である。

### モデル

- manifestは19 assetsを記録し、Haru/Hiyori/Mao/Mark/Ren/Rice/Wanko/Kei/Epsilon/Simple/Haru Receptionist等を検証済みとしている。
- Phase 1の最小受け入れモデルはMark、補助スモークはSimple modelとする。
  - Mark: 最小構造、1 texture、physics、6 idle motions。表情・hit areaはないため描画/idle/DPIに向く。
  - Simple model: 最小障害切分け用。
- 表情切替の実モデル受け入れにはEpsilonを使う（8 expressions）。Markだけでは「設定変更で表情を切替」の証明にならない。
- manifest上の全assetが`redistributionApproved: false`。したがって、ローカル開発・統合テストには使えても、公開buildへの自動同梱をPhase 1完了条件にしてはいけない。

### integrityと再現性

- `assets.json`に各ファイル/ZIP/CoreのSHA-256が記録済み。
- staging commandはコピー前後にSHA-256を照合し、不一致・欠落・path traversal・大文字小文字不一致で非0終了する。
- `FILE_VERIFICATION.md`は参照整合性NG 0を報告しているが、生成元scriptがrepoにない場合は、その報告だけを将来の自動テスト証拠にはしない。Phase 1でvalidatorをコード化する。

## 4. 2026-07-11時点の公式一次情報

### Cubism SDK for Web

- Live2D公式最新stableはCubism 5 SDK for Web R5、tag `5-r.5`（2026-04-02）。ローカル取得物と一致する。  
  https://github.com/Live2D/CubismWebSamples/releases/tag/5-r.5
- FrameworkはCoreと組み合わせて使用し、model表示・操作を提供する。CoreはFramework GitHub repoには含まれず、公式SDK packageから取得する。  
  https://github.com/Live2D/CubismWebFramework
- `.model3.json`を`CubismModelSettingJson`で解析し、相対参照されたmoc3、texture、expression、physics、pose、motion等をロードするのが公式経路。個別ファイルの直指定を独自実装しない。  
  https://docs.live2d.com/en/cubism-sdk-manual/use-framework-web/
- R5正式版ではmodel parameter update orderがFramework側`CubismUpdateScheduler`へ移り、trackingは`CubismLook`へ変わった。古いSampleのupdate順をコピーせず、同梱`5-r.5` Sampleを基準にする。  
  https://docs.live2d.com/cubism-sdk-manual/compatibility-with-cubism-5-3-official/

### Tauri 2

- Capabilityはwindow labelごとに最小権限を付与する。複数Capabilityに同じwindowを含めると権限がmergeされるため、Character専用Capabilityを維持する。  
  https://v2.tauri.app/security/capabilities/
- CSPはremote CDN scriptを避け、bundled local資産へ限定する。Tauriはbuild時にlocal scriptへnonce/hashを補う。  
  https://v2.tauri.app/security/csp/
- 位置・サイズ復元は公式`window-state` pluginで行える。復元はwindow生成後なので、初期`visible: false`にし、復元後にshowすることでflashを避ける。  
  https://v2.tauri.app/plugin/window-state/
- DPI変化は`tauri://scale-change`、resizeは`tauri://resize`で観測できる。CSS sizeとcanvas physical backing sizeを分離する。  
  https://v2.tauri.app/reference/javascript/api/namespaceevent/
- `set_ignore_cursor_events(true)`は透明画素だけではなくwindow全体のcursor eventを背後へ通す。Tauri/taoにalpha-aware hit test APIはない。  
  https://docs.rs/tauri/latest/tauri/window/struct.Window.html#method.set_ignore_cursor_events

## 5. 採用アーキテクチャ

```text
SettingsWindow
  -> typed command/event: CharacterPresentationSettings
  -> Rust CharacterPresentationState (single source of truth)
  -> event to CharacterWindow
  -> React CharacterFeature (mount/unmount/status only)
  -> CharacterController (React independent)
  -> CubismRuntime adapter
  -> Cubism Framework 5-r.5
  -> Cubism Core 5-r.5 + staged local model
```

### 境界

1. `packages/live2d-runtime`はDOM canvasとfetch可能URLを受けるが、React/Tauriをimportしない。
2. Cubism固有classは`CubismRuntimeAdapter`内へ閉じ込める。controller単体テストはfake adapterで行う。
3. Character featureはcontroller factoryを注入でき、jsdomでWebGLなしにlifecycleを検証できる。
4. 実SDK/WebGL/modelはlocal integration testでのみ使う。CIの通常testはproprietary file不在でもgreenであること。
5. model選択・expression・motion・click-throughはRust側のversioned stateを正本にし、SettingsとCharacterのwindow間でtyped eventを使う。

### クリック透過の正確な仕様

- 「クリック透過」は`interactive` / `clickThrough`の明示的なwindow modeとする。
- `clickThrough`ではwindow全体が入力を無視するため、Character内からは解除できない。Settings Windowに常時解除操作を置く。
- `interactive`ではモデル/drag handleを操作できる。ドラッグ中にclick-throughへ遷移させない。
- 透明画素だけをOS hit-testで透過する機能はTauri標準APIで証明できないため、Phase 1の完了条件として偽って扱わない。将来native platform extensionを追加するなら別ADR/Phaseとする。

### 資産の扱い

- dev staging先案: `apps/desktop/public/static/live2d-dev/`（全体をignore）。
- Core script、Framework build output、選択モデルを`tools/live2d/stage-dev-assets.ps1`で配置する。
- Viteはlocal self URLからのみ配信し、remote/CDNを許可しない。
- release buildは`redistributionApproved`がtrueになったassetだけを別manifest経由でbundleする。Phase 1時点はfalseなので、bundle時にモデルを含めず明示的な「モデル未導入」縮退表示を出す。

## 6. Phase 1 TDDタスク分解

以下の各Taskは独立review/commit単位とし、Red -> Green -> Refactor -> full gateを守る。

### Task 1: 再現可能なdev asset stagingとmanifest validation

**Files**

- Create: `tools/live2d/stage-dev-assets.ps1`
- Create: `tools/live2d/Test-Live2DManifest.ps1`
- Create: `tools/live2d/tests/Live2DAssets.Tests.ps1`
- Modify: `.gitignore`
- Modify: `package.json`
- Test input: `project-input/live2d/manifests/assets.json`

**Interfaces**

- Produces command: `pnpm live2d:stage -- -SourceRoot <path> -Model live2d-mark`
- Produces ignored layout: `apps/desktop/public/static/live2d-dev/{core,framework,models/<id>}`
- Failure contract: missing source/hash mismatch/undeclared file/path escapeはexit code 1、成功は0。

**TDD証拠**

1. temp fixtureでmissing Core、hash mismatch、`../` pathを個別にRed確認。
2. valid synthetic fixtureが正しいrelative layoutへcopyされることをGreen確認。
3. 親worktreeの現物をsourceにMarkをstageし、manifest記録hashと全コピー先hash一致を記録。
4. `git status --ignored`でstaged proprietary/model fileが追跡候補に出ないことを確認。

### Task 2: React非依存Live2D runtime public API

**Files**

- Create: `packages/live2d-runtime/package.json`
- Create: `packages/live2d-runtime/tsconfig.json`
- Create: `packages/live2d-runtime/src/contracts.ts`
- Create: `packages/live2d-runtime/src/controller/CharacterController.ts`
- Create: `packages/live2d-runtime/src/controller/CharacterController.test.ts`
- Create: `packages/live2d-runtime/src/manifest/CharacterManifest.ts`
- Create: `packages/live2d-runtime/src/manifest/CharacterManifest.test.ts`
- Create: `packages/live2d-runtime/src/index.ts`

**Interfaces**

```ts
export type CharacterRuntimeStatus =
  | { kind: 'idle' }
  | { kind: 'loading'; modelId: string }
  | { kind: 'ready'; modelId: string }
  | { kind: 'failed'; modelId: string; code: Live2DErrorCode };

export interface CharacterController {
  mount(canvas: HTMLCanvasElement): Promise<void>;
  loadModel(model: CharacterModelSource): Promise<void>;
  playMotion(group: string, index?: number): Promise<void>;
  setExpression(id: string): Promise<void>;
  resize(viewport: CharacterViewport): void;
  dispose(): void;
  subscribe(listener: (status: CharacterRuntimeStatus) => void): () => void;
}

export interface CharacterViewport {
  cssWidth: number;
  cssHeight: number;
  devicePixelRatio: number;
}
```

**TDD証拠**

- load前のmotion/expressionはtyped `not-ready` error。
- model切替は旧renderer/resourcesをdisposeしてから新modelをload。
- repeated mount/disposeがRAF/listener/WebGL resourceをleakしない。
- invalid manifest、unknown expression/motion、fetch failureがstable error codeへ変換される。
- `rg "from ['\"]react|@tauri-apps" packages/live2d-runtime`が0件。

### Task 3: Cubism 5-r.5 adapterと実モデル描画

**Files**

- Create: `packages/live2d-runtime/src/cubism/CubismRuntimeAdapter.ts`
- Create: `packages/live2d-runtime/src/cubism/CubismRuntimeAdapter.test.ts`
- Create: `packages/live2d-runtime/src/renderer/RenderLoop.ts`
- Create: `packages/live2d-runtime/src/renderer/RenderLoop.test.ts`
- Create: `packages/live2d-runtime/tests/integration/mark-render.test.ts`
- Modify: `apps/desktop/character.html`（Core scriptをmodule entryより先にlocal load）
- Modify: `apps/desktop/vite.config.ts`

**Interfaces**

- Consumes staged `live2dcubismcore.min.js` and Framework 5-r.5 output.
- Adapter method: `createModel(canvas, model3JsonUrl): Promise<CubismModelHandle>`。
- Handle: `update(deltaSeconds)`, `draw()`, `playMotion(group,index)`, `setExpression(id)`, `resize(physicalWidth,physicalHeight)`, `dispose()`。

**TDD証拠**

- fake Core global不在で`core-unavailable`、WebGL context不在で`webgl-unavailable`。
- `.model3.json`を入口に相対参照を解決し、個別moc pathをhard-codeしない。
- R5のFramework update scheduler順序を使用する。
- 実Markでcanvasにnon-transparent pixelsが描かれるscreenshot/pixel probe、idle motion時間経過でframe差分が生じる。
- Core/Framework/model fetchがすべて`self` URLで、browser console error 0。

### Task 4: Character React featureと縮退表示

**Files**

- Create: `apps/desktop/src/features/character/CharacterCanvas.tsx`
- Create: `apps/desktop/src/features/character/useCharacterController.ts`
- Create: `apps/desktop/src/features/character/CharacterCanvas.test.tsx`
- Modify: `apps/desktop/src/windows/character/CharacterWindow.tsx`
- Modify: `apps/desktop/src/shared/styles/global.css`（またはCharacter専用CSSを新設）
- Modify: `apps/desktop/src/windows/windows.test.tsx`

**Interfaces**

- UI propsは`modelSource`, `presentation`, `controllerFactory`。
- status表示は`loading/ready/failed`を日本語UIへ変換するが、runtime error codeは保持する。
- unmount時にcontrollerを必ずdisposeする。

**TDD証拠**

- React StrictMode相当のmount/unmount/remountでcontroller instance leakなし。
- load中、ready、failedの各accessible status。
- SDK/modelなしのrelease相当環境でもSVG silhouetteまたは明示placeholderへ縮退し、window自体はcrashしない。
- desktop幅・高DPI・最小window sizeでcanvasがoverflowしない。

### Task 5: typed Character presentation stateとSettings制御

**Files**

- Create/Modify: `crates/pw-contracts/src/dto/character_presentation.rs`
- Create: `apps/desktop/src-tauri/src/commands/character_presentation.rs`
- Modify: `apps/desktop/src-tauri/src/commands/mod.rs`
- Modify: `apps/desktop/src-tauri/src/lib.rs`
- Generate: `packages/contracts/src/generated/*`
- Create: `apps/desktop/src/features/character/characterPresentationClient.ts`
- Modify: `apps/desktop/src/windows/settings/SettingsWindow.tsx`
- Modify: `apps/desktop/src/windows/character/CharacterWindow.tsx`
- Modify: `apps/desktop/src-tauri/permissions/autogenerated/*`
- Modify: `apps/desktop/src-tauri/capabilities/{character,settings}.json`

**Interfaces**

```rust
pub struct CharacterPresentationSettingsDto {
    pub schema_version: u32,
    pub model_id: String,
    pub expression_id: Option<String>,
    pub motion_group: String,
    pub motion_index: Option<u32>,
    pub click_through: bool,
}
```

- Commands: `get_character_presentation`, `set_character_presentation`。
- Event: `character-presentation://changed` with the same generated DTO。
- Settingsのみwrite可能、Characterはread/listenのみ。

**TDD証拠**

- invalid model/expression/motionをRust側でrejectし、frontend入力だけに依存しない。
- Settings change -> Rust state -> Character event -> controller callをcontract/component testで証明。
- Character capabilityからwrite command、filesystem、shell/process権限が利用不能。
- expressionはEpsilon、motionはMark/Epsilonの実modelで切替をvisual diffとconsole log 0で証明。

### Task 6: Tauri Character window behavior、DPI、永続化

**Files**

- Modify: `apps/desktop/src-tauri/Cargo.toml`
- Modify: root `Cargo.toml` / `Cargo.lock`
- Modify: `apps/desktop/src-tauri/src/lib.rs`
- Modify: `apps/desktop/src-tauri/src/windows/definitions.rs`
- Modify: `apps/desktop/src-tauri/src/windows/mod.rs`
- Create: `apps/desktop/src-tauri/src/windows/character_behavior.rs`
- Create: `apps/desktop/src-tauri/tests/character_window.rs`
- Modify: `apps/desktop/src-tauri/capabilities/character.json`
- Modify: `apps/desktop/src-tauri/capabilities/settings.json`

**Interfaces**

- Register `tauri-plugin-window-state`。
- Save/restore flagsはCharacterのposition/sizeを含めるが、復元不能なoff-screen位置は現在monitor内へclampする。
- Rust command internally calls `set_ignore_cursor_events(click_through)`; frontendへ一般的なwindow mutation権限を渡さない。
- window初期`visible: false`、restore/clamp後show。

**TDD証拠**

- window definition remains transparent/decorations false and has explicit initial logical size/min size。
- interactive modeでdrag、clickThrough modeで背後windowがclickを受けるmanual/E2E証拠。
- SettingsからclickThroughをfalseへ戻せる。
- 100%/150%/200% scaleでCSS boundsとphysical canvas sizeが`logical * scaleFactor`になり、ぼけ/切れをscreenshot比較。
- app再起動後にposition/sizeが許容誤差内で復元される。monitor構成変更時は画面外に残らない。

### Task 7: Phase 1受け入れgateと運用文書

**Files**

- Create: `scripts/run-phase1.ps1`
- Create: `docs/development/live2d-assets.md`
- Modify: `README.md`
- Modify: `作業内容.md`
- Modify: CI workflow（通常testではproprietary assetsを要求しない）

**Verification matrix**

| Gate | SDK/modelなしCI | ローカル実資産 | Release許諾後 |
|---|---:|---:|---:|
| unit/component/contract | 必須 | 必須 | 必須 |
| manifest/staging fixture | 必須 | 必須 | 必須 |
| real Core+Mark render | skip理由を機械判定 | 必須 | 必須 |
| expression Epsilon | skip理由を機械判定 | 必須 | 必須 |
| Tauri transparent/DPI/window state | platform test可能範囲 | 必須 | 必須 |
| bundled model/core audit | asset 0を期待 | dev staging除外を期待 | approved manifestだけ期待 |

**最終コマンド案**

```powershell
corepack pnpm test
corepack pnpm typecheck
corepack pnpm build
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features
pwsh ./scripts/run-phase1.ps1 -Live2DSourceRoot <parent-worktree-path>
```

`run-phase1.ps1`は個別gate結果、test件数、SDK/model hash、screenshot path、console error数、window復元のbefore/after logical position/sizeをmachine-readable JSONにも保存する。

## 7. 外部許諾ゲートとローカル実装の分離

### 外部確認なしで完了できる

- Framework/Core adapter、controller、React integration、typed state、Tauri window behavior。
- 取得済みSDK/modelを使うローカル開発・統合テスト。
- manifest/hash validation、staging、CI fake adapter、no-asset縮退build。
- license/NOTICEの保存と、bundleに未承認assetがないことの自動監査。

### 外部確認が必要

- 公開・配布buildへCubism Coreを含める法的判断。RedistributableFiles記載だけで事業者区分やRelease License条件の確認を省略しない。
- 公開・配布buildへLive2D sample characterを同梱する許諾。現manifestは全件`redistributionApproved: false`。
- 年商1000万円以上の事業者に該当する場合のCubism SDK Release License。
- 必要な著作権表示と、Free Material / Sample Data Termsの適用確認。

### gateの実装ルール

- `ALLOW_UNAPPROVED_LIVE2D=1`のような逃げ道を作らない。
- release bundleはreview済みmanifestの`redistributionApproved: true`と、license metadata/notice/hashの全条件が揃わない限りfail closed。
- dev stagingは明示commandかつignored pathのみ。通常の`pnpm build`が親worktreeを探索・コピーしてはいけない。
- SDK/modelなしのbuild成功は「Live2D表示完了」の証拠ではない。実資産acceptance gateの成功を別に必須記録する。

## 8. Phase 1完了判定

次をすべて満たした場合だけPhase 1完了とする。

1. MarkまたはSimpleを、取得済みCore/Framework `5-r.5`でTauri Character Windowへ描画し、console error 0、連続30分でcrash/resource増大なし。
2. idle motionが実時間で継続し、Epsilonの少なくとも2表情と1 motionをSettings変更から切替できる。
3. window背景が透過し、interactive時のdrag、Settingsから切替/復帰可能なwhole-window click-throughが動作する。
4. 100/150/200% DPIでcanvas physical sizeと表示品質を検証し、resize/scale-change後も描画が切れない。
5. 再起動後にCharacter位置・サイズが復元し、monitor構成変更時も画面内にclampされる。
6. SDK/model破損・不在・WebGL failure時にCharacterは縮退表示し、Chat/Settingsは動作継続する。
7. unit/component/contract/Cargo/Tauri/real-asset acceptanceの全gateがgreenで、証拠が`作業内容.md`とmachine-readable reportに残る。
8. release bundleに未承認Core/modelが混入していないことをarchive内容監査で証明する。

## 9. 実装順とレビュー境界

推奨順はTask 1 -> 2 -> 3 -> 4 -> 5 -> 6 -> 7。Task 2と6の一部調査は並行可能だが、同じpackage/config/Cargo.lockを同時編集しない。各Taskはfresh implementer、spec review、code-quality review、rootによる実コマンド再検証を経てから次へ進む。

