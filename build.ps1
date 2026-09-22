# AddonForge build + deliver (the owner's studio convention, cloned from MechaBud's build-apk.ps1)
#
#   .\build.ps1                       # bump build number, release build, deliver to PhoneApps
#   .\build.ps1 -SetVersion 0.2.0     # also change the semver (tauri.conf.json, Cargo.toml, package.json)
#   .\build.ps1 -Note "what testers should try"   # sets meta.json whats_new (omitted = keep previous)
#   .\build.ps1 -SkipTests
#
# Output: build\AddonForge-v<ver>-b<build>.exe (old builds archived, keep 5)
# Delivered: C:\Users\the owner\Desktop\PhoneApps\addonforge\ (sole exe + regenerated index.html + meta.json)

param([string]$SetVersion = "", [string]$Note = "", [switch]$SkipTests)
$ErrorActionPreference = "Stop"
$root  = $PSScriptRoot
$tauri = Join-Path $root "src-tauri"
$phone = "C:\Users\the owner\Desktop\PhoneApps\addonforge"
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
$build++
New-Item -ItemType Directory -Force (Join-Path $root "build") | Out-Null
WriteUtf8NoBom $buildFile "$build"

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
Push-Location $tauri
try {
    if (-not $SkipTests) {
        cargo test | Select-Object -Last 3
        if ($LASTEXITCODE -ne 0) { throw "cargo test failed - not building" }
    }
} finally { Pop-Location }

Push-Location $root
try {
    npx tauri build --no-bundle
    if ($LASTEXITCODE -ne 0) { throw "tauri build failed" }
} finally { Pop-Location }

$exe = Join-Path $tauri "target\release\addonforge.exe"
if (-not (Test-Path $exe)) { throw "expected $exe" }

# ---- archive old builds, keep 5 ------------------------------------------
$name = "AddonForge-v$version-b$build.exe"
$out  = Join-Path $root "build\$name"
Copy-Item $exe $out -Force
Get-ChildItem (Join-Path $root "build") -Filter "AddonForge-*.exe" |
    Sort-Object LastWriteTime -Descending | Select-Object -Skip 5 | Remove-Item -Force
$sizeMB = [math]::Round((Get-Item $out).Length / 1MB, 1)
Write-Host "built $out ($sizeMB MB)" -ForegroundColor Green

# ---- deliver to PhoneApps ------------------------------------------------
New-Item -ItemType Directory -Force $phone | Out-Null
Get-ChildItem $phone -Filter "*.exe" | Remove-Item -Force
Copy-Item $out (Join-Path $phone $name) -Force
Copy-Item (Join-Path $root "icon.svg") (Join-Path $phone "icon.svg") -Force

$html = @"
<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>AddonForge</title>
<style>
*{box-sizing:border-box}body{margin:0;min-height:100vh;display:flex;align-items:center;justify-content:center;font:16px/1.5 system-ui,sans-serif;color:#e8f1fa;background:radial-gradient(circle at 50% 0%,#2c3c54,#18243a 45%,#101a2c);padding:24px}
.card{width:100%;max-width:420px;text-align:center;background:linear-gradient(#34465e,#202e44);border:1px solid #46608a;border-top-color:#a6c4e0;border-radius:26px;padding:34px 26px;box-shadow:0 20px 50px rgba(0,0,0,.45)}
.tile{width:110px;height:110px;margin:0 auto 18px;border-radius:26px;background:radial-gradient(circle at 50% 30%,#1e3752,#12213a);border:1px solid #46608a;box-shadow:0 0 30px rgba(85,220,242,.25);display:flex;align-items:center;justify-content:center}
.tile img{width:84px;height:84px}h1{margin:0;font-size:30px}.ver{color:#9db0c8;font-size:13px;margin:6px 0 20px}
.btn{display:block;background:linear-gradient(#e6fcff,#55dcf2,#189ac2);color:#0a3644;font-weight:800;text-decoration:none;padding:15px 20px;border-radius:999px;border-bottom:4px solid #0d5e75;font-size:17px}
.hint{color:#9db0c8;font-size:13px;margin:16px 0 0}.back{display:inline-block;margin-top:22px;color:#55dcf2;text-decoration:none;font-size:14px}
</style></head><body><div class="card">
<div class="tile"><img src="icon.svg" alt="AddonForge"></div>
<h1>AddonForge</h1>
<div class="ver">v$version &middot; build $build &middot; $sizeMB MB &middot; Windows 10/11</div>
<a class="btn" href="$name">Download for Windows &darr;</a>
<p class="hint">WoW: Forever addon manager. Portable exe, no installer. Windows may warn about an unknown publisher the first time: More info &rarr; Run anyway.</p>
<a class="back" href="/">&larr; all apps</a>
</div></body></html>
"@
WriteUtf8NoBom (Join-Path $phone "index.html") $html

# meta.json for the root launcher card (the owner 2026-07-31)
$metaPath = Join-Path $phone "meta.json"
$whatsNew = $Note
if ($whatsNew -eq "" -and (Test-Path $metaPath)) {
    try { $prev = Get-Content $metaPath -Raw | ConvertFrom-Json; $whatsNew = $prev.whats_new } catch {}
}
$meta = [ordered]@{
    version   = $version
    build     = $build
    updated   = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
    whats_new = $whatsNew
}
WriteUtf8NoBom $metaPath ($meta | ConvertTo-Json)
Write-Host "delivered to $phone  ->  http://192.168.1.64:8080/addonforge/" -ForegroundColor Green
