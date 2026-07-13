# Phase 6 soak test

## 目的と受け入れ基準

長時間運転で資源が「一度も増えない」ことは要求しない。先頭2分をwarm-upとして除外し、残りの標本に線形回帰を適用する。RSSおよびprivate bytesは傾き64 MiB/hour以下、かつ最初と最後の増加64 MiB以下を合格条件とする。handleは傾き60/hour以下かつ増加100以下、threadは傾き6/hour以下かつ増加8以下とする。queue depthは各64以下、cacheは1,000 files以下、logは256 MiB以下、restartは8以下、faultは32以下とする。unexpected exit、panic、orphan processは0件でなければならない。

Rustの決定的短縮stressはfake timestampを使い、warm-up中の揺れを許容しつつ意図的なmemory leakを検出する。実時間soakはroot processとCIM（利用できない環境ではWin32 Toolhelp snapshot）で再帰取得した全子孫のWindows process counterを合算する。全上限は最後の標本ではなくrun中の最大値で判定する。2時間acceptanceでは有効な診断heartbeatが必須で、欠落はunknownとして失敗する。

heartbeatはUTF-8 JSONで、次のschema version 1をatomic replaceで定期更新する。`timestamp_ms`、resolve/fallback後の実active `audio_device`、`supervisor_healthy`、queue、drop、cache、log、restart、panic、fault、監視対象child PIDを実測値として書く。2時間acceptance中に`supervisor_healthy=false`を観測した場合は失敗とする。

全fieldは必須で、数値は非負とする。`process_id`はharnessが起動したroot PID、`timestamp_ms`と`started_timestamp_ms`はharness開始以後でなければならない。起動後10秒間は前runの残存fileを無視し、現runのheartbeatを一度受理した後、またはgrace経過後の欠落・不正を失敗とする。`timestamp_ms`の許容ageは `max(3 * SampleSeconds, 10秒)`。schema不完全、stale、負値、またはroot自身を含むstrict descendantではないchild PIDを含むheartbeatを受理しない。最初のfresh heartbeat取得時には実audio deviceをJSONLの`metadata_update`として追記する。

```json
{"schema_version":1,"process_id":4321,"run_id":"4321-1783900800000","started_timestamp_ms":1783900800000,"timestamp_ms":1783900801000,"audio_device":"Microphone Array","supervisor_healthy":true,"input_queue_depth":0,"output_queue_depth":0,"dropped_items":0,"cache_file_count":12,"log_bytes":4096,"restart_count":0,"panic_count":0,"fault_count":0,"child_process_ids":[1234]}
```

fault injectionは `-FaultInjection -FaultTarget OwnedChild -ConfirmOwnedFault` の3点を明示した場合だけ、harnessが起動したrootのstrict descendantを対象にする。root自身、任意PID、外部serviceは停止しない。faultとcleanupは観測時に全子孫のPIDとprocess開始時刻を固定し、停止直前にも同一identityと現在のownershipを再検証するため、長時間run中に再利用されたPIDを停止しない。通常終了時はrootを最後まで維持し、最後のsample後にもprocess treeを再取得する。PID、開始時刻、rootまでのcaptured ownership lineageを再検証して深い子孫から停止し、supervisorがreplacement childを生成した場合もdeadline内で再取得・停止するpassを空になるまで反復する。root停止後は全processを再列挙して残存子孫をorphanとして報告する。kill後10秒deadlineをtimelineへ記録し、別identityの子processが観測されなければ失敗する。外部AivisSpeech/llama-serverのfault試験はこのflagでは行わず、個別の手動確認手順を用いる。

## 実行

先にdebug executableをビルドする。

```powershell
corepack pnpm --filter @parallel-world/desktop tauri build --debug --no-bundle
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools/scripts/soak-test.ps1 -DurationMinutes 120 -SampleSeconds 5 -OutputDir artifacts/soak -DiagnosticsHeartbeat "$env:APPDATA/com.parallelworld.desktop/logs/soak-heartbeat.json"
```

短時間の自動検証（外部service不要、heartbeat mapping・引数引用・cleanupを含む）:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools/scripts/soak-test.ps1 -SelfTest
```

`-SelfTest -SelfTestRootChild` はroot PIDを`child_process_ids`へ混入したheartbeatを拒否する負例で、終了コード4と専用理由`root_child_heartbeat_rejected`を返す。summaryの`root_child_rejection`にはheartbeat未受理、fault timeline 0、rootがfault対象でないこと、orphan 0、期待外violation 0のassert結果を記録する。

各runは `<UTC>-<seed>.jsonl` と `<UTC>-<seed>-summary.json` を生成する。metadataにはgit hash/dirty、実行ファイルSHA-256、OS、heartbeatから取得した実audio device、seedを記録する。summaryにはfault timeline、全metricのmaximum、warm-up後のslope/growth、threshold、violations、cleanup後のorphan PIDを含める。

終了コードは `0=合格`、`2=引数または実行ファイル不正`、`3=予期しない終了`、`4=資源閾値違反`、`5=成果物生成または実行時エラー`。終了経路にかかわらず起動processとその子孫を停止する。
