# 静止画キャラクタープロファイル

このディレクトリには、静止画キャラクターの `character.json` 例だけを置く。実際のキャラクター画像は作者・権利者の利用条件を確認し、app dataへ手動で配置する。画像をリポジトリや配布bundleへ追加しない。

## 配置

Windowsでの例:

```text
%APPDATA%\com.parallelworld.desktop\characters\epsilon-static\
  character.json
  expressions\
    neutral.png
    happy.webp
```

`example-character.json` を `character.json` としてコピーし、`file` をプロファイルディレクトリからの相対パスにする。絶対パス、`..`、charactersルート外へ抜けるsymlink/junctionは拒否される。

## manifest契約

- ディスク上の `character.json` は `schema_version: 1`。IPCの `CharacterManifestDto` は別契約で `schema_version: 2`。
- `id`、`display_name`、表情名は空にできず、128 Unicode scalar values以下。IDと表情名は重複不可。
- `renderer.kind` は `static_image`。`default_expression` は `expressions` に存在する名前を指定する。
- `file` は透明PNGまたは非アニメーションWebP。拡張子だけではなく実データをdecodeして検証する。
- 全画像はalpha channelを持ち、0より大きい同一pixel寸法・同一位置合わせにする。
- 上限は1プロファイル32表情、1辺4096 px、1ファイル32 MiB、全表情のdecoded RGBA合計256 MiB。
- animated WebP、GIF、動画、破損画像、偽装拡張子、寸法不一致、unknown JSON fieldはプロファイル全体を無効にする。

静止画は画像全体を表情ごとに切り替える。口パク、部位レイヤー、モーションは行わない。会話の読み上げが実際に開始した時、同じ `turn_id` につき1回だけ300 ms・最大12 CSS px跳ねる。OSの「視差効果を減らす」が有効な場合は跳ねない。

## 選択と設定

設定は `%APPDATA%\com.parallelworld.desktop\config\character-settings.json`（schema version 1）に保存される。

```json
{
  "schema_version": 1,
  "active_character_id": "epsilon-static",
  "expression_idle_timeout_seconds": 20
}
```

- `active_character_id` がある場合は完全一致するIDだけを読み込む。見つからなければ `active_character_unavailable` となり、別キャラクターへ自動切替しない。
- `legacy-live2d` は従来Live2D探索用の仮想IDとして予約されているため、明示プロファイルの `id` には使用しない。
- 明示プロファイルが1件だけでID設定がない場合は、その1件を使用し、timeout設定を維持したままIDをatomic保存する。後から2件目を追加しても、この保存済みIDを完全一致で選び続ける。複数件でID設定がない場合は `selection_required`。
- recursive `*.model3.json` の従来Live2D探索は、`characters/*/character.json` が1件も存在しない場合だけ使用する。壊れた明示プロファイルからLive2Dへfallbackしない。
- 複数キャラクター一覧・選択UIは将来対応であり、現時点では対象外。
- 表情の自動復帰は既定20秒、10〜600秒、`null` は「戻さない」。Settingsの選択肢は戻さない、10/20/30秒、1/2/5/10分。
- listening、transcribing、thinking、speaking、interrupting中と音声再生中は復帰timerを停止する。発話開始・終了・停止・cancelや有効な表情変更をactivityとして扱う。

設定ファイルが欠落・破損・範囲外の場合、timeoutは既定20秒へ戻る。プロファイルの `missing_asset`、`invalid_manifest`、`invalid_image`、`selection_required`、`active_character_unavailable` は設定修正が必要な恒久エラーで、character surfaceを隠して通常chatを維持する。修正後はSettingsの再読み込みを使う。一時的なasset読取やrenderer起動失敗だけが有界retryの対象になる。

## 例の構文確認

```powershell
$manifest = Get-Content -Raw -Encoding utf8 project-input/static-character/example-character.json | ConvertFrom-Json
if ($manifest.renderer.kind -ne 'static_image') { throw 'renderer.kind must be static_image' }
```
