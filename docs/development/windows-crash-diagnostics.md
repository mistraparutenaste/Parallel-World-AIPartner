# Windows crash diagnostics

Parallel World は既定で `%APPDATA%` 配下の `crashes` に、秘密情報とプロンプト本文を除外したJSON診断を保存します。クラッシュ診断とログはそれぞれ最大20件・合計20 MiBまでです。Settingsの「診断」から、明示したファイルへ安全にエクスポートできます。

## WER LocalDumps（任意）

Windows Error Reporting の `LocalDumps` は、Rustの診断だけでは原因を特定できない場合に限って手動で有効化してください。プロセスダンプには会話、認証情報、音声バッファなどの機密データが含まれる可能性があります。

- 保存先はユーザー本人だけが読み取れるACLの専用フォルダーにします。
- `DumpCount` を小さく設定し、調査終了後はダンプとレジストリ設定を削除します。
- ダンプを第三者へ渡す前に、組織の情報管理手順に従います。
- 通常運用や自動収集では有効化しません。

Microsoftの[WER Settings](https://learn.microsoft.com/en-us/windows/win32/wer/wer-settings)と[クラッシュ調査手順](https://learn.microsoft.com/en-us/troubleshoot/windows-server/performance/troubleshoot-application-service-crashing-behavior)を参照し、実行ファイル単位のキー、`DumpCount`、`DumpFolder`を明示してください。フルダンプは特に機密性が高いため、最小限の診断で不足する場合だけ使用します。

管理者PowerShellで、専用フォルダーと実行ファイル単位の設定を作成します（パスは必要に応じて変更してください）。

```powershell
$dump = 'C:\ParallelWorldDumps'
New-Item -ItemType Directory -Force -Path $dump
icacls $dump /inheritance:r /grant:r "${env:USERNAME}:(OI)(CI)F" 'SYSTEM:(OI)(CI)F'
$key = 'HKLM:\SOFTWARE\Microsoft\Windows\Windows Error Reporting\LocalDumps\parallel-world-desktop.exe'
New-Item -Force -Path $key
New-ItemProperty -Force -Path $key -Name DumpFolder -PropertyType ExpandString -Value $dump
New-ItemProperty -Force -Path $key -Name DumpCount -PropertyType DWord -Value 3
New-ItemProperty -Force -Path $key -Name DumpType -PropertyType DWord -Value 1
```

`DumpType=1`（mini dump）を既定とします。`DumpType=2`（full dump）は、mini dumpで不足し、機密データを含む可能性と保管責任を明示的に受け入れた場合だけ一時的に指定してください。

調査後は収集を無効化し、ダンプを削除します。

```powershell
$key = 'HKLM:\SOFTWARE\Microsoft\Windows\Windows Error Reporting\LocalDumps\parallel-world-desktop.exe'
Remove-Item -Recurse -Force -Path $key
Remove-Item -Recurse -Force -Path 'C:\ParallelWorldDumps'
```
