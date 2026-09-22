# Rebuild the Windows/app icons from the editable original artwork.
$ErrorActionPreference = "Stop"
$forgeRoot = Split-Path -Parent $PSScriptRoot
$forgeOutput = Join-Path $forgeRoot 'build\icon-render'
$forgeCli = Join-Path $forgeRoot 'node_modules\.bin\tauri.cmd'
& $forgeCli icon (Join-Path $forgeRoot 'icon.svg') --output $forgeOutput
if ($LASTEXITCODE -ne 0) { throw 'Icon rendering failed' }
# Copy only desktop assets. The CLI also generates mobile folders we do not use.
Get-ChildItem -LiteralPath $forgeOutput -File | Where-Object {
    $_.Extension -in @('.png', '.ico', '.icns')
} | ForEach-Object {
    Copy-Item -LiteralPath $_.FullName -Destination (Join-Path $forgeRoot 'src-tauri\icons') -Force
}
Copy-Item -LiteralPath (Join-Path $forgeRoot 'icon.svg') -Destination (Join-Path $forgeRoot 'ui\icon.svg') -Force
Write-Host 'Addon Forge desktop icons and header artwork updated.'
