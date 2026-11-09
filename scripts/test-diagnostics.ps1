param([ValidateRange(1,100)][int]$Samples = 5, [switch]$BenchmarkOnly)
$ErrorActionPreference = 'Stop'
$workspace = Split-Path -Parent $PSScriptRoot
$fixtureRoot = Join-Path $workspace ('src-tauri\target\diagnostics-fixture\' + [Guid]::NewGuid().ToString())
New-Item -ItemType Directory -Force -Path $fixtureRoot | Out-Null
$fixtureExe = Join-Path $fixtureRoot 'diagnostic-engine.exe'
Push-Location $workspace
try {
    if (-not $BenchmarkOnly) {
        foreach ($filter in @('diagnostics', 'trusted_preview', 'first_actual_bytes', 'download_markers')) {
            & cargo test --manifest-path src-tauri/Cargo.toml --lib $filter
            if ($LASTEXITCODE -ne 0) { throw "Diagnostic unit tests failed: $filter" }
        }
    }
    & rustc src-tauri/tests/fixtures/diagnostic_engine.rs --edition 2021 -o $fixtureExe
    if ($LASTEXITCODE -ne 0) { throw 'Diagnostic fixture compilation failed.' }
    & cargo run --manifest-path src-tauri/Cargo.toml --example diagnostics_bench -- $fixtureExe artifacts/diagnostics $Samples
    if ($LASTEXITCODE -ne 0) { throw 'Production pipeline diagnostic fixture failed.' }
} finally { Pop-Location }
