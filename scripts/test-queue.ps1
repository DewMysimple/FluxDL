$ErrorActionPreference = 'Stop'
$workspace = Split-Path -Parent $PSScriptRoot
$fixtureRoot = Join-Path $workspace 'src-tauri\target\queue-fixture'
New-Item -ItemType Directory -Force -Path $fixtureRoot | Out-Null
$fixtureExe = Join-Path $fixtureRoot 'fluxdl-test-engine.exe'
$source = Join-Path $workspace 'src-tauri\tests\fixtures\queue_engine.rs'
& rustc $source --edition 2021 -o $fixtureExe
if ($LASTEXITCODE -ne 0) { throw 'Fixture engine compilation failed.' }
$previous = $env:FLUXDL_TEST_ENGINE
try {
    $env:FLUXDL_TEST_ENGINE = $fixtureExe
    Push-Location (Join-Path $workspace 'src-tauri')
    try {
        & cargo test --lib manager::tests::worker_lifecycle_fixture -- --ignored --nocapture
        if ($LASTEXITCODE -ne 0) { throw 'Queue lifecycle fixture test failed.' }
    } finally { Pop-Location }
} finally { $env:FLUXDL_TEST_ENGINE = $previous }
