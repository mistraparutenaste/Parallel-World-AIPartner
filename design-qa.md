# 会話中心UI Design QA

- source visual truth path: `E:\app\parallel-world\docs\superpowers\specs\assets\2026-07-17-conversation-first-ui-dark-theme-selected.png`
- secondary source path: `E:\app\parallel-world\docs\superpowers\specs\assets\2026-07-16-conversation-first-ui-category-selector.png`
- implementation screenshot path: `E:\app\parallel-world\output\playwright\conversation-first-ui\settings-dark-1486x1058-final.png`
- full comparison path: `E:\app\parallel-world\output\playwright\conversation-first-ui\comparison-dark-final.png`
- focused comparison path: `E:\app\parallel-world\output\playwright\conversation-first-ui\comparison-dark-crown-final.png`
- approved close-flow mock path: `C:\Users\deele\.codex\visualizations\2026\07\16\019f6ab4-0a18-7b42-8f5b-68a45ee50dea\settings-close-flow.html`
- focused-screen screenshot path: `C:\Users\deele\AppData\Local\Temp\parallel-world-settings-1280x720.png`
- focused-screen mobile screenshot path: `C:\Users\deele\AppData\Local\Temp\parallel-world-settings-320x844.png`
- focused-screen comparison path: `C:\Users\deele\AppData\Local\Temp\parallel-world-settings-comparison.png`
- target viewport: 1486 × 1058
- additional target viewports: 640 × 900、320 × 844
- target state: `設定 > 表示`、ライト／ダーク、カテゴリーの通常／ホバー／フォーカス／選択状態

## Capture method

初回比較ではインアプリBrowserの実行ツールが公開されていなかったため、ユーザー承認済みのPlaywright CLIへ切り替えた。Tauri IPCを決定論的なローカルモックへ置き換え、Viteの実画面を同一viewportで操作・撮影した。モック値は表示確認用に限定し、製品コードへは含めていない。

2026-07-17の共通クローズ構造の再検証ではインアプリBrowserを使用した。1280 × 720と320 × 844で実画面を操作し、DOM、フォーカス、スクリーンショット、consoleのerror／warnを確認した。通常ブラウザではTauri IPCが利用できないため、画面内に読込失敗文が表示されるが、対象の開閉挙動とレイアウト検証には影響しない。

## Full-view comparison evidence

採用済み画像と実装画面を同じ1486 × 1058で横に並べ、次を確認した。

- 上段4個、下段3個で接続するハート・クラウンの寸法と接続関係。
- 左下の3個が接続するハート形メニューと、右下の独立したチャットひし形。
- 深い濃紺、細い明るいシアン、面を浮かせないフラットな構成。
- 会話背景へ一様なヴェールを掛け、カテゴリーと設定本文を前景へ出す階層。
- 送信ボタンのない下部入力線。

採用画像は設計説明用の会話内容を背景に置いている。一方、実装画面では選択中の「表示」設定を操作できる本文へ置き換え、現在地に合わせて左下の「設定」を塗り状態とした。この差は機能状態を正しく伝えるための意図的な差分である。

共通クローズ構造の比較では、採用済みダーク画像と現在の`設定 > 表示`を一枚の比較画像へ並べた。元画像は1486 × 1058、インアプリBrowserの撮影は1280 × 720のため、元画像をレターボックスで全景保持して比較した。次の差は、ユーザー承認済みモックによる意図的な更新である。

- 設定、性格、会話を開いた後は下部ハートとチャットひし形を表示しない。
- 3画面共通で右上へ「×」を置き、チャットへ戻る唯一の導線とする。
- 下部ナビゲーション用の余白を解放し、設定本文を画面下まで使用する。

この意図的差分以外では、深い濃紺、細いシアン、7個のハート・クラウン、選択中のシアン面、フラットな本文区切りを維持している。

## Focused region comparison evidence

ハート・クラウンを同じ矩形で切り出して比較した。最終パスでは次を満たしている。

- クラウン全幅と高さは基準画像に近い。
- 「キャラクター」は`キャラ / クター`の均衡した2行表示になっている。
- ひし形同士の辺が離れず、選択中の「表示」だけがシアン面になる。
- 非選択、ホバー、フォーカス、押下は塗りと輪郭の両方で区別できる。

## Automated interaction evidence

Playwrightで次の主要経路を操作した。

1. チャットから設定へ移動する。
2. カテゴリーのホバーとキーボードフォーカスを確認する。
3. ライト／ダークを切り替える。
4. 「会話」で頻度スライダーを変更し、自然文と保存値が更新されることを確認する。
5. 「性格」でサディズム、強いダーク表現、セーフワード、再開操作を確認する。
6. チャットで`SafewordTriggeredEvent`を受け、停止通知と再開導線を確認する。
7. 640 × 900と320 × 844へ変更し、横スクロールや操作不能がないことを確認する。
8. 設定、性格、会話を順に開き、各画面で下部ナビゲーションがDOMから外れることを確認する。
9. 各画面の「×」が44 × 44pxで、開いた直後にフォーカスされることを確認する。
10. 「×」で閉じ、チャット画面、下部ナビゲーション、「チャット」ひし形のフォーカスが復元することを確認する。

最終実行では`consoleErrors: []`、`pageErrors: []`であり、頻度変更も成功した。共通クローズ構造の再検証でもBrowserのerror／warnログは空だった。

## Findings and comparison history

### Pass 1

- [P1] デスクトップのクラウンが基準画像より小さい。
- [P1] 下部入力線が高く、左下ハートと右下ひし形も小さい。
- [P2] 狭幅時の背景会話が前景に近く、設定との階層が弱い。
- [P2] 「キャラクター」の改行が不均衡。

対応:

- デスクトップ時のクラウン幅、高さ、ひし形寸法を拡大した。
- 会話レイヤーの下余白を詰め、入力線を画面下端へ近づけた。
- デスクトップの下部ハート寸法を基準画像へ近づけた。
- 狭幅時の背景不透明度を下げた。
- 「キャラクター」を視覚上だけ明示的な2行へ分け、アクセシブルネームは維持した。

### Pass 2

- 同一viewportで再撮影し、全景とクラウン切り出しを比較した。
- 実装対象範囲にP0／P1／P2の未解消差分はない。
- 320px、640px、ライト、ダーク、ホバー、フォーカス、セーフワード通知を再確認した。

### Pass 3: 共通クローズ構造

- 設定、性格、会話の3画面で`navigationCount: 0`を確認した。
- 3画面のダイアログ名と「閉じる」ボタン名が画面ごとに一致した。
- 開いた直後は「×」、閉じた直後は「チャット」ひし形へフォーカスが移った。
- 320 × 844で「×」は44 × 44px、設定画面の`scrollWidth`と`clientWidth`は320pxで一致し、横方向の欠けはなかった。
- 1280 × 720と320 × 844のスクリーンショットで、下部ナビゲーションの残像、本文との重なり、閉じる操作の欠けはなかった。
- 実装対象範囲にP0／P1／P2の未解消差分はない。

## Implementation checklist

- [x] 1486 × 1058の`設定 > 表示`をライト／ダークで撮影する。
- [x] ハート・クラウンの通常／ホバー／フォーカス／選択を撮影する。
- [x] 左下ハートと右下チャットの接続、文字表示、クリック領域を確認する。
- [x] 640pxと320pxで欠け、重なり、横スクロールがないことを確認する。
- [x] 会話設定、性格、セーフワードの主要操作を確認する。
- [x] コンソールエラーとページエラーがないことを確認する。
- [x] P0／P1／P2差分を修正し、同条件で再撮影する。
- [x] 設定、性格、会話で下部ナビゲーションを隠し、共通の「×」からチャットへ戻れることを確認する。
- [x] 1280 × 720と320 × 844で「×」の操作領域、フォーカス、横方向の欠けを確認する。

## Intentional remaining boundary

エピソードの自動分割、左側7件の会話の足跡、最近使用20件、アーカイブ、再開はPhase Bへ残している。今回の空き領域は未完成UIを偽装せず空のままにしている。

final result: passed
