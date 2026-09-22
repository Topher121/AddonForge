# AddonForge build + deliver (the owner's studio convention, cloned from MechaBud's build-apk.ps1)
#
#   .\build.ps1                       # bump build number, test, release build, write build\latest.json
#   .\build.ps1 -SetVersion 0.2.0     # also change the semver (tauri.conf.json, Cargo.toml, package.json)
#   .\build.ps1 -Note "what changed"  # release note for latest.json + the GitHub release (omitted = keep previous)
#   .\build.ps1 -SkipTests
#   .\build.ps1 -Release              # also publish the GitHub release (tag v<ver>-b<n>) with the exe + latest.json
#   .\build.ps1 -DeliverOnly -Release # no bump/build: publish the newest build\*.exe as it is
#
# Output: build\AddonForge-v<ver>-b<build>.exe (old builds archived, keep 5)
# Distribution: GitHub Releases only (the owner 2026-09-22: not on the PhoneApps page, it is a Windows tool).
# Users get new builds through the app's own self-update banner (reads latest.json from the latest release).
param([string]$SetVersion = "", [string]$Note = "", [switch]$SkipTests, [switch]$DeliverOnly, [switch]$Release)
$ErrorActionPreference = "Stop"
$root  = $PSScriptRoot
$tauri = Join-Path $root "src-tauri"
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"

function WriteUtf8NoBom($path, $text) { [IO.File]::WriteAllText($path, $text, (New-Object Text.UTF8Encoding $false)) }

# ---- version + build number ---------------------------------------------
$confPath = Join-Path $tauri "tauri.conf.json"
$conf = Get-Content $confPath -Raw | ConvertFrom-Json
$version = if ($SetVersion) { $SetVersion } else { $conf.version }
if ($version -notmatch '^\d+\.\d+\.\d+$') { throw "version must be x.y.z (got '$version')" }

$buildFile = Join-Path $root "build\.buildnum"
$build = 0
if (Test-Path $buildFile) { $build = [int](Get-Content $buildFile -Raw).Trim() }
if (-not $DeliverOnly) {
    $build++
    New-Item -ItemType Directory -Force (Join-Path $root "build") | Out-Null
    WriteUtf8NoBom $buildFile "$build"
}

if ($SetVersion) {
    foreach ($f in @($confPath, (Join-Path $root "package.json"))) {
        $t = Get-Content $f -Raw
        $t = $t -replace '"version":\s*"[^"]+"', "`"version`": `"$version`""
        WriteUtf8NoBom $f $t
    }
    $cargo = Join-Path $tauri "Cargo.toml"
    $t = Get-Content $cargo -Raw
    $t = $t -replace '(?m)^version = "[^"]+"', "version = `"$version`""
    WriteUtf8NoBom $cargo $t
}
Write-Host "AddonForge v$version build $build" -ForegroundColor Cyan

# ---- verify + build -----------------------------------------------------
if ($DeliverOnly) {
    $exe = Get-ChildItem (Join-Path $root "build") -Filter "AddonForge-v*.exe" | Sort-Object LastWriteTime -Descending | Select-Object -First 1
    if (-not $exe) { throw "nothing in build\ to deliver" }
    $name = $exe.Name; $out = $exe.FullName
    $setupName = ($name -replace "^AddonForge-", "AddonForge-Setup-")
    if ($name -match '-v(\d+\.\d+\.\d+)-b(\d+)\.exe$') { $version = $Matches[1]; $build = [int]$Matches[2] }
    $sizeMB = [math]::Round($exe.Length / 1MB, 1)
    Write-Host "reusing $name ($sizeMB MB)" -ForegroundColor Cyan
} else {
Push-Location $tauri
try {
    if (-not $SkipTests) {
        cargo test | Select-Object -Last 3
        if ($LASTEXITCODE -ne 0) { throw "cargo test failed - not building" }
    }
} finally { Pop-Location }

Push-Location $root
try {
    $env:ADDONFORGE_BUILD = "$build"   # baked into the exe (selfupdate::build)
    npx tauri build                    # exe + NSIS installer (bundle targets in tauri.conf.json)
    if ($LASTEXITCODE -ne 0) { throw "tauri build failed" }
} finally { Pop-Location }

$exe = Join-Path $tauri "target\release\addonforge.exe"
if (-not (Test-Path $exe)) { throw "expected $exe" }

# ---- archive old builds, keep 5 ------------------------------------------
$name = "AddonForge-v$version-b$build.exe"
$out  = Join-Path $root "build\$name"
Copy-Item $exe $out -Force
Get-ChildItem (Join-Path $root "build") -Filter "AddonForge-v*.exe" |
    Sort-Object LastWriteTime -Descending | Select-Object -Skip 5 | Remove-Item -Force
$sizeMB = [math]::Round((Get-Item $out).Length / 1MB, 1)
Write-Host "built $out ($sizeMB MB)" -ForegroundColor Green
$setupSrc = Get-ChildItem (Join-Path $tauri "target\release\bundle\nsis") -Filter "*-setup.exe" | Sort-Object LastWriteTime -Descending | Select-Object -First 1
if ($setupSrc) {
    $setupName = "AddonForge-Setup-v$version-b$build.exe"
    Copy-Item $setupSrc.FullName (Join-Path $root "build\$setupName") -Force
    Get-ChildItem (Join-Path $root "build") -Filter "AddonForge-Setup-*.exe" | Sort-Object LastWriteTime -Descending | Select-Object -Skip 5 | Remove-Item -Force
    Write-Host "built installer build\$setupName" -ForegroundColor Green
}
}

# ---- release note ----------------------------------------------------------
# -Note "..." sets the note used for latest.json + the GitHub release; omitted = keep the previous one.
$notePath = Join-Path $root "build\.note"
$whatsNew = $Note
if ($whatsNew -eq "" -and (Test-Path $notePath)) { $whatsNew = (Get-Content $notePath -Raw).Trim() }
WriteUtf8NoBom $notePath $whatsNew

# ---- latest.json (self-update manifest) ----------------------------------
# The app fetches https://github.com/Topher121/AddonForge/releases/latest/download/latest.json
$tag = "v$version-b$build"
$latest = [ordered]@{
    version  = $version
    build    = $build
    filename = $name
    url      = "https://github.com/Topher121/AddonForge/releases/download/$tag/$name"
    size     = (Get-Item $out).Length
    notes    = $whatsNew
}
$latestPath = Join-Path $root "build\latest.json"
WriteUtf8NoBom $latestPath ($latest | ConvertTo-Json)

if ($Release) {
    Push-Location $root
    try {
        $assets = @($out, $latestPath)
        $setupPath = Join-Path $root "build\$setupName"
        if ($setupName -and (Test-Path $setupPath)) { $assets += $setupPath }
        gh release create $tag @assets --title "AddonForge $tag" --notes "$whatsNew"
        if ($LASTEXITCODE -ne 0) { throw "gh release create failed" }
        Write-Host "published GitHub release $tag" -ForegroundColor Green
    } finally { Pop-Location }
}
