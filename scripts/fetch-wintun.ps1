# Download wintun.dll from the official site, verify it, and copy it into the given directories.
#
#   pwsh scripts/fetch-wintun.ps1 -Destination target\debug, target\debug\deps
#
# Two checks, both must pass:
#   1. SHA-256 of the zip matches the pinned value below
#   2. The DLL carries a valid Authenticode signature from WireGuard LLC
# Meshora loads wintun.dll only from the directory of its own executable, so whoever can
# write that directory decides which DLL runs. Install it only into admin-writable places.
#
# ASCII only on purpose: Windows PowerShell 5.1 misreads UTF-8 scripts without a BOM.

param(
    [Parameter(Mandatory = $true)]
    [string[]]$Destination,
    [string]$Architecture = 'amd64'
)

$ErrorActionPreference = 'Stop'

$version = '0.14.1'
$expected = '07c256185d6ee3652e09fa55c0b673e2624b565e02c4b9091c79ca7d2f24ef51'

$temp = if ($env:RUNNER_TEMP) { $env:RUNNER_TEMP } else { [IO.Path]::GetTempPath() }
$work = Join-Path $temp "wintun-$version-$PID"
New-Item -ItemType Directory -Force $work | Out-Null
try {
    $zip = Join-Path $work 'wintun.zip'
    Invoke-WebRequest -Uri "https://www.wintun.net/builds/wintun-$version.zip" -OutFile $zip -UseBasicParsing
    $hash = (Get-FileHash -Algorithm SHA256 $zip).Hash.ToLower()
    Write-Output "wintun-$version.zip SHA256: $hash"
    if ($hash -ne $expected) {
        throw "wintun-$version.zip SHA256 mismatch"
    }

    Expand-Archive $zip -DestinationPath $work -Force
    $dll = Join-Path $work "wintun\bin\$Architecture\wintun.dll"
    $sig = Get-AuthenticodeSignature $dll
    Write-Output "wintun.dll signature: $($sig.Status) / $($sig.SignerCertificate.Subject)"
    if ($sig.Status -ne 'Valid' -or $sig.SignerCertificate.Subject -notmatch 'O=WireGuard LLC') {
        throw 'wintun.dll is not validly signed by WireGuard LLC'
    }

    foreach ($dir in $Destination) {
        New-Item -ItemType Directory -Force $dir | Out-Null
        Copy-Item $dll (Join-Path $dir 'wintun.dll') -Force
        Write-Output "copied to $dir"
    }
    # The license travels with the DLL (the installer ships it next to wintun.dll)
    $license = Join-Path $work 'wintun\LICENSE.txt'
    if (-not (Test-Path $license)) {
        throw 'LICENSE.txt not found in the wintun zip'
    }
    foreach ($dir in $Destination) {
        Copy-Item $license (Join-Path $dir 'wintun-LICENSE.txt') -Force
    }
}
finally {
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}
