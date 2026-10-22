[CmdletBinding(SupportsShouldProcess = $true)]
param([switch]$Apply)
$ErrorActionPreference = 'Stop'
$cleanupMain = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\..')).Path
$cleanupParent = Split-Path -Parent $cleanupMain
$cleanupRoots = @{
    'rust-saas' = $cleanupMain
    'saas-work-orders' = Join-Path $cleanupParent 'saas-work-orders'
    'saas-stores' = Join-Path $cleanupParent 'saas-stores'
}
$cleanupPlan = Get-Content -LiteralPath (Join-Path $PSScriptRoot '..\retired-paths.json') -Raw | ConvertFrom-Json
$cleanupValidated = @()
foreach ($entry in $cleanupPlan.entries) {
    if (-not $cleanupRoots.ContainsKey($entry.project)) { throw 'Unknown project in cleanup plan' }
    $projectRoot = [IO.Path]::GetFullPath($cleanupRoots[$entry.project])
    $target = [IO.Path]::GetFullPath((Join-Path $projectRoot $entry.path))
    if (-not $target.StartsWith($projectRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw 'Target escaped project root' }
    $entrySegments = $entry.path -split '[\\/]'
    if ($entrySegments -contains '.git' -or $entry.path -in @('backend', 'frontend', '.github', 'backend/ops', 'backend/ops/monitoring', 'backend/ops/recovery')) { throw 'Protected project path' }
    if (-not (Test-Path -LiteralPath $target)) { continue }
    $resolved = (Resolve-Path -LiteralPath $target).Path
    if (-not $resolved.StartsWith($projectRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw 'Resolved target escaped project root' }
    if ((Get-Item -LiteralPath $resolved -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw ('Refusing reparse target: ' + $resolved) }
    $cleanupValidated += $resolved
}
$cleanupValidated | ForEach-Object { Write-Output $_ }
if (-not $Apply) {
    Write-Output ('Preview only: ' + $cleanupValidated.Count + ' retired paths. Review retired-paths.json, then run this script with -Apply to remove them.')
    return
}
foreach ($target in $cleanupValidated) {
    if ($PSCmdlet.ShouldProcess($target, 'Remove retired project path')) {
        Remove-Item -LiteralPath $target -Force -Recurse
    }
}

