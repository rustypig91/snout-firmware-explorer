# Exercise failures without building Rust binaries or requiring the Windows SDK.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$source = Join-Path $PSScriptRoot '../build-msix.ps1'
$tokens = $null
$parseErrors = $null
[void][System.Management.Automation.Language.Parser]::ParseFile($source, [ref]$tokens, [ref]$parseErrors)
if ($parseErrors.Count) { throw ($parseErrors | Out-String) }

$fixture = Join-Path ([System.IO.Path]::GetTempPath()) "snout-msix-test-$([guid]::NewGuid().ToString('N'))"
$previousOs = $env:OS
$previousTargetDir = $env:CARGO_TARGET_DIR
$previousFlags = $env:CARGO_ENCODED_RUSTFLAGS
$previousPlainFlags = $env:RUSTFLAGS
$previousProcessDirectory = [Environment]::CurrentDirectory
New-Item -ItemType Directory -Path (Join-Path $fixture 'scripts') -Force | Out-Null
Copy-Item -LiteralPath $source -Destination (Join-Path $fixture 'scripts/build-msix.ps1')

# These functions are inherited by the child script, but never launch tools.
function cargo {
    if ($args[0] -eq 'metadata') {
        $global:LASTEXITCODE = 0
        @{ packages = @(@{ name = 'snout'; version = $fixtureVersion }) } | ConvertTo-Json -Depth 4
    } else {
        if ($PackageVersion -cne "$fixtureVersion.0") { throw 'Package version was not derived from the app version' }
        if (-not [string]::Equals($env:CARGO_ENCODED_RUSTFLAGS, $expectedBuildFlags, [StringComparison]::Ordinal)) {
            throw 'Build received incorrect compiler flags'
        }
        throw 'Simulated build failure'
    }
}
function rustup { $global:LASTEXITCODE = 0 }
function Get-Command {
    param([string]$Name, [string]$ErrorAction)
    if ($Name -in @('MakeAppx.exe', 'MakePri.exe')) {
        [pscustomobject]@{ Source = 'unused-sdk-tool' }
    } else { throw "Unexpected tool lookup: $Name" }
}
function Remove-Item {
    param([string]$LiteralPath, [switch]$Recurse, [switch]$Force)
    if ($failCleanup) { throw 'Simulated staging cleanup failure' }
    Microsoft.PowerShell.Management\Remove-Item -LiteralPath $LiteralPath -Recurse:$Recurse -Force:$Force
}

Push-Location ([System.IO.Path]::GetTempPath())
try {
    $env:OS = 'Windows_NT'
    # Keep the process directory different from PowerShell's current location
    # and the fixture root, as happens in interactive PowerShell sessions.
    [Environment]::CurrentDirectory = [System.IO.Path]::GetTempPath()
    # Compare flags ordinally: culture comparisons can ignore unit separators.
    $flagCases = @(
        @{ Encoded = "--cfg=original_flags$([char]31)-C$([char]31)debuginfo=0"; Plain = '--cfg=ignored'; Expected = @('--cfg=original_flags', '-C', 'debuginfo=0') -join [char]31 },
        @{ Encoded = $null; Plain = "  --cfg=plain_flags `t -C debuginfo=0  "; Expected = @('--cfg=plain_flags', '-C', 'debuginfo=0') -join [char]31 },
        @{ Encoded = $null; Plain = " `t "; Expected = '' },
        @{ Encoded = $null; Plain = $null; Expected = '' }
    )
    # PowerShell 7.5+ preserves empty environment variables. Cargo treats an
    # explicitly empty encoded variable as overriding nonempty plain flags.
    if ($PSVersionTable.PSVersion -ge [version]'7.5') {
        $flagCases += @{ Encoded = ''; Plain = '--cfg=ignored'; Expected = '' }
    }
    foreach ($fixtureVersion in @('1.0.0', '2.17.9', '65535.65535.65535')) {
        foreach ($flagCase in $flagCases) {
            foreach ($failCleanup in @($false, $true)) {
                $env:CARGO_TARGET_DIR = 'original-target'
                $env:CARGO_ENCODED_RUSTFLAGS = $flagCase.Encoded
                $env:RUSTFLAGS = $flagCase.Plain
                $expectedBuildFlags = if ($flagCase.Expected) {
                    "$($flagCase.Expected)$([char]31)-Ctarget-feature=+crt-static"
                } else { '-Ctarget-feature=+crt-static' }
                $location = (Get-Location).Path
                $expected = if ($failCleanup) { 'Simulated staging cleanup failure' } else { 'Simulated build failure' }
                $failure = $null
                try {
                    & (Join-Path $fixture 'scripts/build-msix.ps1') -OutputDirectory 'review-output'
                } catch { $failure = $_.Exception.Message }
                if ($failure -ne $expected) { throw "Expected '$expected', got '$failure'" }
                if (-not (Test-Path -LiteralPath (Join-Path $fixture 'review-output'))) {
                    throw 'Relative output was not resolved against the repository root'
                }
                if ($env:CARGO_TARGET_DIR -ne 'original-target' -or
                    -not [string]::Equals($env:CARGO_ENCODED_RUSTFLAGS, $flagCase.Encoded, [StringComparison]::Ordinal) -or
                    -not [string]::Equals($env:RUSTFLAGS, $flagCase.Plain, [StringComparison]::Ordinal)) {
                    throw 'Cargo environment was not restored after failure'
                }
                if ((Get-Location).Path -ne $location) { throw 'Caller location was not restored after failure' }
                $staging = @(Get-ChildItem -LiteralPath (Join-Path $fixture 'target/store') -Directory -Filter 'staging-*')
                if (-not $failCleanup -and $staging.Count) { throw 'Failed build left staging behind' }
                # The cleanup-failure fixture deliberately retains its staging directory.
                foreach ($directory in $staging) {
                    Microsoft.PowerShell.Management\Remove-Item -LiteralPath $directory.FullName -Recurse -Force
                }
            }
        }
    }
    foreach ($case in @(
        @{ Version = '0.8.1'; Error = 'PackageVersion must be major.minor.patch.0, with a nonzero major version.' },
        @{ Version = '1.0.0-beta.1'; Error = 'Store releases require a numeric major.minor.patch version.' },
        @{ Version = '1.2.3.4'; Error = 'Store releases require a numeric major.minor.patch version.' },
        @{ Version = '65536.1.2'; Error = 'Each package version component must be at most 65535.' },
        @{ Version = '1.65536.2'; Error = 'Each package version component must be at most 65535.' }
    )) {
        $fixtureVersion = $case.Version
        $location = (Get-Location).Path
        $failure = $null
        try { & (Join-Path $fixture 'scripts/build-msix.ps1') } catch { $failure = $_.Exception.Message }
        if ($failure -ne $case.Error) { throw "Unexpected result for $fixtureVersion`: $failure" }
        if ((Get-Location).Path -ne $location -or $env:CARGO_TARGET_DIR -ne 'original-target' -or
            -not [string]::Equals($env:CARGO_ENCODED_RUSTFLAGS, $flagCase.Encoded, [StringComparison]::Ordinal)) {
            throw 'Invalid-version failure changed caller state'
        }
    }
    Write-Host 'MSIX script regression checks passed (derived versions, invalid versions, compiler flags, relative output, build failure, cleanup failure, environment and location restoration).'
} finally {
    $env:OS = $previousOs
    $env:CARGO_TARGET_DIR = $previousTargetDir
    $env:CARGO_ENCODED_RUSTFLAGS = $previousFlags
    $env:RUSTFLAGS = $previousPlainFlags
    [Environment]::CurrentDirectory = $previousProcessDirectory
    Pop-Location
    Microsoft.PowerShell.Management\Remove-Item -LiteralPath $fixture -Recurse -Force
}
