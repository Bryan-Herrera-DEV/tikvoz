<#
.SYNOPSIS
    Compara baselines de Criterion (diseño A-B-A) y, opcionalmente, el tamaño de dos .exe.

.DESCRIPTION
    Lee target/criterion/<grupo>/<benchmark>/<baseline>/estimates.json.

    Diseño A-B-A: la rama base se mide dos veces (-Base antes del PR y -Base2
    después) y el PR (-New) entre ambas. Así:
      - la deriva de la máquina durante la medición se reparte por igual, y
      - la diferencia entre las dos mediciones de la base estima el RUIDO real de
        cada benchmark en esa máquina.

    Un benchmark cuenta como REGRESIÓN solo si:
      1. su mediana empeora más que max(-Threshold, 2 × ruido) respecto de la
         media de las dos bases, y
      2. el intervalo de confianza del 95 % del PR no se solapa con el de
         ninguna de las bases.
    Sin -Base2 se compara contra una sola base (útil en local, menos robusto).

    Sale con código 1 si hay alguna regresión o si el .exe crece más que
    -SizeThreshold, para que CI marque el PR.

.EXAMPLE
    # En local, con una sola base:
    cargo bench -- --save-baseline main      # en main
    git switch mi-rama
    cargo bench -- --save-baseline pr
    pwsh scripts/bench-compare.ps1 -Base main -New pr
#>
param(
    [string]$Base = "base",
    [string]$Base2 = "",
    [string]$New = "pr",
    [double]$Threshold = 10,
    [string]$BaseExe = "",
    [string]$NewExe = "",
    [double]$SizeThreshold = 5,
    [string]$Summary = "",
    [string]$CriterionDir = "target/criterion"
)

$ErrorActionPreference = "Stop"

function Format-Time([double]$ns) {
    if ($ns -ge 1e6) { return "{0:N2} ms" -f ($ns / 1e6) }
    if ($ns -ge 1e3) { return "{0:N2} µs" -f ($ns / 1e3) }
    return "{0:N0} ns" -f $ns
}

function Read-Estimates([string]$dir, [string]$name) {
    $f = Join-Path $dir "$name/estimates.json"
    if ($name -and (Test-Path $f)) { return Get-Content $f -Raw | ConvertFrom-Json }
    return $null
}

# ¿El intervalo de `a` queda completamente por encima del de `b`?
function Is-Above($a, $b) {
    return $a.mean.confidence_interval.lower_bound -gt $b.mean.confidence_interval.upper_bound
}
function Is-Below($a, $b) {
    return $a.mean.confidence_interval.upper_bound -lt $b.mean.confidence_interval.lower_bound
}

$rows = @()
$regressions = 0
$improvements = 0

$newFiles = Get-ChildItem $CriterionDir -Recurse -Filter estimates.json -ErrorAction SilentlyContinue |
    Where-Object { $_.Directory.Name -eq $New }

foreach ($f in $newFiles | Sort-Object FullName) {
    $benchDir = $f.Directory.Parent.FullName
    $id = [IO.Path]::GetRelativePath((Resolve-Path $CriterionDir), $benchDir) -replace '\\', '/'
    $n = Get-Content $f.FullName -Raw | ConvertFrom-Json
    $b1 = Read-Estimates $benchDir $Base
    $b2 = Read-Estimates $benchDir $Base2

    if (-not $b1) {
        $rows += "| ``$id`` | — | $(Format-Time $n.median.point_estimate) | — | — | nuevo |"
        continue
    }
    $bases = @($b1) + @($b2 | Where-Object { $_ })
    $baseMedian = ($bases | ForEach-Object { $_.median.point_estimate } | Measure-Object -Average).Average
    $noise = 0
    if ($b2) {
        $noise = [math]::Abs($b1.median.point_estimate - $b2.median.point_estimate) / $baseMedian * 100
    }
    $limit = [math]::Max($Threshold, 2 * $noise)
    $nm = $n.median.point_estimate
    $change = ($nm - $baseMedian) / $baseMedian * 100

    $slower = @($bases | Where-Object { Is-Above $n $_ }).Count -eq $bases.Count
    $faster = @($bases | Where-Object { Is-Below $n $_ }).Count -eq $bases.Count

    $verdict = "sin cambios"
    if ($slower -and $change -gt $limit) { $verdict = "**REGRESIÓN**"; $regressions++ }
    elseif ($faster -and $change -lt -$limit) { $verdict = "mejora"; $improvements++ }
    elseif ([math]::Abs($change) -gt $limit) { $verdict = "dentro del ruido" }

    $noiseText = if ($b2) { "{0:N1} %" -f $noise } else { "—" }
    $rows += ("| ``$id`` | $(Format-Time $baseMedian) | $(Format-Time $nm) | {0:+0.0;-0.0;0.0} % | $noiseText | $verdict |" -f $change)
}

$design = if ($Base2) { "A-B-A: la base se midió antes y después del PR, y su diferencia es el ruido de cada benchmark." } else { "Una sola medición de la base (menos robusto que A-B-A)." }
$lines = @(
    "## Benchmarks: ``$New`` contra ``$Base``",
    "",
    "Medianas medidas en la misma máquina. $design",
    "Regresión = empeora más de max($Threshold %, 2 × ruido) **y** su intervalo de confianza del 95 % no se solapa con el de la base.",
    "",
    "| Benchmark | Base | PR | Cambio | Ruido | Resultado |",
    "|---|---:|---:|---:|---:|---|"
) + $rows

$sizeFail = $false
if ($BaseExe -and $NewExe -and (Test-Path $BaseExe) -and (Test-Path $NewExe)) {
    $bs = (Get-Item $BaseExe).Length
    $ns = (Get-Item $NewExe).Length
    $sc = ($ns - $bs) / $bs * 100
    $sizeVerdict = "sin cambios"
    if ($sc -gt $SizeThreshold) { $sizeVerdict = "**CRECE DEMASIADO**"; $sizeFail = $true }
    elseif ([math]::Abs($sc) -gt 0.5) { $sizeVerdict = "cambio menor" }
    $lines += @(
        "",
        "### Tamaño del ejecutable",
        "",
        "| | Base | PR | Cambio | Resultado |",
        "|---|---:|---:|---:|---|",
        ("| ``tikvoz.exe`` | {0:N2} MB | {1:N2} MB | {2:+0.0;-0.0;0.0} % | {3} |" -f ($bs / 1MB), ($ns / 1MB), $sc, $sizeVerdict),
        "",
        "Límite: +$SizeThreshold %. Es determinista: no depende del ruido de la máquina."
    )
}

$lines += @("", "Resultado: $regressions regresiones, $improvements mejoras.")
$md = $lines -join "`n"
Write-Output $md
if ($Summary) { $md | Out-File -FilePath $Summary -Append -Encoding utf8 }

if ($newFiles.Count -eq 0) {
    Write-Error "No hay resultados de la baseline '$New' en $CriterionDir (¿corriste cargo bench -- --save-baseline $New?)"
}
if ($regressions -gt 0 -or $sizeFail) { exit 1 }
