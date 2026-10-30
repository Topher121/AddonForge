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
param([string]$SetVersion = "", [string]$Note = "", [switch]$SkipTests, [switch]$DeliverOnly, [switch]$Release, [string]$ConfirmTag = "")
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
    # Security gate (studio rule, maintainer 2026-10-01): refuses to build if any protection is weakened.
    node (Join-Path $root "scripts\security-check.mjs")
    if ($LASTEXITCODE -ne 0) { throw "SECURITY CHECK FAILED - not building. Fix the config; never weaken a protection to get a build out." }
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

# ---- notes history (build\.history.json) ----------------------------------
# Newest first, last 8 builds. The app shows every note between the build a
# user last ran and the one they just got, so skipped builds are not lost.
$historyPath = Join-Path $root "build\.history.json"
$history = @()
# PS 5.1 hands a JSON array back as ONE object; ForEach-Object unrolls it.
if (Test-Path $historyPath) { $history = @((Get-Content $historyPath -Raw | ConvertFrom-Json) | ForEach-Object { $_ }) }
$history = @($history | Where-Object { $_.build -ne $build })
$history = @([pscustomobject]@{ version = $version; build = $build; notes = $whatsNew }) + $history
$history = @($history | Sort-Object -Property build -Descending | Select-Object -First 8)
WriteUtf8NoBom $historyPath (ConvertTo-Json @($history) -Depth 3)

# ---- latest.json (self-update manifest) ----------------------------------
# The app fetches https://github.com/Topher121/AddonForge/releases/latest/download/latest.json
$tag = "v$version-b$build"
$latest = [ordered]@{
    version  = $version
    build    = $build
    filename = $name
    url      = "https://github.com/Topher121/AddonForge/releases/download/$tag/$name"
    size     = (Get-Item $out).Length
    sha256   = (Get-FileHash $out -Algorithm SHA256).Hash.ToLower()
    notes    = $whatsNew
    history  = @($history)
}
$latestPath = Join-Path $root "build\latest.json"
WriteUtf8NoBom $latestPath ($latest | ConvertTo-Json -Depth 4)

# Sign the manifest with the offline release key (kept outside the repo; see
# scripts/sign-release.js). A release without a signature would leave every
# updated install refusing updates, so -Release stops without one.
$sigPath = "$latestPath.sig"
if ($env:ADDONFORGE_RELEASE_KEY) {
    & node (Join-Path $root "scripts\sign-release.js") $latestPath
    if ($LASTEXITCODE -ne 0) { throw "signing latest.json failed" }
    & node (Join-Path $root "scripts\sign-release.js") --verify $latestPath
    if ($LASTEXITCODE -ne 0) { throw "signature check failed after signing" }
} elseif ($Release) {
    throw "Not publishing: set ADDONFORGE_RELEASE_KEY to the release key path so latest.json is signed."
}

# ---- Defender gate: never publish a file Defender flags -------------------
# Scans the exact exe (and installer) about to be released. Read-only with
# respect to Defender: no settings, exclusions or restores. Any detection on
# these files stops the build before anything reaches GitHub.
$gateStart = Get-Date
$gateFiles = @($out)
if ($setupName -and (Test-Path (Join-Path $root "build\$setupName"))) { $gateFiles += (Join-Path $root "build\$setupName") }
foreach ($f in $gateFiles) {
    try { Start-MpScan -ScanType CustomScan -ScanPath $f -ErrorAction Stop }
    catch { throw "Defender scan could not run on $f ($($_.Exception.Message)). Not publishing." }
}
Start-Sleep -Seconds 3
# No time filter: a repeat detection of a known threat keeps its original
# InitialDetectionTime, so any detection that names these files blocks the
# release. Fails closed; the file names carry the version, so old history
# cannot block a new build by accident.
$gateHits = @(Get-MpThreatDetection | Where-Object {
    $r = $_.Resources -join " "
    $gateFiles | Where-Object { $r -like "*$(Split-Path $_ -Leaf)*" }
})
if ($gateHits.Count -gt 0) {
    $names = $gateHits | ForEach-Object { try { (Get-MpThreat -ThreatID $_.ThreatID).ThreatName } catch { "threat $($_.ThreatID)" } }
    throw "DEFENDER FLAGGED the release build: $($names -join ', '). Not publishing. Report the exact name to the maintainer; do not add exclusions."
}
Write-Host "Defender gate: clean ($($gateFiles.Count) file(s))" -ForegroundColor Green

# Sign-off: publishing needs the maintainer to type the exact tag. Testing on the
# exact file and the Defender gate are required first, and this check does not
# replace them. Usage: build.ps1 -Release -ConfirmTag <tag>
if ($Release -and $ConfirmTag -ne $tag) {
    throw "Not publishing $($tag): pass -ConfirmTag $($tag) once the exact file has been tried and the maintainer has signed off."
}
if ($Release) {
    Push-Location $root
    try {
        $assets = @($out, $latestPath, $sigPath)
        $setupPath = Join-Path $root "build\$setupName"
        if ($setupName -and (Test-Path $setupPath)) { $assets += $setupPath }
        gh release create $tag @assets --title "AddonForge $tag" --notes "$whatsNew"
        if ($LASTEXITCODE -ne 0) { throw "gh release create failed" }
        Write-Host "published GitHub release $tag" -ForegroundColor Green
    } finally { Pop-Location }
}
