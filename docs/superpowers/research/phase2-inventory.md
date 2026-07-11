# Phase 2 音声入力・STT 実装前調査

調査日: 2026-07-11

## 1. 結論

Phase 2 は、`pw-audio`（デバイス、capture、sample変換、ring buffer、16 kHz mono化、診断）と `pw-stt-sherpa`（Silero VAD、発話境界、ReazonSpeech、認識結果フィルター）を別crateとして実装する。Tauriは両crateを所有するcomposition rootに留め、生PCMをWebViewへ送らない。

標準経路は公式 `sherpa-onnx` Rust APIを採用する。基本設計にある「Rust APIとC APIをspikeで比較」は、2026年現在、公式Rust crateが提供され、標準ではstatic linkと対応native libraryの自動取得が案内されているため、Rust APIのcompile/run smokeを先に実施する。C APIはRust APIで必要なVAD・offline recognizer・キャンセル境界・配布対象Windows/macOSを満たせないことが実測で判明した場合だけ、`pw-stt-sherpa/src/ffi/`へ隔離して比較する。

テストは二層に分ける。

1. 通常の `cargo test` は実マイク、native library、ONNXモデルを必要としない決定論的adapter/domain/fixtureテストとする。
2. 実モデル検証は明示的な環境変数とローカルモデル配置を要求するignored testまたは専用binaryとし、SHA-256・version・license manifestを検証してから実行する。

## 2. 仕様から抽出した不変条件

- capture flow: `cpal → bounded SPSC ring buffer → mono正規化 → 16 kHz resampling → level/noise floor → Silero VAD → utterance boundary → ReazonSpeech → normalization/filter → final utterance`。
- `cpal` callback内で許されるのはsample format変換と、事前確保した有界bufferへの投入だけ。割り当て、ロック待ち、VAD/STT、log、IPCは禁止。
- buffer満杯は待たずにdropし、drop数を診断へ記録する。
- VAD/STTはworker側で実行する。
- 生PCMはRustプロセス内だけで扱う。UIへ公開できるのはlevel、VAD状態、発話開始/終了、確定text、error/diagnosticのみ。
- 無音、短すぎる発話、低VAD確率、低SNR、重複、音響タグだけ、TTS再生中・終了直後のloopback候補、buffer欠落を複合条件で棄却する。
- 禁止語リスト単独での棄却は禁止。
- TTS再生中はSTTを停止できる。
- STT障害時にもtext入力を維持する。
- Phase 2受入条件: 無音10分でLLM送信0件、通常の短文を安定認識、TTS再生中のSTT停止、2時間運転でメモリが継続増加しない。
- Phase 6の復旧要件を壊さない境界を今から作る: device切断通知、STT再初期化、診断snapshot、秘密情報を含まないerror。

## 3. 現在のリポジトリ実物

### 3.1 実装済み基盤

- workspace membersは `parallel-world-desktop`、`pw-contracts`、`pw-domain`、`pw-platform` の4つ。
- Rust 1.96、edition 2024、workspace lint `unsafe_code = "forbid"`。
- `pw-domain` は会話状態、`pw-contracts` はRust/TypeScript DTO、`pw-platform` はapp data layoutを提供する。
- app data layoutには `models`、`cache`、`logs` 等が既にある。モデル配置先を新規にハードコードせず、このlayoutから導出する。
- 現在のcrateにはaudio device、PCM、VAD、STT、fixture runnerの実装はない。

### 3.2 モデル・fixture・licenseの実物確認

- tracked `project-input/` はLive2D資料だけで、Silero/ReazonSpeech ONNX、tokens、STT license、音声fixtureは存在しない。
- `tests/audio-fixtures/` も存在しない。
- Live2D model内のWAVはlip-sync/motion sampleであり、録音条件と期待結果を持つPhase 2 fixtureではないため流用しない。
- ReazonSpeech archiveとSilero modelは大容量／外部配布物なのでgitへ直接追加しない。Phase 7のmodel downloaderと同じmanifest形式へつなげられるよう、Phase 2ではローカルmanifestとhash検証を先に作る。
- license UIへ渡すため、取得物ごとにsource URL、version、SHA-256、license identifier、license/NOTICE pathをmanifestへ記録する。モデルをbundleする判断はlicenseレビュー後に別gateとする。

## 4. 公式一次情報の確認結果

### cpal

- 公式docs.rsの現行表示は `cpal 0.18.1`。default input configまたはsupported input configsを列挙し、実際にsupportedなconfigからstreamを構築する。device切断ではconfig列挙自体がerrorになり得る。
- sample formatはdevice依存なので `f32` 固定を仮定せず、`F32/I16/U16` 等を共通の正規化関数へ渡す。
- 参照: [cpal docs](https://docs.rs/cpal/latest/cpal/)

### ring buffer

- `ringbuf` はlock-free SPSCで、push/popは即時成功または失敗する。`HeapRb`を初期化時に確保し、producerをcallback、consumerをworkerが所有する形が仕様に合う。
- callbackではoverwrite APIを使わない。満杯時に古いsampleを暗黙上書きせず、`try_push`/`push_slice`の未投入数をatomic counterへ加算する。
- 参照: [ringbuf repository](https://github.com/agerasev/ringbuf)

### resampling

- `rubato`の現行docsはchunk処理と処理中のallocation回避を明記する。固定sample rate変換にはFFT、clock drift追従にはAsyncが案内されている。
- Phase 2ではcallback外workerでdevice nominal rateから16 kHzへ変換する。まず固定比 `Fft` を使い、2時間試験でclock driftによる欠落・蓄積が観測された場合だけ `Async` とring occupancy feedbackへ切り替える。bufferは初期化時に確保し `process_into_buffer` を用いる。
- 2026-07-11の検索結果ではlatest表示が3.0.0と4.0.0の更新境界にあるため、実装時にCargo解決版のAPIを固定し、lockfileとdocs URLをreview evidenceへ残す。
- 参照: [rubato docs](https://docs.rs/rubato/latest/rubato/)

### sherpa-onnx / Silero VAD

- 公式docsはRust APIを提供し、標準は `sherpa-onnx` crateのstatic link。native libraryはbuild時に対応archiveを自動取得する。offline/reproducible buildでは `SHERPA_ONNX_LIB_DIR` を明示する。
- 公式Silero pageはv4/v5をサポートする。k2-fsa配布の `silero_vad.onnx`（629 KB）とint8（208 KB）は16 kHz専用。Phase 2 pipelineの16 kHz固定と一致する。
- native auto-downloadをCIの暗黙副作用にしない。依存取得spikeで取得URL/hashを確定し、CIはmock tests、real-model jobは事前配置artifactを使用する。
- 参照: [Rust API installation](https://k2-fsa.github.io/sherpa/onnx/rust-api/install.html)、[advanced installation](https://k2-fsa.github.io/sherpa/onnx/rust-api/advanced-install.html)、[Silero VAD](https://k2-fsa.github.io/sherpa/onnx/vad/silero-vad.html)、[C API](https://k2-fsa.github.io/sherpa/onnx/c-api/index.html)

### ReazonSpeech

- sherpa-onnx公式配布モデル `sherpa-onnx-zipformer-ja-reazonspeech-2024-08-01` は日本語専用、35,000時間学習。入力featureは16 kHz、encoder/decoder/joiner/tokensからなるoffline transducerで、公式にSilero VADとのmicrophone例がある。
- 初期標準はint8 encoder/joinerと通常decoderを候補にし、fp32との認識品質・memory・real-time factorを実モデルgateで比較する。品質差が受入閾値を超える場合はfp32を標準にする。
- ReazonSpeech repositoryとsherpa-onnxはApache-2.0。ただしmodel archive内のlicense/NOTICEも取得時に検査し、repository licenseだけで再配布可否を断定しない。
- 参照: [official ReazonSpeech model instructions](https://k2-fsa.github.io/sherpa/onnx/pretrained_models/offline-transducer/zipformer-transducer-models.html#sherpa-onnx-zipformer-ja-reazonspeech-2024-08-01-japanese)、[ReazonSpeech repository](https://github.com/reazon-research/ReazonSpeech)、[sherpa-onnx license](https://github.com/k2-fsa/sherpa-onnx/blob/master/LICENSE)

## 5. 推奨ファイル構成とinterface

```text
crates/pw-audio/
  src/lib.rs
  src/device.rs            # device列挙、stable key、supported config
  src/capture.rs           # cpal callbackとstream lifetime
  src/pcm.rs               # sample format変換、mono mix
  src/ring.rs              # bounded SPSC + drop counters
  src/resample.rs          # worker側16 kHz変換
  src/metrics.rs           # level、noise floor、SNR、diagnostics
  src/worker.rs            # capture consumerとframe dispatch
  tests/pipeline.rs
crates/pw-stt-sherpa/
  src/lib.rs
  src/ports.rs             # mock可能なVadEngine/Recognizer
  src/segmenter.rs         # 発話境界state machine
  src/filter.rs            # 複合誤認識filter
  src/service.rs           # cancellation/TTS gate/turn IDs
  src/model_manifest.rs    # path/hash/version/license検証
  src/sherpa.rs            # official Rust API adapter
  tests/adapter_contract.rs
  tests/fixture_acceptance.rs
tests/audio-fixtures/
  schema/fixture.schema.json
  silence/
  fan-noise/
  keyboard/
  laughter/
  short-command/
  normal-conversation/
  false-start/
  tts-loopback/
  long-running/
tools/audio-fixture-runner/
  Cargo.toml
  src/main.rs
project-input/stt/
  README.md                # local placement/download/hash procedure
  models.example.json      # pathsを含まない配布manifest例
  licenses/                # 再配布可能と確認したlicense textのみ
```

予定interface（隣接task間で固定する契約）:

```rust
pub struct AudioFrame {
    pub samples: Box<[f32]>,
    pub sample_rate_hz: u32,
    pub captured_at: std::time::Instant,
    pub discontinuity: bool,
}

pub struct AudioDiagnosticsSnapshot {
    pub device_key: Option<String>,
    pub input_sample_rate_hz: Option<u32>,
    pub ring_capacity_samples: usize,
    pub ring_queued_samples: usize,
    pub dropped_samples_total: u64,
    pub discontinuities_total: u64,
    pub rms_dbfs: f32,
    pub noise_floor_dbfs: f32,
}

pub trait VadEngine: Send {
    fn accept_16khz(&mut self, samples: &[f32]) -> Result<Vec<VadObservation>, SttError>;
    fn reset(&mut self) -> Result<(), SttError>;
}

pub trait SpeechRecognizer: Send {
    fn recognize_16khz(&mut self, utterance: &[f32]) -> Result<Recognition, SttError>;
    fn reset(&mut self) -> Result<(), SttError>;
}

pub enum SttEvent {
    Level { rms_dbfs: f32, noise_floor_dbfs: f32 },
    VadChanged { speaking: bool, probability: f32 },
    UtteranceStarted { utterance_id: u64 },
    UtteranceEnded { utterance_id: u64, duration_ms: u64 },
    FinalTranscript { conversation_id: String, turn_id: String, text: String },
    Rejected { utterance_id: u64, reasons: Vec<RejectionReason> },
    Diagnostic(AudioDiagnosticsSnapshot),
    Unavailable { code: SttErrorCode },
}
```

`Box<[f32]>`はworker境界の所有bufferでありcallback内では生成しない。callbackは事前確保ringへscalar/slice pushし、workerがchunkを所有化する。

## 6. TDDタスク分解

### Task 2.1: Audio domain、sample変換、有界ring

**Files:** `crates/pw-audio/Cargo.toml`、`src/{lib,pcm,ring,metrics}.rs`、`tests/pipeline.rs`、root `Cargo.toml`。

**Consumes:** cpal callback由来のinterleaved `f32/i16/u16` sampleとchannel count。

**Produces:** lock-free producer/consumer、normalized mono `f32`、drop/discontinuity counters、`AudioDiagnosticsSnapshot`。

TDD順序:

1. `i16::MIN/0/MAX` と `u16::MIN/mid/MAX` の正規化、stereo mono mix、NaN/Inf clampの失敗testを書く。
2. 容量4へ6 sampleを投入すると4 sampleが順序通り読め、2 drop、次frameが`discontinuity=true`となるtestを書く。
3. 実装し、callback pathのsource scan testで `Mutex`、logging、allocation APIがcapture closureにないことをgate化する。
4. `cargo test -p pw-audio` とclippyを通す。

### Task 2.2: device列挙とcapture lifecycle

**Files:** `crates/pw-audio/src/{device,capture}.rs`、`tests/device_contract.rs`。

**Consumes:** `cpal::Host`を包む `AudioHost` test port。

**Produces:** `InputDeviceDescriptor`、supported config選択、`CaptureSession::{start,stop}`、`AudioDeviceEvent::{Disconnected,Reenumerated}`。

TDD順序:

1. default device不在、同名device複数、44.1/48 kHz、F32/I16/U16、切断errorのfake-host testを書く。
2. stable keyはbackend IDがあればID、なければhost/name/indexの複合keyとし、表示名と永続keyを分離する。
3. stopを2回呼んでも安全、session dropでstreamが停止するtestを通す。
4. real microphone smokeはignored testとし、通常CIから分離する。

### Task 2.3: 16 kHz resamplerと音響metrics

**Files:** `crates/pw-audio/src/{resample,worker}.rs`、`tests/resample_contract.rs`。

**Consumes:** discontinuity付きmono frames。

**Produces:** 16,000 Hz frames、RMS/noise floor/SNR、reset可能なworker。

TDD順序:

1. 48 kHzの1 kHz sine、44.1 kHz sine、silence、chunk境界、discontinuity resetをfixture生成式でtestする。
2. 10秒入力に対する出力sample数を理論値±resampler delay以内で検証する。
3. bufferを初期化時に確保し、処理loopのallocation count regression testを追加する。
4. 2時間相当を高速feedし、queueとowned buffer countが定常上限内であるtestを通す。

### Task 2.4: STT portsと発話境界state machine

**Files:** `crates/pw-stt-sherpa/Cargo.toml`、`src/{lib,ports,segmenter}.rs`、`tests/segmenter.rs`、root `Cargo.toml`。

**Consumes:** 16 kHz PCM、決定論的 `VadObservation` sequence。

**Produces:** start/end event、pre-roll、max utterance、reset、discontinuity handling。

TDD順序:

1. silence、short blip、normal speech、false start、two utterances、max duration、buffer discontinuityのtable testを書く。
2. threshold、min speech、min silence、pre-roll、max speechをvalidated configにする。
3. fake VADだけで全state transitionを通し、ONNXを通常testから除外する。
4. `cargo test -p pw-stt-sherpa segmenter` を通す。

### Task 2.5: transcript normalizationと複合filter

**Files:** `crates/pw-stt-sherpa/src/filter.rs`、`tests/filter.rs`。

**Consumes:** recognition text、duration、VAD probability、RMS/noise floor、previous transcript、TTS gate、discontinuity。

**Produces:** `AcceptedTranscript` または複数の `RejectionReason`。

TDD順序:

1. 空白正規化、空、音響タグだけ、短時間+低VAD、低SNR、重複、TTS loopback、buffer dropのmatrix testを書く。
2. 単一条件では棄却しない境界（短いが高VAD・高SNRの「はい」等）を明示testする。
3. TTS gateは再生中とcooldown終了時刻をmonotonic clock portでtestする。
4. user textそのものをdiagnostic/logへ含めないことをserialization testで確認する。

### Task 2.6: model manifestとofficial Rust adapter spike

**Files:** `crates/pw-stt-sherpa/src/{model_manifest,sherpa}.rs`、`tests/{model_manifest,adapter_contract}.rs`、`project-input/stt/{README.md,models.example.json,licenses/*}`。

**Consumes:** app data `models` root、expected SHA-256、Silero/Reazon file set。

**Produces:** validated `SherpaModelPaths`、`SherpaVadEngine`、`SherpaOfflineRecognizer`。

TDD順序:

1. missing file、path traversal、hash mismatch、version mismatch、license metadata欠落をtempdir fake filesでtestする。
2. `sherpa-onnx`のversionを固定し、Rust APIでVADとoffline transducerをconstructするcompile smokeを追加する。
3. `#[ignore = "requires verified local sherpa models"]` real-model testで公式test WAVの期待日本語、empty audio、reset/recreateを検証する。
4. Windows/macOS native libraryの取得物、hash、link mode、bundle filesをevidenceへ保存する。
5. Rust APIが要件を満たせない場合だけC API spike branchを作り、同じadapter contract testsで比較する。採否はmemory、RTF、shutdown、packaging evidenceで決める。

### Task 2.7: STT service、キャンセル、TTS gate、Tauri IPC

**Files:** `crates/pw-stt-sherpa/src/service.rs`、`pw-contracts`のSTT DTO、`apps/desktop/src-tauri/src/commands/stt.rs`、settings UIのdiagnostic component/tests。

**Consumes:** audio worker events、VAD/recognizer ports、conversation/turn ID、TTS playback state。

**Produces:** bounded event channel、start/stop/reinitialize/select-device command、typed `SttEventDto`。

TDD順序:

1. stale turn result破棄、stop後result破棄、TTS startでcapture-to-recognizer停止、cooldown後再開、recognizer errorで`STT_UNAVAILABLE`となるservice testを書く。
2. raw PCMをDTO/serde eventへ変換できないcompile-time architecture testを追加する。
3. diagnostics UIにdevice/config/ring drops/VAD/STT state/error codeを表示し、transcript/raw PCMをdiagnostic dumpへ入れないcomponent testを通す。
4. Capabilityはsettings/chatに必要なcommand/eventだけ追加し、negative capability testを通す。

### Task 2.8: fixture harnessとPhase 2受入証拠

**Files:** `tests/audio-fixtures/schema/fixture.schema.json`、各categoryの`*.expected.json`、`tools/audio-fixture-runner/*`、`docs/verification/phase2-stt.md`。

**Consumes:** WAV PCMと次のmetadata: source/license/recording condition、sample rate/channels、expected accept/reject、expected transcript alternatives、VAD boundaries tolerance、max RTF、max memory growth。

**Produces:** machine-readable JSON report、per-fixture verdict、2時間soak metrics。

TDD順序:

1. schema validatorを作り、metadata欠落、WAV mismatch、unknown rejection reasonを失敗させる。
2. synthetic fixture（silence、tones、noise、scripted fake recognition）でrunner自体を決定論的にtestする。
3. 権利確認済み実音声を各categoryへ配置し、SHA-256とlicenseを記録する。権利不明audioはcommitしない。
4. real model環境で無音10分 `accepted=0`、short-command/normal-conversationの期待候補一致、TTS loopback `accepted=0` をJSON evidence化する。
5. 2時間soakでRSS、ring occupancy、drop、worker/task countを一定間隔で記録する。合格条件は、warm-up後のRSS回帰傾きが正でないことを信頼区間で確認し、同時にqueue/task数が設定上限を超えないこと。単なる開始/終了2点比較にはしない。

## 7. Fixture schema最小契約

```json
{
  "schema_version": 1,
  "audio_sha256": "64 lowercase hex characters",
  "source": { "kind": "synthetic|recorded|upstream", "license": "SPDX-or-reviewed-id" },
  "recording": { "sample_rate_hz": 16000, "channels": 1, "environment": "anechoic|room|fan|keyboard|loopback" },
  "expected": {
    "decision": "accept|reject",
    "transcript_any_of": ["こんにちは"],
    "rejection_reasons_any_of": [],
    "utterance_count": 1,
    "boundary_tolerance_ms": 250
  }
}
```

silence/noise fixtureでは `transcript_any_of` を空配列、`decision=reject` とする。real recognitionの表記ゆれは無制限regexではなく、正規化後の有限候補で管理する。

## 8. Acceptance evidence matrix

| 要件 | 通常CI evidence | 実機・実モデル evidence |
|---|---|---|
| callback非ブロッキング | bounded ring unit test、source/architecture gate | callback duration histogram、drop counter |
| sample変換/16 kHz | generated sine/silence tests | device 44.1/48 kHz capture report |
| Silero発話境界 | fake VAD state-machine tests | category fixture boundary report |
| ReazonSpeech | fake recognizer contract | verified model + known WAV transcript |
| 無音10分送信0 | accelerated fake clock + silence PCM | 10-minute report `accepted=0` |
| TTS中停止 | fake clock/service test | loopback fixture and playback integration |
| 2時間memory安定 | accelerated ownership/queue bound test | RSS slope + queue/task time series |
| device切断準備 | fake host disconnect/re-enumerate | USB/Bluetooth disconnect manual evidence |
| 秘密情報/生PCM非公開 | DTO serialization/architecture tests | diagnostic export inspection |

## 9. 実装開始前gate

- Cargoで `cpal`、`ringbuf`、`rubato`、`sherpa-onnx` の正確な解決versionを固定し、Windows/macOSでcompile spikeを行う。
- sherpa native auto-downloadを観測し、URL、archive hash、展開file、license/NOTICE、static/shared link結果を保存する。
- SileroとReazonSpeech archiveをapp data `models`へ配置する手順を作り、全fileのSHA-256を確定する。
- fixture音声の作成・利用許諾を確定する。現状は実音声fixtureが0件なので、real-model acceptanceは素材取得前には達成扱いにしない。
- Phase 2完了判定ではmock greenを実モデル合格の代用にしない。一方、実モデル失敗をadapter/domain unit testへ混入させず、原因をモデル、native linking、audio pipeline、filterに分離して報告する。

