[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$trustedStatuses = @('ready', 'ready_without_voice', 'warmup_failed')
if ($env:PW_IRODORI_BOOTSTRAP_STATUS -notin $trustedStatuses) {
    exit 0
}

$appData = [Environment]::GetEnvironmentVariable('APPDATA')
if ([string]::IsNullOrWhiteSpace($appData)) {
    exit 0
}

$configDirectory = Join-Path $appData 'com.parallelworld.desktop\config'
$settingsPath = Join-Path $configDirectory 'tts.json'
if (Test-Path -LiteralPath $settingsPath) {
    exit 0
}

[void] [IO.Directory]::CreateDirectory($configDirectory)
$settings = [ordered]@{
    schema_version = 1
    enabled = $true
    base_url = 'http://127.0.0.1:8088'
    engine = 'irodori'
    voice_id = 'none'
    irodori_lora_adapter = ''
    style_id = 0
    volume = 1.0
    speed = 1.0
}
$temporaryPath = "$settingsPath.$([Guid]::NewGuid().ToString('N')).tmp"
try {
    [IO.File]::WriteAllText(
        $temporaryPath,
        ($settings | ConvertTo-Json),
        [Text.UTF8Encoding]::new($false)
    )
    Move-Item -LiteralPath $temporaryPath -Destination $settingsPath
    Write-Host '[TTS] Configured Irodori as the default engine for this new profile.' -ForegroundColor Green
} finally {
    Remove-Item -LiteralPath $temporaryPath -Force -ErrorAction SilentlyContinue
}
