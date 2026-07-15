# Phase 7 Distribution Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Windows NSIS installerとmacOS app bundle、署名必須updater、検証付きモデル取得、第三者ライセンス画面、Windows/macOS配布CIを再現可能に実装する。

**Architecture:** PR/ローカルbuildは秘密情報なしのunsigned bundleとmock署名検証を担当し、production releaseだけをprotected environmentとfail-closed設定生成へ分離する。モデル本体はbundleへ含めず、Rustの単一installerがmanifest、artifact hash、license hash、archive path、展開後filesを検証してapp dataへatomic installする。updater minisignとOS code signing/notarizationは別の信頼境界として両方を要求する。

**Tech Stack:** Tauri 2.11、`tauri-plugin-updater` 2、Rust 1.96、Node.js 24.15.0、Corepack pnpm 11.11.0、NSIS、GitHub Actions、tauri-action、Azure Artifact Signing、Apple codesign/notarization、cargo-about、pnpm licenses。

## Global Constraints

- 既存の3 window、schema-versioned DTO、window-scoped events、Tauri Capability境界を維持する。
- Windowsの既定配布形式はNSIS current-user installとし、MSIは必須成果物にしない。
- production updaterはHTTPS endpoint、公開鍵、署名秘密鍵のいずれかが欠けた場合にbuildを失敗させ、署名検証を無効化する設定を提供しない。
- updater minisign、Windows Authenticode、macOS signing/notarizationを代替関係として扱わない。
- 未許諾のモデル、Live2Dモデル、ユーザー生成データ、秘密鍵をbundle/CI artifactへ含めない。
- VAD/STT/LLM/TTSの各モデルは、バージョン、配布形態、SPDX式、許諾根拠URL、本文hashを登録する。アプリが再配布しない外部LLM/TTSは`external_not_redistributed`と明記し、再配布対象へ暗黙に格上げしない。
- 外部ゲート未提供時もunsigned local bundle、mock updater検証、設定検証、ライセンス生成、CI定義まで完成させる。
- 全GitHub Actionsの`uses:`は40文字commit SHAに固定し、major tagやbranch名をrelease/CI workflowに残さない。
- 製品bundleは現在のplaceholder iconを使わず、`assets/branding/app-icon.png`からTauri CLIで再生成したicon setだけを使う。
- 全TaskをRED→GREEN→REFACTOR、独立レビュー、個別コミットで進める。
- commit前は各Taskの**Files**に列挙したexact pathだけを`git add -- <path...>`でstageし、directory単位の`git add`を禁止する。`git diff --cached --name-only`をTask固有allowlistとbyte-for-byte比較し、余分・不足があればcommitしない。

## Authoritative references

- [Tauri Windows installer](https://v2.tauri.app/distribute/windows-installer/)
- [Tauri Windows signing](https://v2.tauri.app/distribute/sign/windows/)
- [Tauri macOS signing](https://v2.tauri.app/distribute/sign/macos/)
- [Tauri updater](https://v2.tauri.app/ja/plugin/updater/)
- [Tauri GitHub pipeline](https://v2.tauri.app/distribute/pipelines/github/)
- [tauri-action](https://github.com/tauri-apps/tauri-action)
- [cargo-about](https://embarkstudios.github.io/cargo-about/cli/generate/config.html)
- [pnpm licenses](https://pnpm.io/cli/licenses)
- [GitHub deployment environments](https://docs.github.com/en/actions/concepts/workflows-and-actions/deployment-environments)
- [GitHub artifact attestations](https://docs.github.com/en/actions/how-tos/secure-your-work/use-artifact-attestations/use-artifact-attestations)

---

### Task 1: Fail-closed distribution config and unsigned local bundles

**Files:**
- Modify: `apps/desktop/src-tauri/tauri.conf.json`
- Create: `assets/branding/app-icon.png`
- Create: `apps/desktop/src-tauri/tauri.windows.local.json`
- Create: `apps/desktop/src-tauri/tauri.macos.local.json`
- Create: `tools/scripts/verify-distribution-config.mjs`
- Create: `tools/scripts/verify-distribution-config.test.mjs`
- Create: `tools/fixtures/generated-icon-files.txt`（`tauri icon`が生成する全exact relative pathの固定allowlist）
- Modify: `apps/desktop/src-tauri/icons/`配下のうち`tools/fixtures/generated-icon-files.txt`に1行ずつ列挙したexact filesのみ
- Modify: `package.json`
- Modify: `.gitignore`

**Interfaces:**
- Consumes: base identifier `com.parallelworld.desktop` and existing icon set。
- Produces: `deepMerge(base, overlay)`、`loadEffectiveConfig(basePath, overlayPath)`、`verifyDistributionConfig(effectiveConfig, mode, platform)`、`distribution:verify`、`bundle:windows:local`、`bundle:macos:local`。

- [ ] **Step 1: Write failing policy tests**

```js
import assert from "node:assert/strict";
import test from "node:test";
import { deepMerge, verifyDistributionConfig } from "./verify-distribution-config.mjs";

test("overlay is deep-merged before policy is evaluated", () => {
  const base = { bundle: { active: false, resources: ["models/**"] }, plugins: { updater: { dangerousInsecureTransportProtocol: true } } };
  const overlay = { bundle: { active: true, targets: ["nsis"] } };
  const effective = deepMerge(base, overlay);
  assert.throws(() => verifyDistributionConfig(effective, "local", "windows"), /model resources/);
});

test("release updater URL and key fail closed", () => {
  const config = { bundle: { active: true, targets: ["nsis"], createUpdaterArtifacts: true }, plugins: { updater: { endpoints: ["https://user:secret@example.test/latest.json"], pubkey: TEST_FIXTURE_PUBLIC_KEY, dangerousInsecureTransportProtocol: false } } };
  assert.throws(() => verifyDistributionConfig(config, "release", "windows"), /credentials|fixture/);
});
```

- [ ] **Step 2: Verify RED**

Run: `node --test tools/scripts/verify-distribution-config.test.mjs`

Expected: FAIL because the validator does not exist。

- [ ] **Step 3: Implement effective-config loading and the validator**

```js
export function deepMerge(base, overlay) {
  if (Array.isArray(overlay)) return structuredClone(overlay);
  if (overlay && typeof overlay === "object") {
    const result = structuredClone(base && typeof base === "object" ? base : {});
    for (const [key, value] of Object.entries(overlay)) result[key] = deepMerge(result[key], value);
    return result;
  }
  return structuredClone(overlay);
}

export function verifyDistributionConfig(config, mode, platform, fixturePublicKey) {
  const bundle = config.bundle ?? {};
  if (bundle.active !== true) throw new Error("bundle.active must be true");
  const targets = Array.isArray(bundle.targets) ? bundle.targets : [bundle.targets].filter(Boolean);
  if (platform === "windows" && (targets.length !== 1 || targets[0] !== "nsis")) throw new Error("Windows target must be NSIS only");
  if (platform === "macos" && !targets.includes("app")) throw new Error("macOS target must include app");
  if (/models|characters/i.test(JSON.stringify(bundle.resources ?? []))) throw new Error("model resources and character assets must not be bundled");
  if (mode === "release") {
    if (bundle.createUpdaterArtifacts !== true) throw new Error("release must create updater artifacts");
    const updater = config.plugins?.updater ?? {};
    if (!Array.isArray(updater.endpoints) || updater.endpoints.length === 0) throw new Error("non-empty HTTPS updater endpoints are required");
    for (const raw of updater.endpoints) {
      const endpoint = new URL(raw);
      if (endpoint.protocol !== "https:") throw new Error("HTTPS updater endpoint is required");
      if (endpoint.username || endpoint.password) throw new Error("updater endpoint must not contain credentials");
    }
    const rejectDangerous = (value, path = "plugins.updater") => {
      if (!value || typeof value !== "object") return;
      for (const [key, nested] of Object.entries(value)) {
        if (/^dangerous/i.test(key) && nested !== false) throw new Error(`${path}.${key} must be false`);
        rejectDangerous(nested, `${path}.${key}`);
      }
    };
    rejectDangerous(updater);
    const pubkey = String(updater.pubkey ?? "").trim();
    if (!pubkey || pubkey === fixturePublicKey.trim()) throw new Error("non-fixture updater public key is required");
  }
}
```

CLIは`node tools/scripts/verify-distribution-config.mjs --base apps/desktop/src-tauri/tauri.conf.json --overlay <overlay> --mode <local|release> --platform <windows|macos>`とし、テスト内の手作りobjectではなくTauri CLIが読むbase+overlayの実効JSONを必ず検証する。配列はoverlayで置換、objectは再帰merge、scalarはoverlay優先とする。

- [ ] **Step 4: Add deterministic local overlays**

Windows overlayは`bundle.active=true`、`targets=["nsis"]`、`createUpdaterArtifacts=false`、NSIS `installMode="currentUser"`、Japanese/English、WebView2 `downloadBootstrapper`とする。macOS overlayは`targets=["app"]`、`createUpdaterArtifacts=false`、`bundle.macOS.signingIdentity="-"`のad-hoc signとする。model/character pathはresourcesへ追加しない。`assets/branding/app-icon.png`はユーザー提供のParallel World icon PNGとし、`corepack pnpm --filter @parallel-world/desktop tauri icon ../../assets/branding/app-icon.png`で全iconを再生成する。生成前後のpath集合を`generated-icon-files.txt`のexact allowlistと比較し、追加・欠落・allowlist外変更を拒否する。verifierはplaceholder時の既知hashも拒否する。

- [ ] **Step 5: Add scripts and verify GREEN**

```json
{
  "distribution:verify": "node --test tools/scripts/verify-distribution-config.test.mjs && node tools/scripts/verify-distribution-config.mjs --base apps/desktop/src-tauri/tauri.conf.json --overlay apps/desktop/src-tauri/tauri.windows.local.json --mode local --platform windows && node tools/scripts/verify-distribution-config.mjs --base apps/desktop/src-tauri/tauri.conf.json --overlay apps/desktop/src-tauri/tauri.macos.local.json --mode local --platform macos",
  "bundle:windows:local": "corepack pnpm --filter @parallel-world/desktop tauri build --config src-tauri/tauri.windows.local.json --bundles nsis",
  "bundle:macos:local": "corepack pnpm --filter @parallel-world/desktop tauri build --config src-tauri/tauri.macos.local.json --bundles app"
}
```

Run: `corepack pnpm distribution:verify`

Expected: all policy tests PASS。Windowsでは`corepack pnpm bundle:windows:local`がnon-empty NSISを生成し、staged bundleにmodel/character bytesがない。

- [ ] **Step 6: Commit**

```powershell
# Files節の各exact file（生成iconも個別名）だけをgit add --でstageし、cached name allowlistを照合する
git commit -m "build(distribution): add fail-closed local bundle configs"
```

---

### Task 2: Mandatory signed updater with Settings-only control

**Files:**
- Modify: `Cargo.toml`, `Cargo.lock`, `apps/desktop/src-tauri/Cargo.toml`
- Modify: `apps/desktop/src-tauri/src/lib.rs`, `apps/desktop/src-tauri/src/commands/mod.rs`
- Create: `crates/pw-contracts/src/dto/update.rs`
- Modify: `crates/pw-contracts/src/dto/mod.rs`, `crates/pw-contracts/src/bin/export_bindings.rs`, `packages/contracts/src/index.ts`
- Create: `apps/desktop/src-tauri/src/updates/mod.rs`, `backend.rs`, `service.rs`
- Create: `apps/desktop/src-tauri/tests/updater_official.rs`
- Create: `apps/desktop/src-tauri/src/commands/updates.rs`
- Create: `apps/desktop/src/windows/settings/UpdatesPanel.tsx`, `UpdatesPanel.test.tsx`
- Modify: `apps/desktop/src/windows/settings/SettingsWindow.tsx`
- Modify: `apps/desktop/src-tauri/capabilities/settings.json`, `apps/desktop/src-tauri/tests/capabilities.rs`
- Create: `tools/scripts/render-release-config.mjs`, `tools/scripts/render-release-config.test.mjs`
- Create: `tools/fixtures/updater/test-public.key`, `tools/fixtures/updater/test-private.key`

**Interfaces:**
- Produces `UpdateStatusDto` and `UpdateStateDto`、window-scoped `update-progress`。
- Produces Settings-only `get_update_state`、`check_for_updates`、`install_update(approved_version)`。
- Produces ignored `apps/desktop/src-tauri/generated/release.json`; secret values are never logged。

- [ ] **Step 1: Write DTO/backend tests first**

```rust
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "UpdateStatusDto.ts")]
pub enum UpdateStatusDto { Disabled, Checking, UpToDate, Available, Downloading, Installing, RestartPending, Failed }

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[ts(export, export_to = "UpdateStateDto.ts")]
pub struct UpdateStateDto {
    pub schema_version: u16,
    pub status: UpdateStatusDto,
    pub current_version: String,
    pub available_version: Option<String>,
    pub notes: Option<String>,
    pub error: Option<String>,
}
```

Service tests prove startup checkと手動check/installがsingle-flight、未承認versionのinstallを拒否、progressはsettings windowだけへemitすることを固定する。Windowsは公式pluginがinstall中にprocess exitを所有するため、`download()`完了後に`RestartPending`をemit・永続化し、`on_before_exit` callbackでも同じflushをidempotentに1回だけ確認してから、同一objectの`install(bytes)`へ渡す。install後処理や独自restartは置かない。macOSはinstall成功後にserviceが`InstalledNeedsRelaunch`を返し、command adapterが最後に`AppHandle::restart() -> !`を呼ぶ。test fakeはOS別contract、状態順序、flushの1回性とmacOS outcomeを検証する。Official-backend integration testはfixture公開鍵で署名したmock artifactを、`UpdaterBuilder::configure_client`でfixture CAを明示的にtrustしたloopback HTTPS test serverからTauri updaterへ渡す。正常artifactはcheck済みの同一`Update` objectに保持し、公式`Update::download()`を呼んでminisign検証成功を確認する（実install/restartはしない）。one-byte tamper/wrong keyは同じ公式download検証経路でinstall前に拒否されることを固定する。error文にendpoint credentialを含めない。

- [ ] **Step 2: Verify RED**

Run: `cargo test -p parallel-world-desktop updates -- --nocapture`

Expected: FAIL because update module/commands do not exist。

- [ ] **Step 3: Implement injectable service and official production backend**

```rust
#[async_trait::async_trait]
pub trait CheckedUpdate: Send {
    fn version(&self) -> &str;
    fn notes(&self) -> Option<&str>;
    async fn download(
        &mut self,
        progress: Box<dyn Fn(u64, Option<u64>) + Send + Sync>,
    ) -> Result<Vec<u8>, UpdateError>;
    async fn install(
        self: Box<Self>,
        bytes: Vec<u8>,
    ) -> Result<InstallDisposition, UpdateError>;
}

#[async_trait::async_trait]
pub trait UpdateBackend: Send + Sync {
    async fn check(&self) -> Result<Option<Box<dyn CheckedUpdate>>, UpdateError>;
}

pub enum InstallDisposition { PluginOwnsProcessExit, InstalledNeedsRelaunch }

#[tauri::command]
pub fn get_update_state(state: tauri::State<'_, UpdateService>) -> UpdateStateDto;
#[tauri::command]
pub async fn check_for_updates(state: tauri::State<'_, UpdateService>) -> Result<UpdateStateDto, String>;
#[tauri::command]
pub async fn install_update(approved_version: String, state: tauri::State<'_, UpdateService>) -> Result<(), String>;
```

`TauriCheckedUpdate` は`tauri_plugin_updater::Update`自体をopaqueに所有する。productionとofficial integration testの`download`は、その保持した同一objectの公式methodを直接呼び、返ったverified bytesと同じobjectだけを`install(bytes)`へ進める。`UpdateService`は`Mutex<ServiceState>`内に`pending: Option<Box<dyn CheckedUpdate>>`と`operation_in_flight: bool`を持ち、checkで検証済みの同一objectを格納、ユーザーが画面に表示されたversionを`approved_version`として明示承認した場合だけtakeする。install直前の再checkやmetadataだけの再構成は行わない。local endpoint未構成時は`Disabled`; production overlay未構成はbuild前に拒否する。Windowsでは`PluginOwnsProcessExit`をproductionで戻る前提にせず、serviceのpre-install flushと`UpdaterBuilder::on_before_exit`へ登録した同一idempotent flusherの両方をinstall前に構成する。macOS serviceは`InstalledNeedsRelaunch`をcommand adapterへ返し、adapterが最後に`AppHandle::restart() -> !`を呼ぶ。service testはこのoutcomeをassertし、戻らないTauri APIをfake化しない。

`tauri-plugin-updater`は`=2.10.1`へexact pinしCargo.lockを固定する。fixture CA注入に使う`configure_client` APIもこのminorで固定し、upgrade時はofficial integration testの再承認を必須とする。

Production updater builderはダウングレード許可を`false`に固定し、effective configの`dangerous*`オプションは存在する場合に`false`以外を拒否する。

- [ ] **Step 4: Wire plugin, startup check, UI, bindings and Capability**

`lib.rs`へpluginとcommandsを追加する。setup後にendpoint構成済みの場合のみ1回startup checkを起動する。`UpdatesPanel`はcheck結果のversion/notesを表示し、別の「このバージョンを導入」buttonでのみ`install_update(approved_version)`を呼ぶ。実行中操作はdisableし、`update-progress`は`app.get_webview_window("settings")?.emit(...)`のみとする。Windowsはinstall開始前と`on_before_exit`で`RestartPending`をidempotent flushし、pluginにexitを委ねる。macOSだけinstall成功後にrelaunchする。Settingsだけにpermissionを付与し、Character/Chat拒否テストを追加する。

- [ ] **Step 5: Implement fail-closed release overlay rendering**

`PW_UPDATER_PUBLIC_KEY`、HTTPS `PW_UPDATER_ENDPOINT`、`TAURI_SIGNING_PRIVATE_KEY`を必須とする。base+generated overlayをTask 1の`loadEffectiveConfig`でmergeし、その実効configをrelease mode validatorに渡す。Windows OS signingはTask 5の公式OIDC actionによる二段階署名を唯一の経路とし、release overlayへclient-secret型`signCommand`を生成しない。fixture test keyと同一内容/fingerprintがrelease modeへ入れば失敗する。

- [ ] **Step 6: Verify GREEN**

Run: `node --test tools/scripts/render-release-config.test.mjs`

Run: `cargo test -p pw-contracts update`

Run: `cargo test -p parallel-world-desktop updates`

Run: `cargo test -p parallel-world-desktop --test updater_official -- --nocapture`

Run: `cargo test -p parallel-world-desktop --test capabilities`

Run: `corepack pnpm typecheck`

Run: `corepack pnpm test`

Expected: updater/tamper/capability tests PASS and bindings match DTOs。

- [ ] **Step 7: Commit**

```powershell
# Files節の各exact fileだけをgit add --でstageし、cached name allowlistを照合する
git commit -m "feat(updater): require signed update verification"
```

---

### Task 3: Verified model manifest and atomic installer

**Files:**
- Create: `crates/pw-platform/src/models/mod.rs`, `manifest.rs`, `installer.rs`
- Create: `crates/pw-platform/src/bin/install-models.rs`
- Create: `crates/pw-platform/tests/model_installer.rs`, `model_installer_cli.rs`
- Modify: `crates/pw-platform/src/lib.rs`, `crates/pw-platform/Cargo.toml`
- Modify: `Cargo.toml`, `Cargo.lock`
- Modify: `content/model-manifests/vad/silero-vad-v5.json`, `content/model-manifests/stt/reazonspeech-k2-v2.json`
- Modify: `tools/scripts/download-stt-models.mjs`
- Create: `crates/pw-contracts/src/dto/model.rs`, `apps/desktop/src-tauri/src/commands/models.rs`
- Modify: `crates/pw-contracts/src/dto/mod.rs`, `crates/pw-contracts/src/bin/export_bindings.rs`, `packages/contracts/src/index.ts`
- Modify: `apps/desktop/src-tauri/src/commands/mod.rs`, `apps/desktop/src-tauri/src/lib.rs`
- Modify: `apps/desktop/src-tauri/capabilities/settings.json`, `apps/desktop/src-tauri/tests/capabilities.rs`
- Create: `apps/desktop/src/windows/settings/ModelsPanel.tsx`, `ModelsPanel.test.tsx`
- Modify: `apps/desktop/src/windows/settings/SettingsWindow.tsx`
- Modify: `package.json`, `pnpm-lock.yaml`

**Interfaces:**
- Manifest v2: `schema_version=2,id,kind,version,name,file,url,size_bytes,sha256,license{spdx,url,text_sha256},install_dir,files[{path,size_bytes,sha256}]`。
- Settings-only commands: `list_models() -> Result<Vec<ModelStatusDto>, String>`、async `install_model(id: String) -> Result<ModelStatusDto, String>`、`cancel_model_install(id: String) -> Result<(), String>`。
- Window-scoped event: `model-install-progress`。

- [ ] **Step 1: Write malicious/corrupt input tests**

Reject HTTP URL、HTTPSからHTTPへのredirect、credential付き/final URL、wrong artifact length/hash、license body hash mismatch、absolute/`..`/Windows prefix install path、archive traversal、symlink、hardlink、Windows reparse point、case-fold collision、duplicate entry/path、entry数上限超過、単一file/展開後総byte上限超過、missing/unlisted expected file、per-file size/hash mismatch、existing install with one modified byte、unknown manifest IDを個別に固定する。Cancel時はdownload/extract/swapのそれぞれで停止し、現行installを保存する。Successは`layout.models` 直下の同一volume stagingで全outputを検証後、backup swapし、各rename直後のcrash注入テストでstartup recoveryがold/newのどちらか一方のみを完全なinstallとして復元することを固定する。license本文はhash検証後に`LICENSE.txt`としてinstallへ保存する。

- [ ] **Step 2: Verify RED**

Run: `cargo test -p pw-platform models -- --nocapture`

Expected: FAIL because `pw_platform::models` does not exist。

- [ ] **Step 3: Implement strict schema and streaming installer**

```rust
pub async fn install_verified(
    client: &reqwest::Client,
    layout: &AppDataLayout,
    manifest: &ModelManifest,
    cancelled: &std::sync::atomic::AtomicBool,
    progress: impl Fn(InstallProgress) + Send,
) -> Result<InstalledModel, ModelInstallError>;
```

Manifest registryは次のcompile-time bytesからのみ生成し、runtimeの任意path/URLを受け取らない。

```rust
const BUILTIN_MANIFESTS: &[&[u8]] = &[
    include_bytes!("../../../../content/model-manifests/vad/silero-vad-v5.json"),
    include_bytes!("../../../../content/model-manifests/stt/reazonspeech-k2-v2.json"),
];
pub fn builtin_manifest(id: &str) -> Result<&'static ModelManifest, ModelManifestError>;
```

HTTP clientはredirectごとにHTTPS、credential無し、host policyを再評価し、`Response::url()`のfinal URLも同じpolicyで検証する。Artifact/licenseをstreamしてbyte数/SHA-256を計測する。Archiveはentry 10,000、単一file 1 GiB、展開後総量 4 GiBをhard capとし、entry type/path/case-fold/duplicateをwrite前に検査する。展開後は`symlink_metadata`とWindows file attributesでreparse pointがないことも検証する。既存installもexistence-onlyでskipせず全expected fileのsize/hashする。Stagingは`models/.staging/<id>-<nonce>`、backupは`models/.backup/<id>`とし、journal `models/.transactions/<id>.json`をfsyncして`current -> backup`, `staging -> current`, verify, backup削除の順に進める。`recover_model_transactions()`をbootstrapでspeech serviceより前に実行する。

- [ ] **Step 4: Migrate manifests without bundling model bytes**

Silero version `5.1.2`、ReazonSpeech version `2024-08-01`、artifact/license実測hash、各expected fileの実測size/SHA-256を記録する。Generator testはartifactを一時取得してmanifest値と照合し、各manifestの`files`が非空でpath/size/hashの重複がないことを固定する。model bytesはGit/bundleへ入れない。

- [ ] **Step 5: Add Settings IPC/UI and share installer with CLI**

DTOはschema、id、version、state、verified、license SPDX、installed bytesを含む。UIは未導入/検証済み/破損/取得中/取得中止/失敗を区別し、取得中のみ`cancel_model_install`を出す。`install_model`はIDごと1本だけの実行とし、`model-install-progress`はsettings windowにだけemitする。`download-stt-models.mjs`は下記binaryを呼ぶthin wrapperへ変更し、独自tar/existence-only処理を削除する。

`crates/pw-platform/src/bin/install-models.rs`のCLIは`install-models --app-data-root <absolute-path> --id <silero-vad-v5|reazonspeech-k2-v2|all> [--json]`だけを受理し、stdout JSONは`{"schema_version":1,"id":"...","state":"verified","installed_bytes":0}`のNDJSONとする。Exit codeは0=全検証成功、2=引数/root不正、3=unknown ID、4=download/hash/license/archive検証失敗、5=filesystem/swap/recovery失敗、130=cancelとする。stderrにURL credentialやlicense本文を出さない。Node wrapperはOSごとのapp data rootを解決し、`cargo run --locked -p pw-platform --bin install-models -- --app-data-root <root> --id all --json`を`execFile`で呼び、exit codeをそのまま返す。

- [ ] **Step 6: Verify GREEN**

Run: `cargo test -p pw-platform --test model_installer`

Run: `cargo test -p pw-platform --test model_installer_cli`

Run: `cargo test -p parallel-world-desktop models`

Run: `cargo test -p parallel-world-desktop --test capabilities`

Run: `corepack pnpm --filter @parallel-world/desktop test -- ModelsPanel`

Expected: hash/license/traversal/atomicity/UI/capability tests PASS。

- [ ] **Step 7: Commit**

```powershell
# Files節の各exact fileだけをgit add --でstageし、cached name allowlistを照合する
git commit -m "feat(models): verify licensed model installs atomically"
```

---

### Task 4: Reproducible third-party inventory and Settings UI

**Files:**
- Create: `about.toml`, `tools/licenses/about.hbs`, `content/third-party/manual-licenses.json`, `content/third-party/model-license-register.json`
- Create: `tools/scripts/generate-third-party-licenses.mjs`, `generate-third-party-licenses.test.mjs`
- Create: `apps/desktop/src-tauri/resources/third-party-licenses.json`
- Create: `crates/pw-contracts/src/dto/license.rs`, `apps/desktop/src-tauri/src/commands/licenses.rs`
- Modify: `crates/pw-contracts/src/dto/mod.rs`, `crates/pw-contracts/src/bin/export_bindings.rs`, `packages/contracts/src/index.ts`
- Modify: `apps/desktop/src-tauri/src/commands/mod.rs`, `apps/desktop/src-tauri/src/lib.rs`
- Modify: `apps/desktop/src-tauri/capabilities/settings.json`, `apps/desktop/src-tauri/tests/capabilities.rs`
- Create: `apps/desktop/src/windows/settings/LicensesPanel.tsx`, `LicensesPanel.test.tsx`
- Modify: `apps/desktop/src/windows/settings/SettingsWindow.tsx`
- Modify: `package.json`, `pnpm-lock.yaml`

**Interfaces:**
- Generator merges cargo-about、`corepack pnpm licenses list --prod --json --long`、Live2D manual entries、model manifest license records。
- Settings-only `get_third_party_licenses() -> Result<Vec<ThirdPartyLicenseDto>, String>`。

- [ ] **Step 1: Write generator policy tests and verify RED**

Reject unknown/missing/empty license、missing text、license本文hash mismatch、conflicting duplicate、absolute source path、pnpm workspace importer取りこぼし、VAD/STT/LLM/TTSの種別取りこぼし。Assert stable ecosystem/name/version sort and byte-identical repeated output。downloadable Silero VAD/ReazonSpeechのversion/SPDX/許諾URL/本文hashはTask 3のmodel manifestを唯一のsource of truthとし、`model-license-register.json`には重複記録せず、現在特定modelを内包しないexternal LLM/AivisSpeech TTSの`external_not_redistributed`だけを記録する。registerからdownloadable IDを再定義した場合、またはmanifestと重複した場合はfatalとする。external entryには「ユーザーが選択したmodelは配布artifactに含まない」という固定説明と再配布不可を記録する。

Run: `node --test tools/scripts/generate-third-party-licenses.test.mjs`

Expected: FAIL because generator does not exist。

- [ ] **Step 2: Implement deterministic generation**

Tool versionは`cargo-about 0.8.4`に固定し、root package scriptを次の通り追加する。

```json
{
  "licenses:setup": "cargo install cargo-about --version 0.8.4 --locked",
  "licenses:generate": "node tools/scripts/generate-third-party-licenses.mjs",
  "licenses:check": "corepack pnpm licenses:generate && git diff --exit-code -- apps/desktop/src-tauri/resources/third-party-licenses.json"
}
```

Generatorは実行前に`cargo about --version` が`cargo-about 0.8.4`と一致することを求め、`cargo about generate tools/licenses/about.hbs --config about.toml --output target/license-generation/rust.json`を実行する。generator CLIは`--output <path>`と`--verify-reproducible --evidence-output <path>`を受理する。後者は独立temp directoryへ2回生成し、Bufferをbyte比較して差異時exit 1、成功時は両方のSHA-256をmachine-readable evidenceへ記録する。未指定時だけtracked inventoryへ出力する。`about.toml`は`private={ignore=true}`、`no-clearly-defined=true`、Windows/macOS target両方を指定し、network augmentationを無効にする。Ignoreは非公開`UNLICENSED` workspace packagesだけとし、dependency failuresは必ずfatalにする。Run `corepack pnpm install --frozen-lockfile` before collection; missing package index is an environment error, not permission to skip JS dependencies。

JSは`pnpm-workspace.yaml`と各`package.json`からroot、`@parallel-world/desktop`、`@parallel-world/contracts`、`@parallel-world/live2d-runtime`の4 importerを導出し、それぞれに`corepack pnpm --filter <importer> licenses list --prod --json --long`を実行する。期待importer setと収集済みsetが一致しなければfatal。License本文はpackageが指すlicense fileから読み、SHA-256をinventoryに保存する。Live2D/modelはmanual registerの本文path/hashと一致した場合だけmergeする。

- [ ] **Step 3: Add embedded command and Settings panel**

Rust uses`include_bytes!` and typed deserialization; no arbitrary file path is accepted。`ThirdPartyLicenseDto`は`name, version, ecosystem, spdx, source_url, license_text, license_text_sha256, distribution_status`を持ち、commandはembedded bytesの各license本文hashを再計算してから返す。UI supports search、ecosystem filter、license text and model/Live2D provenance。Chat/Character capability denial remains tested。

- [ ] **Step 4: Verify GREEN, reproducibility and drift**

初回生成前にgenerator自身のfail-closed byte比較を実行する。

Run: `node tools/scripts/generate-third-party-licenses.mjs --verify-reproducible --evidence-output target/license-generation/reproducibility.json`

Expected: exit 0、2つのSHA-256が一致。1 byteでも異なればexit 1。tracked inventory作成後は次をdrift gateとする。

Run: `corepack pnpm licenses:generate && git diff --exit-code -- apps/desktop/src-tauri/resources/third-party-licenses.json`

Run: `cargo test -p pw-contracts license`

Run: `cargo test -p parallel-world-desktop licenses`

Run: `cargo test -p parallel-world-desktop --test capabilities`

Run: `corepack pnpm --filter @parallel-world/desktop test -- LicensesPanel`

Expected: inventory unchanged and all license/UI/capability tests PASS。

- [ ] **Step 5: Commit**

```powershell
# Files節の各exact fileだけをgit add --でstageし、cached name allowlistを照合する
git commit -m "feat(licenses): generate and display third-party notices"
```

---

### Task 5: Secretless PR CI and protected signed release CI

**Files:**
- Modify: `.github/workflows/ci.yml`
- Create: `.github/workflows/release.yml`
- Create: `tools/scripts/verify-bundle-artifacts.mjs`, `verify-bundle-artifacts.test.mjs`
- Create: `tools/scripts/verify-release-version.mjs`, `verify-release-version.test.mjs`
- Create: `tools/scripts/generate-latest-json.mjs`, `generate-latest-json.test.mjs`
- Create: `tools/scripts/publish-draft-release.mjs`, `publish-draft-release.test.mjs`
- Create: `tools/scripts/create-artifact-manifest.mjs`, `create-artifact-manifest.test.mjs`
- Create: `docs/releases/app-v0.1.0.md`（以後はrelease準備commitで`docs/releases/<tag>.md`を追加）
- Modify: `package.json`, `pnpm-lock.yaml`

**Interfaces:**
- PR CI produces unsigned Windows NSIS/macOS app without signing secrets。
- Release CI consumes protected `production` environment and fails closed if updater/OS signing inputs are missing。

- [ ] **Step 1: Write artifact/workflow tests and verify RED**

Reject zero-byte installer、missing updater signature、invalid`latest.json`、bundle containing models/characters、production output containing test-key fingerprint、placeholder icon hash、release job without`production` environment、signing secret reference in pull_request job、`uses:`の非40文字SHA、OIDC/attestation permissions不足、曖昧runner/target/artifact key、tag/Tauri/Cargo/npm version不一致。Workflow structural testはbuild jobにrelease ID/tag/GITHUB_TOKEN/release upload/`uploadUpdaterJson: true`が存在しないこと、各tauri-actionへ`uploadUpdaterJson: false`が明示されること、単一publish jobだけがgeneratorとpublisherを各1回呼ぶことを固定する。NSIS内容検査はdevDependencyの`7zip-bin@5.2.0`の`path7za`だけを使用し、runnerのPATH上の7zやinstaller実行は使わない。

`generate-latest-json.mjs --artifact-manifest <json> --base-url <https-url> --version <semver> --notes-file <path> --pub-date <RFC3339> --output <path>`はexactly 3つのartifact keyと固定Tauri platform key `windows-x86_64`、`darwin-aarch64`、`darwin-x86_64`を入力する。versionはtag/Tauri/Cargo/npm version verifierの値と厳密一致する。notes pathは必ずtracked `docs/releases/app-v<version>.md`で、front matter `tag: app-v<version>`と`version: <version>`、非空本文を要求し、missing/untracked/mismatchをfail closedにする。初回は`docs/releases/app-v0.1.0.md`を作成し、以後はrelease準備commitで同規約の新規fileを追加する。pub-dateはtag commit timestampをworkflowがRFC3339へ正規化した固定値とし、現在時刻を読まない。各entryは`bundle_path/bundle_sha256`、`updater_path/updater_sha256`、`signature_path/signature_sha256`を別々に持つ。Windowsで同じpathを共有する場合もhash一致を必須とし、macOSは配布`.zip`とupdater `.app.tar.gz`を別pathにする。Tauri static updater schemaの`version,notes,pub_date,platforms[target].url,platforms[target].signature`へ決定的にmappingし、missing/duplicate/unknown target、credential/HTTP URL、invalid timestamp、複数writer markerを拒否する。testは入力順序を入れ替えてもbyte-identicalな唯一の`latest.json`になることを固定する。

`create-artifact-manifest.mjs`はTask 5で実装し、release workflow自身が署名完了後に3 platformのmanifestを生成する。Task 6は同じCLIをlocal acceptanceにも再利用する。

`publish-draft-release.mjs`はGitHub REST APIをNode 24標準`fetch`だけで呼び、draft releaseを作成または同一tagの既存draftへ再開する。再実行時はasset nameごとにremote digest/sizeを照合し、同一digestなら再利用、異なるdigest・duplicate・manifest外のstale assetがあればfail closedとする（暗黙置換しない）。検証済みinstaller/updater/macOS archiveを先にuploadし、upload後digestを再照合してから`latest.json`を必ず最後に1回だけuploadする。公開直前にasset setがmanifestと完全一致することを再確認する。tag pushはdraft作成までとし、`workflow_dispatch`の明示`publish=true`とproduction environment承認時だけ全evidenceを再検証してdraftをpublishedへ遷移する。draft中はGitHub `releases/latest`から見えないため、production updater endpointへ`latest.json`を公開済みと扱わない。

Run: `node --test tools/scripts/verify-bundle-artifacts.test.mjs`

Expected: FAIL because verifier does not exist。

- [ ] **Step 2: Extend PR CI without secrets**

Keep Windows/macOS matrixだが、runner/target/artifact keyは次の3行に固定する。

```yaml
include:
  - runner: windows-2025
    target: x86_64-pc-windows-msvc
    artifact_key: windows-x86_64-nsis
  - runner: macos-15
    target: aarch64-apple-darwin
    artifact_key: macos-aarch64-app
  - runner: macos-15-intel
    target: x86_64-apple-darwin
    artifact_key: macos-x86_64-app
```

Pin Node 24.15.0、pnpm 11.11.0、Rust 1.96.0。Run frozen install、`licenses:setup`、license drift、distribution config、fmt、clippy -D warnings、Rust/frontend tests、typecheck/build。Build/verify/upload Windows NSIS and ad-hoc-signed macOS app local artifacts。`uses:`はレビュ時に確認した40文字commit SHAを記録し、`verify-bundle-artifacts.test.mjs`は`uses: owner/repo@<40 hex>`以外を拒否する。Artifact uploadはkeyをnameに使い`if-no-files-found: error`とする。PR CI permissionは`contents: read`だけとする。

Workflowで使うactionは実装時に公式release/tagとcommitを再照合し、`uses: owner/repo@<40 hex> # vX.Y.Z`のようにtag commentも残す。現時点のreview済みSHAは `actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683`、`actions/setup-node@49933ea5288caeca8642d1e84afbd3f7d6820020`、`pnpm/action-setup@f2b2b233b538f500472c7274c7012f57857d8ce0`、`actions/upload-artifact@ea165f8d65b6e75b540449e92b4886f43607fa02`、`actions/download-artifact@d3f86a106a0bac45b974a628896c90dbdf5c8093`、`azure/login@a457da9ea143d694b1b9c7c869ebb04ebe844ef5`、`azure/artifact-signing-action@c0ae2c1d0c1847ab81ac0ab8521bee597cfedd30`、`tauri-apps/tauri-action@e834788a94591d81e3ae0bd9ec06366f44c867a9`、`actions/attest-build-provenance@e8998f949152b193b063cb0ec769d69d929409be`。verifierは40文字SHAに加えてtag commentを要求する。Release uploadは追加actionやrunner同梱`gh`に依存せず、repo-owned Node publisherだけを使う。Rustはactionを使わず`rustup toolchain install 1.96.0 --profile minimal --component rustfmt,clippy`を実行する。

- [ ] **Step 3: Add protected release workflow**

Trigger `app-v*` and manual dispatch。3つのOS build jobはそれぞれ`environment: production`で固定runner/target/artifact keyを使い、署名・notarize・staple・updater署名まで行った後、GitHub workflow artifactへだけuploadする。各jobからreleaseへ直接publishせず、OS間で共有される`latest.json`も生成しない。後段の単一`publish` jobも`environment: production`とし、全3artifactをdownloadして再検証後、全platformを含む唯一の`latest.json`を生成し、provenanceをattestしてから1つのdraft releaseへuploadする。Workflow/job permissionsは最小scopeで`contents: write`, `id-token: write`, `attestations: write`, `artifact-metadata: write`を明示し、それ以外は`none`とする。`verify-release-version.mjs --tag $GITHUB_REF_NAME`は`app-vX.Y.Z`、`tauri.conf.json.version`、`apps/desktop/src-tauri/Cargo.toml package.version`、`apps/desktop/package.json.version`を厳密一致させ、pre-release/build metadataの表記差も拒否する。Render release overlay。tauri-actionは署名済みupdater artifactsを生成するが、release uploadとupdater JSON publishは無効にし、`uploadUpdaterJson: false`を明示する。updater artifact生成に使うrepoの`@tauri-apps/cli`は`2.11.4` exact、Rust `tauri`はCargo.lockの`2.11.5`、pluginは`2.10.1` exactとし、workflow/acceptance verifierが3versionを記録する。

Windows jobはGitHub OIDCによるAzure loginのためproduction environment vars `AZURE_CLIENT_ID`, `AZURE_TENANT_ID`, `AZURE_SUBSCRIPTION_ID`, `AZURE_ARTIFACT_SIGNING_ENDPOINT`, `AZURE_ARTIFACT_SIGNING_ACCOUNT`, `AZURE_ARTIFACT_SIGNING_PROFILE`を必須とし、`azure/login`へclient/tenant/subscription IDを明示的に渡す。client secretを要求する`artifact-signing-cli 0.11.0`は使用しない。公式`azure/artifact-signing-action` v2（上記SHA）はDefaultAzureCredential workload identityを使う。署名順序を固定する: (1) `tauri build --no-bundle`で生成したinner application `.exe`をactionでAuthenticode署名・検証、(2) `tauri bundle --bundles nsis`でNSISと一時`.sig`を生成、(3) outer NSIS `.exe`をactionでAuthenticode署名・検証、(4) bytesが変わる前のstale `.sig`を削除、(5) exact `@tauri-apps/cli 2.11.4`の`tauri signer sign`で**最終NSIS bytes**をupdater秘密鍵により再署名、(6) public keyでsignatureを検証してからmanifestを生成する。stale `.sig`の再利用とminisign後のNSIS変更をverifierで拒否する。

macOS jobはsecrets `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_KEYCHAIN_PASSWORD`, `APPLE_API_KEY_CONTENT`、vars `APPLE_SIGNING_IDENTITY`, `APPLE_API_ISSUER`, `APPLE_API_KEY`, `APPLE_TEAM_ID`を必須とする。Certificateをtemporary keychainへimportし、API key contentをpermission 0600のtemporary `.p8`へ書き、Tauriに`APPLE_SIGNING_IDENTITY`, `APPLE_API_ISSUER`, `APPLE_API_KEY`, `APPLE_API_KEY_PATH`, `APPLE_TEAM_ID`を渡してcodesign/notarize/stapleする。終了時にkeychain/.p8をalways cleanupする。

OS build jobのartifact verifierがAuthenticode/codesign/notarization/updater signature/model非内包/license inventory/iconを確認してからworkflow artifactへuploadする。macOS `.app`は通常zipではなく`ditto -c -k --sequesterRsrc --keepParent`でsymlink、mode、extended attributesを保持したarchiveにし、publish jobで展開後に再度codesign/notarization/staplingを検証する。単一publish jobは全platform updater artifactとsignatureから`latest.json`を一度だけ生成・検証し、検証済みinstaller、macOS app archive、updater artifacts、`latest.json`をattestation subjectにしてからdraft releaseへuploadする。PR jobs never reference secrets。

- [ ] **Step 4: Verify GREEN**

Run: `node --test tools/scripts/verify-bundle-artifacts.test.mjs`

Run: `node --test tools/scripts/verify-release-version.test.mjs`

Run: `node --test tools/scripts/create-artifact-manifest.test.mjs tools/scripts/generate-latest-json.test.mjs tools/scripts/publish-draft-release.test.mjs`

Run: `corepack pnpm distribution:verify && corepack pnpm licenses:generate`

Expected: workflow/artifact tests PASS and PR CI requires no release secret。

- [ ] **Step 5: Commit**

```powershell
git add -- .github/workflows/ci.yml .github/workflows/release.yml tools/scripts/verify-bundle-artifacts.mjs tools/scripts/verify-bundle-artifacts.test.mjs tools/scripts/verify-release-version.mjs tools/scripts/verify-release-version.test.mjs tools/scripts/create-artifact-manifest.mjs tools/scripts/create-artifact-manifest.test.mjs tools/scripts/generate-latest-json.mjs tools/scripts/generate-latest-json.test.mjs tools/scripts/publish-draft-release.mjs tools/scripts/publish-draft-release.test.mjs docs/releases/app-v0.1.0.md package.json pnpm-lock.yaml
# 次にgit diff --cached --name-onlyを上記15 pathのallowlistと照合する
git commit -m "ci(release): verify and publish signed desktop artifacts"
```

---

### Task 6: Acceptance, external gates and cross-cutting review

**Files:**
- Create: `docs/development/phase7-acceptance.md`, `docs/development/phase7-external-gates.md`
- Create: `tools/scripts/verify-phase7-acceptance.mjs`, `verify-phase7-acceptance.test.mjs`
- Modify: `docs/development/getting-started.md`, `docs/development/handoff-2026-07-13.md`, `README.md`

**Interfaces:**
- Acceptance separates complete local/mock gates from credential/permission-dependent signed production gates。
- `verify-phase7-acceptance.mjs --mode <local|release> --platform <windows|macos|all> --artifact-manifest <json> --evidence-output <json>` integrates config、license、workflow、version、bundle、latest JSON、signature、icon、model exclusion checks into one machine-readable summary。Exit 0=all required gates pass、2=argument/manifest invalid、3=verification failed。
- `create-artifact-manifest.mjs --mode <local|release> --platform <windows|macos|all> --artifact-root <path> --output <json>` discovers only the fixed Tauri target/profile paths, requires exactly one match for every required bundle/updater/signature kind, computes SHA-256, and emits the schema consumed by both latest generator and acceptance verifier。Zero/multiple/unknown matches fail。

- [ ] **Step 1: Record the exact local acceptance matrix**

Document commands/artifact paths for `windows-2025/x86_64-pc-windows-msvc/windows-x86_64-nsis`、`macos-15/aarch64-apple-darwin/macos-aarch64-app`、`macos-15-intel/x86_64-apple-darwin/macos-x86_64-app`、valid/tampered official mock update、model per-file hash/license/cancel/crash recovery/rollback、license drift、Updates/Models/Licenses Settings UI/Capability、full gates。Artifact manifestはkeyごとにexact path、target、profile、SHA-256、signature pathを持ち、globを受け付けず、同じkey/pathの重複と不足を拒否する。local modeは選択platformのexactly-one artifact、release modeは3 key完全性・唯一の`latest.json`・OS署名/notarization・updater署名・attestation evidenceを必須とする。Record local artifact SHA-256 and prove generated license JSON/new icon included、placeholder icon/model/character bytes excluded。macOS local artifactは`codesign --verify --deep --strict`でad-hoc signatureを検証する。

- [ ] **Step 2: Record external release blockers**

Name owner/input/evidence for Azure OIDC client/tenant/subscription + Artifact Signing endpoint/account/profile、Apple certificate/identity/API issuer/key/team/notarization、HTTPS updater URL、updater keypair、Live2D SDK/Coreのrelease permission、Live2D character model redistribution permission、Silero VAD、ReazonSpeech STT、user-selected external LLM、AivisSpeech TTS/voice modelのversion/配布有無/SPDX/許諾URL/本文hash/再配布permission。LLM/TTSが`external_not_redistributed`の間は「内包なし」をartifact検証の証拠とし、特定modelをbundleする変更はこのgateとmanifest追加なしでmergeしない。Missing gate blocks signed release but not local implementation status。

- [ ] **Step 3: Run complete verification**

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
corepack pnpm build
corepack pnpm typecheck
corepack pnpm test
corepack pnpm distribution:verify
corepack pnpm licenses:setup
node tools/scripts/generate-third-party-licenses.mjs --verify-reproducible --evidence-output target/license-generation/reproducibility.json
corepack pnpm licenses:generate
git diff --exit-code -- apps/desktop/src-tauri/resources/third-party-licenses.json
node --test tools/scripts/verify-release-version.test.mjs
node --test tools/scripts/verify-bundle-artifacts.test.mjs
corepack pnpm --filter @parallel-world/desktop tauri build --debug --no-bundle
corepack pnpm bundle:windows:local
node --test tools/scripts/create-artifact-manifest.test.mjs tools/scripts/verify-phase7-acceptance.test.mjs
node tools/scripts/create-artifact-manifest.mjs --mode local --platform windows --artifact-root target/release/bundle --output artifacts/phase7/windows-local-artifacts.json
node tools/scripts/verify-phase7-acceptance.mjs --mode local --platform windows --artifact-manifest artifacts/phase7/windows-local-artifacts.json --evidence-output artifacts/phase7/windows-local-acceptance.json
```

Expected on Windows: all gates PASS and NSIS verifier reports non-empty/model-free。macOS CI must also pass`bundle:macos:local` before local acceptance is complete。

- [ ] **Step 4: Request independent review**

Review updater same-object/signature bypass resistance、effective-config fail-closed release、separate OS signing、model redirect/link/reparse/case collision/caps/cancel/rehash/backup swap/crash recovery、VAD/STT/LLM/TTS license completeness、Capability denial、CI secret/OIDC/Apple boundary、action SHA/tool pin、version match、artifact model/placeholder exclusion。Resolve all Critical/Important。

- [ ] **Step 5: Commit acceptance docs**

```powershell
git add -- docs/development/phase7-acceptance.md docs/development/phase7-external-gates.md docs/development/getting-started.md docs/development/handoff-2026-07-13.md README.md tools/scripts/verify-phase7-acceptance.mjs tools/scripts/verify-phase7-acceptance.test.mjs
# 次にgit diff --cached --name-onlyを上記7 pathのallowlistと照合する
git commit -m "docs: define phase 7 distribution acceptance"
```

- [ ] **Step 6: Apply the completion rule**

Until credentials、public URL、permissions exist, status is`implementation complete; signed production release externally gated`。After protected release succeeds, append exact runner/target/artifact key、installer hashes、Authenticode verification、codesign/notarization/stapling result、updater artifact signature + same-object install result、`latest.json` verification、license inventory hash、icon source/generated hashes、GitHub attestation verification、CI/release URLs to acceptance docs and commit final evidence。
