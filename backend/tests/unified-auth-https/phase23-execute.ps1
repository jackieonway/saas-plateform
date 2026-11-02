#requires -Version 7.0
param(
    [Parameter(Mandatory = $true)][ValidatePattern('^phase(23|25)-tools-r[1-9][0-9]*$')][string]$Snapshot,
    [Parameter(Mandatory = $true)][ValidatePattern('^phase(23|25)-apps-r[1-9][0-9]*$')][string]$BuildSnapshot,
    [ValidateSet(23, 25)][int]$Phase = 23,
    [ValidatePattern('^r[1-9][0-9]*$')][string]$RunId,
    [switch]$ResumePrepared,
    [string]$FileNegativeEvidence,
    [ValidateSet('full', 'recovery')][string]$ValidationProfile = 'full'
)

$ErrorActionPreference = 'Stop'
# Native commands have explicit exit checks, including independent preflight
# aggregation. Keep that behavior stable under caller PowerShell preferences.
$PSNativeCommandUseErrorActionPreference = $false
if ($Snapshot -notmatch "^phase$Phase-tools-" -or $BuildSnapshot -notmatch "^phase$Phase-apps-") {
    throw 'EXECUTION_PHASE_SNAPSHOT_MISMATCH'
}
if ($Phase -eq 25 -and (-not $RunId -or $FileNegativeEvidence -or $ResumePrepared -or $ValidationProfile -ne 'full')) {
    throw 'PHASE25_FRESH_FULL_EXECUTION_REQUIRED'
}
$taskDirectory = '/tmp/rust-saas-unified-auth-20261028'
$frozenTools = "$taskDirectory/source-snapshot-$Snapshot/saas-plateform/backend/tests/unified-auth-https"
$reportStem = "phase-$Phase-" + $(if ($RunId) { "$RunId-" } else { '' })
$outputFile = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot "../../docs/designs/unified-auth/validation/${reportStem}integration.json"))
if ($Phase -eq 25 -and (Test-Path -LiteralPath $outputFile)) { throw 'PHASE25_NEW_RESULT_PATH_REQUIRED' }
$browserFile = Join-Path $PSScriptRoot 'phase23-browser.mjs'
if ($FileNegativeEvidence) {
    $FileNegativeEvidence = [IO.Path]::GetFullPath($FileNegativeEvidence)
    if ([IO.Path]::GetDirectoryName($FileNegativeEvidence) -ne [IO.Path]::GetDirectoryName($outputFile) -or
        [IO.Path]::GetFileName($FileNegativeEvidence) -notmatch '^phase-23-[a-z0-9-]+\.json$' -or
        -not (Test-Path -LiteralPath $FileNegativeEvidence -PathType Leaf)) {
        throw 'PHASE23_FILE_NEGATIVE_EVIDENCE_PATH_INVALID'
    }
}
$nodeBinary = (Get-Command node.exe -ErrorAction Stop).Source
$proxyProcess = $null
$browserProcess = $null
$prepareAttempted = $false
$deploymentId = $null
$hbaAttempted = $false
$cleanupFailures = [Collections.Generic.List[string]]::new()
$preflightFailures = [Collections.Generic.List[string]]::new()
$executionStage = 'FROZEN_ENTRYPOINT_CHECKS'
$executionSucceeded = $false
$executionStartedAt = [DateTime]::UtcNow.ToString('o')
$logDirectory = Join-Path ([IO.Path]::GetTempPath()) ('rust-saas-phase23-executor-' + [Guid]::NewGuid().ToString('N'))

function Invoke-OwnedWsl {
    # A simple function preserves Python's -c and every child switch verbatim;
    # an advanced PowerShell parameter named CommandArguments would consume -c.
    & wsl.exe -d Ubuntu -u root --exec @args
    if ($LASTEXITCODE -ne 0) { throw 'PHASE23_OWNED_COMMAND_FAILED' }
}

try {
    $snapshotChecks = & wsl.exe -d Ubuntu -u root --exec python3 -c 'import json,pathlib,sys; root=pathlib.Path(sys.argv[1]); value=json.loads((root/("snapshot-"+sys.argv[2]+".json")).read_text()); prefix="saas-plateform/backend/tests/unified-auth-https/"; print(json.dumps({name:value["inputs"][prefix+name] for name in ("phase23-execute.ps1","connect-proxy.mjs","phase23-browser.mjs")}))' $taskDirectory $Snapshot
    if ($LASTEXITCODE -ne 0) { throw 'PHASE23_ENTRYPOINT_MANIFEST_REQUIRED' }
    $snapshotChecks = $snapshotChecks | ConvertFrom-Json -AsHashtable
    foreach ($name in $snapshotChecks.Keys) {
        if ((Get-FileHash -LiteralPath (Join-Path $PSScriptRoot $name) -Algorithm SHA256).Hash.ToLowerInvariant() -ne $snapshotChecks[$name]) {
            throw 'PHASE23_LIVE_ENTRYPOINT_NOT_FROZEN'
        }
    }
    if ($Phase -eq 25) {
        $phase25Checks = & wsl.exe -d Ubuntu -u root --exec python3 -c 'import json,pathlib,sys; root=pathlib.Path(sys.argv[1]); value=json.loads((root/("snapshot-"+sys.argv[2]+".json")).read_text()); prefix="saas-plateform/backend/tests/unified-auth-https/"; print(json.dumps({name:value["inputs"][prefix+name] for name in ("phase25-ui-browser.mjs","phase25-business-browser.mjs","phase25-resource-browser.mjs","phase25-protocol-browser.mjs","phase25-context-browser.mjs","phase19-browser.mjs","phase20-browser.mjs")}))' $taskDirectory $Snapshot
        if ($LASTEXITCODE -ne 0) { throw 'PHASE25_ENTRYPOINT_MANIFEST_REQUIRED' }
        $phase25Checks = $phase25Checks | ConvertFrom-Json -AsHashtable
        foreach ($name in $phase25Checks.Keys) {
            if ((Get-FileHash -LiteralPath (Join-Path $PSScriptRoot $name) -Algorithm SHA256).Hash.ToLowerInvariant() -ne $phase25Checks[$name]) {
                throw 'PHASE25_LIVE_ENTRYPOINT_NOT_FROZEN'
            }
        }
    }
    # Protocol imports use the frozen RP, whose sole allowed external dependency
    # mount is documented in verifySource(). Neither source files nor the
    # reusable dependency bytes are changed here.
    $rpDependencyCode = @'
import pathlib,sys,os,hashlib,stat
task=pathlib.Path(sys.argv[1]); snapshot=sys.argv[2]
assert task.resolve()==task and task.stat().st_uid==os.getuid()
assert (task/'OWNER').read_text()=='rust-saas-unified-auth-20261028-pg55494\n'
source=task/('source-snapshot-'+snapshot)/'saas-examples/oidc-rp'
modules=task/'phase21-rp-source/node_modules'
assert source.resolve()==source and modules.resolve()==modules and modules.is_dir()
assert source.stat().st_uid==os.getuid() and modules.stat().st_uid==os.getuid()
assert hashlib.sha256((source/'package-lock.json').read_bytes()).digest()==hashlib.sha256((modules.parent/'package-lock.json').read_bytes()).digest()
for current,dirs,files in os.walk(modules,followlinks=False):
 for item in [pathlib.Path(current),*(pathlib.Path(current)/name for name in dirs+files)]:
  info=item.lstat(); assert not item.is_symlink() and info.st_uid==os.getuid() and not stat.S_IMODE(info.st_mode)&0o022
link=source/'node_modules'
if link.is_symlink(): assert link.resolve(strict=True)==modules
else:
 assert not link.exists(); link.symlink_to(modules,target_is_directory=True)
print('PHASE23_FROZEN_RP_DEPENDENCY_MOUNT_VERIFIED')
'@
    & wsl.exe -d Ubuntu -u jackie --exec python3 -B -c $rpDependencyCode $taskDirectory $Snapshot
    if ($LASTEXITCODE -ne 0) { throw 'PHASE23_FROZEN_RP_DEPENDENCY_MOUNT_FAILED' }
    $prepareAttempted = $true
    $executionStage = 'CORE_PREPARATION'
    if ($ResumePrepared) {
        Invoke-OwnedWsl python3 -B "$frozenTools/phase23-environment.py" select-tools --task-dir $taskDirectory --snapshot $Snapshot --build-snapshot $BuildSnapshot --hba-restored
    }
    else {
        Invoke-OwnedWsl python3 -B "$frozenTools/phase23-environment.py" prepare --task-dir $taskDirectory --snapshot $Snapshot --build-snapshot $BuildSnapshot
    }
    $deploymentId = (& wsl.exe -d Ubuntu -u root --exec python3 -c 'import json,pathlib; print(json.loads(pathlib.Path("/tmp/rust-saas-unified-auth-20261028/https-environment.json").read_text())["ids"]["deployment"])').Trim()
    if ($LASTEXITCODE -ne 0 -or $deploymentId -notmatch '^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$') {
        throw 'PHASE23_DEPLOYMENT_BINDING_REQUIRED'
    }
    if ($Phase -eq 25) {
        $executionStage = 'BUSINESS_PREPARATION'
        Invoke-OwnedWsl python3 -B "$frozenTools/phase25-business-fixture.py" prepare --task-dir $taskDirectory
    }
    $executionStage = 'CORE_START'
    Invoke-OwnedWsl python3 -B "$frozenTools/phase23-environment.py" start --task-dir $taskDirectory
    if ($Phase -eq 25) {
        $executionStage = 'BUSINESS_START'
        Invoke-OwnedWsl python3 -B "$frozenTools/phase25-business-runtime.py" start --task-dir $taskDirectory
    }
    $executionStage = 'WINDOWS_CONNECT_PROXY'
    [IO.Directory]::CreateDirectory($logDirectory) | Out-Null
    $proxyArguments = @(
        (Join-Path $PSScriptRoot 'connect-proxy.mjs'), '--task-dir', $taskDirectory,
        '--additional-public-origin', 'https://customer.saas.test:24446'
    )
    if ($Phase -eq 25) { $proxyArguments += @('--business-public-origin', 'https://console.saas.test:24447') }
    $proxyProcess = Start-Process -FilePath $nodeBinary -ArgumentList $proxyArguments -WindowStyle Hidden -PassThru `
        -RedirectStandardOutput (Join-Path $logDirectory 'proxy.stdout.log') `
        -RedirectStandardError (Join-Path $logDirectory 'proxy.stderr.log')
    $proxyReady = $false
    for ($attempt = 0; $attempt -lt 100; $attempt++) {
        if ($proxyProcess.HasExited) { throw 'PHASE23_PROXY_START_FAILED' }
        $listener = Get-NetTCPConnection -State Listen -LocalAddress '127.0.0.1' -LocalPort 24445 -ErrorAction SilentlyContinue
        if ($listener -and $listener.OwningProcess -eq $proxyProcess.Id) { $proxyReady = $true; break }
        Start-Sleep -Milliseconds 100
    }
    if (-not $proxyReady) { throw 'PHASE23_OWNED_PROXY_NOT_READY' }
    if ($Phase -eq 25) {
        foreach ($entry in @(@('phase25-ui-browser.mjs', 'ui'), @('phase25-business-browser.mjs', 'business'), @('phase25-resource-browser.mjs', 'resource'), @('phase25-protocol-browser.mjs', 'protocol'), @('phase25-context-browser.mjs', 'context'))) {
            $executionStage = 'ACTUAL_' + $entry[1].ToUpperInvariant()
            $preflightArguments = @((Join-Path $PSScriptRoot $entry[0]), '--snapshot', $Snapshot,
                '--output', (Join-Path ([IO.Path]::GetDirectoryName($outputFile)) ("phase-25-" + $entry[1] + "-current-$RunId.json")))
            & $nodeBinary @preflightArguments
            if ($LASTEXITCODE -ne 0) {
                $preflightFailures.Add('PHASE25_' + $entry[1].ToUpperInvariant() + '_FAILED')
                Write-Output ('PHASE25_PREFLIGHT_FAILED ' + $entry[1].ToUpperInvariant())
            }
        }
        foreach ($entry in @(@('phase19-browser.mjs', 'management'), @('phase20-browser.mjs', 'logout'))) {
            $executionStage = 'ACTUAL_' + $entry[1].ToUpperInvariant()
            $managementArguments = @((Join-Path $PSScriptRoot $entry[0]), '--phase', '25', '--snapshot', $Snapshot,
                '--environment', "$taskDirectory/https-environment.json", '--proxy', 'http://127.0.0.1:24445',
                '--output', (Join-Path ([IO.Path]::GetDirectoryName($outputFile)) ("phase-25-" + $entry[1] + "-current-$RunId.json")))
            & $nodeBinary @managementArguments
            if ($LASTEXITCODE -ne 0) {
                $preflightFailures.Add('PHASE25_' + $entry[1].ToUpperInvariant() + '_FAILED')
                Write-Output ('PHASE25_PREFLIGHT_FAILED ' + $entry[1].ToUpperInvariant())
            }
        }
        # Every driver owns its browser and narrow fixture finally. Collect the
        # independent results in the same owned installation; a failed group
        # still prevents any recovery/HBA write window and overall success.
        if ($preflightFailures.Count -gt 0) { throw ($preflightFailures -join ',') }
        $executionStage = 'FRESH_BUSINESS_RECOVERY_CARRIERS'
        $captureArguments = @((Join-Path $PSScriptRoot 'phase25-resource-browser.mjs'), '--snapshot', $Snapshot,
            '--capture-recovery', 'true', '--output',
            (Join-Path ([IO.Path]::GetDirectoryName($outputFile)) ("phase-25-resource-recovery-capture-current-$RunId.json")))
        & $nodeBinary @captureArguments
        if ($LASTEXITCODE -ne 0) { throw 'PHASE25_RECOVERY_CARRIER_CAPTURE_FAILED' }
        $executionStage = 'BUSINESS_CLEANUP_BEFORE_RECOVERY'
        Invoke-OwnedWsl python3 -B "$frozenTools/phase25-business-runtime.py" stop --task-dir $taskDirectory
        $executionStage = 'ACTUAL_BUSINESS_RESTORE_NEW'
        $businessRecoveryOutput = & wsl.exe -d Ubuntu -u root --exec python3 -B "$frozenTools/phase25-business-recovery.py" --task-dir $taskDirectory
        $businessRecoveryExitCode = $LASTEXITCODE
        if ($businessRecoveryOutput) {
            $businessRecoveryOutput | ConvertFrom-Json | ConvertTo-Json -Depth 30 |
                Set-Content -LiteralPath (Join-Path ([IO.Path]::GetDirectoryName($outputFile)) ("phase-25-business-recovery-current-$RunId.json")) -Encoding utf8
        }
        if ($businessRecoveryExitCode -ne 0) { throw 'PHASE25_BUSINESS_RESTORE_NEW_FAILED' }
        $executionStage = 'BUSINESS_FIXTURE_CLEANUP_BEFORE_IDENTITY_RECOVERY'
        Invoke-OwnedWsl python3 -B "$frozenTools/phase25-business-fixture.py" cleanup --task-dir $taskDirectory
    }
    $executionStage = 'OWNED_HBA_INSTALL'
    $hbaAttempted = $true
    Invoke-OwnedWsl python3 -B "$frozenTools/phase23-hba.py" install --task-dir $taskDirectory --deployment-id $deploymentId
    $browserArguments = @(('"' + $browserFile + '"'), '--snapshot', $Snapshot, '--output', ('"' + $outputFile + '"'))
    $browserArguments += @('--validation-profile', $ValidationProfile)
    $browserArguments += @('--phase', [string]$Phase)
    if ($FileNegativeEvidence) { $browserArguments += @('--file-negative-evidence', ('"' + $FileNegativeEvidence + '"')) }
    $browserProcess = Start-Process -FilePath $nodeBinary -ArgumentList $browserArguments `
        -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $logDirectory 'browser.stdout.log') `
        -RedirectStandardError (Join-Path $logDirectory 'browser.stderr.log')
    $executionStage = 'ACTUAL_FULL_KEY_LIFECYCLE_AND_RECOVERY'
    while (-not $browserProcess.WaitForExit(500)) { }
    Get-Content -LiteralPath (Join-Path $logDirectory 'browser.stdout.log')
    Get-Content -LiteralPath (Join-Path $logDirectory 'browser.stderr.log')
    if ($browserProcess.ExitCode -ne 0) { throw 'PHASE23_ACTUAL_BROWSER_FAILED' }
    $executionSucceeded = $true
}
finally {
    if ($null -ne $browserProcess) {
        try {
            if (-not $browserProcess.HasExited) {
                $browserProcess.Kill($true)
                if (-not $browserProcess.WaitForExit(10000)) { throw 'PHASE23_BROWSER_STOP_TIMEOUT' }
            }
        }
        catch { $cleanupFailures.Add('OWNED_WINDOWS_BROWSER_CLEANUP_FAILED') }
        finally { $browserProcess.Dispose() }
    }
    # Stop owned writers even when HBA restoration needs owner reconciliation.
    if ($Phase -eq 25 -and $prepareAttempted) {
        try { Invoke-OwnedWsl python3 -B "$frozenTools/phase25-business-runtime.py" stop --task-dir $taskDirectory }
        catch { $cleanupFailures.Add('PHASE25_BUSINESS_PROCESS_CLEANUP_FAILED') }
        try { Invoke-OwnedWsl python3 -B "$frozenTools/phase25-business-recovery.py" --task-dir $taskDirectory --cleanup-only }
        catch { $cleanupFailures.Add('PHASE25_BUSINESS_RECOVERY_CLEANUP_FAILED') }
        try { Invoke-OwnedWsl python3 -B "$frozenTools/phase25-business-fixture.py" cleanup --task-dir $taskDirectory }
        catch { $cleanupFailures.Add('PHASE25_BUSINESS_FIXTURE_CLEANUP_FAILED') }
    }
    if ($prepareAttempted) {
        try { Invoke-OwnedWsl python3 -B "$frozenTools/phase23-environment.py" cleanup --task-dir $taskDirectory }
        catch { $cleanupFailures.Add('OWNED_ENVIRONMENT_PROCESS_CLEANUP_FAILED') }
    }
    $hbaRestored = -not $hbaAttempted
    if ($hbaAttempted -and $deploymentId) {
        try {
            $hbaProof = Invoke-OwnedWsl python3 -B "$frozenTools/phase23-hba.py" restore --task-dir $taskDirectory --deployment-id $deploymentId
            $hbaProof | ConvertFrom-Json | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath ($outputFile -replace '-integration.json$', '-hba-restoration.json') -Encoding utf8
            Invoke-OwnedWsl python3 -B "$frozenTools/phase23-hba.py" cleanup-receipt --task-dir $taskDirectory --deployment-id $deploymentId
            $hbaRestored = $true
        }
        catch { $cleanupFailures.Add('HBA_RESTORATION_REQUIRES_OWNER_RECONCILIATION') }
    }
    if ($prepareAttempted -and $hbaRestored) {
        try { Invoke-OwnedWsl python3 -B "$frozenTools/phase23-environment.py" cleanup --task-dir $taskDirectory --hba-restored }
        catch { $cleanupFailures.Add('OWNED_ENVIRONMENT_CLEANUP_FAILED') }
        try {
            $cleanupProof = Invoke-OwnedWsl python3 -c 'import json,pathlib; print(json.dumps(json.loads(pathlib.Path("/tmp/rust-saas-unified-auth-20261028/phase23-environment-cleanup.json").read_text())))'
            $cleanupProof | ConvertFrom-Json | ConvertTo-Json -Depth 30 | Set-Content -LiteralPath ($outputFile -replace '-integration.json$', '-cleanup.json') -Encoding utf8
        }
        catch { $cleanupFailures.Add('OWNED_CLEANUP_PROOF_UNAVAILABLE') }
    }
    if ($null -ne $proxyProcess) {
        try {
            if (-not $proxyProcess.HasExited) {
                $proxyProcess.Kill($true)
                if (-not $proxyProcess.WaitForExit(10000)) { throw 'PHASE23_PROXY_STOP_TIMEOUT' }
            }
        }
        catch { $cleanupFailures.Add('OWNED_WINDOWS_PROXY_CLEANUP_FAILED') }
        finally { $proxyProcess.Dispose() }
    }
    if (Test-Path -LiteralPath $logDirectory) {
        foreach ($name in @('proxy.stdout.log', 'proxy.stderr.log', 'browser.stdout.log', 'browser.stderr.log')) {
            $filename = Join-Path $logDirectory $name
            if (Test-Path -LiteralPath $filename) { Remove-Item -LiteralPath $filename -Force }
        }
        Remove-Item -LiteralPath $logDirectory
    }
    if ($Phase -eq 25) {
        [ordered]@{
            contractVersion = 'unified-auth-phase25-execution-journal/1'
            phase = 25
            runId = $RunId
            snapshot = $Snapshot
            buildSnapshot = $BuildSnapshot
            startedAtUtc = $executionStartedAt
            finishedAtUtc = [DateTime]::UtcNow.ToString('o')
            result = $(if ($executionSucceeded -and $cleanupFailures.Count -eq 0) { 'PASS' } else { 'FAIL' })
            lastExecutionStage = $executionStage
            cleanupFailureCodes = @($cleanupFailures)
            preflightFailureCodes = @($preflightFailures)
            fullRecoveryRequested = $true
            hbaInstallAttempted = $hbaAttempted
            fullRecoveryBrowserStarted = $null -ne $browserProcess
            productionDeploymentExecuted = $false
            secretsOrRawLogsPersisted = $false
        } | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath ($outputFile -replace '-integration.json$', '-execution.json') -Encoding utf8
    }
    if ($cleanupFailures.Count -gt 0) { throw ($cleanupFailures -join ',') }
}
