# Irodori BAT Bootstrap Design

**Date:** 2026-07-19

**Scope:** Windowsで`ParallelWorld_run.bat`から開発版Parallel Worldを起動する際の、Irodori-TTS環境確認・任意構築・起動・終了処理

## Goal

`ParallelWorld_run.bat` の起動時にIrodori-TTS環境を検査し、未構築または破損している場合だけユーザーへ構築確認を表示する。同意された場合は、repositoryやsystem Pythonへ依存環境を作らず、ユーザーデータ領域へ再現可能なIrodori環境を構築する。

## Non-goals

- Tauriアプリ内のセットアップUI、IPC、progress eventは追加しない。
- NSIS、DMGなどの製品インストーラーは作成しない。
- CUDA Toolkit、GPU driver、WSL、ROCmをsystemへインストールしない。
- macOS、Linux向けbootstrap entry pointは追加しない。
- LLM serverを起動または停止しない。
- LoRAや参照音声を自動取得、生成、変換、削除しない。

## Entry Point and Flow

`ParallelWorld_run.bat` はPowerShellのbootstrap scriptを呼び出す。bootstrapは次の順序で処理する。

1. ユーザーデータ領域にあるmanaged Irodori環境を検査する。
2. 利用可能な環境がなければ、backend、概算容量、保存先、ライセンスと音声利用上の注意を表示し、構築するか確認する。
3. 拒否または失敗時はTTSを縮退させたままParallel Worldを起動する。
4. 同意時は固定manifestに従ってruntimeとmodelを取得・検証・構築する。
5. Irodori serverをloopback限定で起動し、healthとvoice listを確認する。voiceがある場合だけ短文WAV warm-upも確認する。
6. Parallel Worldをforegroundで起動する。
7. Parallel World終了後、この起動セッションが開始したSTT/TTS関連processだけを停止する。外部起動済みTTSとLLMは残す。

## Managed Environment

Irodori環境はrepository内ではなく、`%LOCALAPPDATA%\com.parallelworld.desktop\irodori`に配置する。`runtime\<manifest-version>`、`cache\downloads`、`transactions`、`user\voices`、`user\loras`を分離し、server source、portable `uv`、uv-managed CPython 3.10.20、virtual environment、model、codec、tokenizerをruntime内で管理する。

参照音声とLoRAはmanaged runtimeとは別directoryに置き、runtimeの修復・更新・削除対象に含めない。Dynamic LoRAを有効にするため、managed serverは常に `IRODORI_COMPILE_MODEL=false` で起動する。

system Python、Git、repository-local virtual environmentは使用しない。portable `uv`、server archive、Python、dependency lock、model、codec、`sbintuitions/sarashina2.2-0.5b` tokenizerは固定versionまたはrevisionとSHA-256をmanifestで管理する。起動時はHugging Faceをoffline modeにし、manifest外の暗黙downloadを拒否する。

## Windows Backend Selection

- supported NVIDIA GPU: `cu128`
- Radeon、Intel GPU、unsupported hardware: `cpu`

backendは自動検出した推奨値を確認画面へ表示する。対応状況が不明なGPUを黙ってGPU backendへ割り当てない。GPU backendのwarm-upが失敗した場合、ユーザー確認なしに別backendを追加構築しない。

## Download and Installation Safety

- HTTPSだけを許可し、redirect後のURLも検証する。
- download sizeとSHA-256を検証してから展開する。
- archive traversal、absolute path、symlink、hardlink、Windows reparse pointを拒否する。
- download、展開、environment構築、model配置、warm-upに必要な最大空き容量を開始前に確認する。
- incomplete環境にはcompletion markerを作らない。
- 同じmanifest versionの完成済み環境だけを再利用する。
- 新環境はversioned final pathで構築し、検証成功後にactive markerをatomic更新する。
- cancellation、失敗、端末終了後は次回起動時にincomplete transactionを安全に再開または破棄する。
- modelを初回推論時に未固定revisionから暗黙downloadさせない。

## Process Ownership and Shutdown

bootstrap sessionは、自身が起動したprocessのsession GUID、PID、process start time、canonical executable pathを保持する。

- Irodori、bootstrap中の`uv`/Python、アプリ管理のAivisSpeechはprocess tree単位で停止する。
- アプリ内STT worker、queue、audio deviceは通常のTauri shutdownで解放する。
- 起動前から存在したIrodori/AivisSpeech、同名の無関係process、LLM serverは停止しない。
- port競合時は未知processを終了またはmanaged serverとして採用しない。
- 通常終了ではgraceful stopを試み、期限後にowned process treeだけを強制終了する。

## User Interaction

`ParallelWorld_run.bat`は、呼出元が`PW_TTS_ENGINE`を設定していない場合だけ`irodori`を選ぶ。従来の`dev-up.bat`と`dev-up.ps1`の既定`aivis`は変更しない。

未構築時の確認には、検出backend、download容量、必要最大空き容量、保存先、主要ライセンス、voice cloningと第三者音声利用上の注意を表示する。

選択肢は「構築する」と「今回はしない」とする。「今回はしない」は恒久設定にせず、環境がない場合は次回のBAT起動時に再度確認する。構築中はstageと進捗をconsoleへ表示し、キャンセルを受け付ける。

## Failure Behavior

環境検査、download、構築、server起動、warm-upの失敗はアプリ全体を停止させない。安全な短いエラーを表示してTTSを縮退させ、Parallel Worldの起動を続行する。voiceが0件の場合は構築失敗にせず`ready_without_voice`として配置先を案内する。credential、authorization header、環境変数全体、ユーザーファイルの完全pathはログへ出力しない。

## Verification

自動テストはPowerShellを直接外部networkへ接続させず、fixture manifest、local HTTP server、fake executablesで次を検証する。

- 完成済み環境の再利用と未構築時の確認
- 拒否時にもアプリ起動を継続
- backend検出とplatform分岐
- size/hash不一致、unsafe archive、disk不足の拒否
- cancellation、incomplete transaction、atomic active marker
- health、voiceなしの`ready_without_voice`、voiceがある場合のRIFF/WAVE warm-up判定
- owned STT/TTSだけの停止と外部TTS/LLMの維持
- LoRAと参照音声の保持
- secretとユーザーpathのlog redaction

実serviceを使うacceptance testは明示的な環境変数opt-inとし、通常のtest suiteでは実model、外部API、有料serviceへ接続しない。公開前にはWindows 11のclean environmentで、確認、構築、warm-up、音声生成、終了後のprocess解放を確認する。

## Deferred Work

- Windows Radeon向けWSL/ROCm自動構築
- Linux Radeon向けROCm bootstrap
- Apple Silicon向けMPS bootstrap
- BATから起動する外部STT sidecarのownership管理
- 製品アプリ内セットアップUI
- 製品installerへのruntime同梱
- managed environmentのGUI修復・削除
- GitHub一般公開に必要なParallel World本体licenseとthird-party noticesの確定
