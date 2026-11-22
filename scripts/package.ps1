<#
.SYNOPSIS
    Compila TikVoz en modo release y arma los archivos de una versión en dist/.

.DESCRIPTION
    Lo usan tanto GitHub Actions (release.yml, ci.yml) como cualquiera en su PC,
    así lo que se publica es reproducible. Genera:
      dist/tikvoz-v<ver>-windows-x64.exe   ejecutable suelto
      dist/tikvoz-v<ver>-windows-x64.zip   exe + README + LICENSE + CHANGELOG
      dist/SHA256SUMS.txt                  sumas de verificación (formato sha256sum)
      dist/RELEASE_NOTES.md                notas de esta versión sacadas del CHANGELOG

.EXAMPLE
    pwsh scripts/package.ps1
    pwsh scripts/package.ps1 -SkipBuild
#>
param(
    [string]$OutDir = "dist",
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

# Versión única: la de Cargo.toml ([package] version = "x.y.z").
$cargo = Get-Content Cargo.toml -Raw
if ($cargo -notmatch '(?ms)^\[package\].*?^version\s*=\s*"([^"]+)"') {
    throw "No se encontró la versión en Cargo.toml"
}
$version = $Matches[1]
$name = "tikvoz-v$version-windows-x64"
Write-Host "Empaquetando TikVoz v$version" -ForegroundColor Cyan

if (-not $SkipBuild) {
    cargo build --release --locked
    if ($LASTEXITCODE -ne 0) { throw "cargo build falló" }
}
$exe = Join-Path $root "target/release/tikvoz.exe"
if (-not (Test-Path $exe)) { throw "No existe $exe (¿compilaste?)" }

# Carpeta limpia.
if (Test-Path $OutDir) { Remove-Item $OutDir -Recurse -Force }
$stage = Join-Path $OutDir $name
New-Item -ItemType Directory -Force $stage | Out-Null

Copy-Item $exe (Join-Path $stage "tikvoz.exe")
foreach ($f in "README.md", "LICENSE", "CHANGELOG.md") {
    if (Test-Path $f) { Copy-Item $f $stage }
}

$zip = Join-Path $OutDir "$name.zip"
Compress-Archive -Path (Join-Path $stage "*") -DestinationPath $zip -CompressionLevel Optimal
Copy-Item $exe (Join-Path $OutDir "$name.exe")
Remove-Item $stage -Recurse -Force

# Sumas SHA-256 (verificables con `sha256sum -c SHA256SUMS.txt` o Get-FileHash).
$sums = Get-ChildItem $OutDir -File | Where-Object { $_.Extension -in ".exe", ".zip" } | Sort-Object Name | ForEach-Object {
    "{0}  {1}" -f (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLower(), $_.Name
}
$sums | Set-Content (Join-Path $OutDir "SHA256SUMS.txt") -Encoding ascii

# Notas de la versión: la sección "## [x.y.z]" del CHANGELOG.
$notes = "TikVoz v$version"
if (Test-Path CHANGELOG.md) {
    $log = Get-Content CHANGELOG.md -Raw
    $pattern = "(?ms)^## \[" + [regex]::Escape($version) + "\][^\n]*\n(.*?)(?=^## \[|\z)"
    if ($log -match $pattern) {
        # Sin las definiciones de enlaces de Keep a Changelog ("[1.0.0]: ...").
        $notes = ($Matches[1] -split "`n" | Where-Object { $_ -notmatch '^\[[^\]]+\]:\s' }) -join "`n"
        $notes = $notes.Trim()
    }
}
$notes += "`n`n### Verificación`n`n``````text`n" + ($sums -join "`n") + "`n```````n"
$notes | Set-Content (Join-Path $OutDir "RELEASE_NOTES.md") -Encoding utf8

Write-Host "`nListo en $OutDir/:" -ForegroundColor Green
Get-ChildItem $OutDir | ForEach-Object { "  {0,-40} {1,8:N1} MB" -f $_.Name, ($_.Length / 1MB) }
