$ErrorActionPreference = 'Stop'

[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$repository = 'qinodes/stoker'
$asset = 'stoker-windows-x86_64.zip'
$releaseVersion = '__STOKER_RELEASE_VERSION__'
$installDirectory = Join-Path $env:LOCALAPPDATA 'Programs\stoker'
$temporaryDirectory = Join-Path ([IO.Path]::GetTempPath()) ('stoker-install-' + [Guid]::NewGuid().ToString('N'))
if ($releaseVersion -eq '__STOKER_RELEASE_VERSION__') {
    $releaseBaseUrl = "https://github.com/$repository/releases/latest/download"
    $releaseLabel = 'latest release'
}
else {
    $releaseBaseUrl = "https://github.com/$repository/releases/download/v$releaseVersion"
    $releaseLabel = "v$releaseVersion"
}
$archivePath = Join-Path $temporaryDirectory $asset
$checksumsPath = Join-Path $temporaryDirectory 'SHA256SUMS'
$extractDirectory = Join-Path $temporaryDirectory 'extracted'

try {
    New-Item -ItemType Directory -Path $temporaryDirectory -Force | Out-Null

    Write-Host "Downloading Stoker for Windows ($releaseLabel)..."
    Invoke-WebRequest -Uri "$releaseBaseUrl/$asset" -OutFile $archivePath -UseBasicParsing
    Invoke-WebRequest -Uri "$releaseBaseUrl/SHA256SUMS" -OutFile $checksumsPath -UseBasicParsing

    $expectedHash = $null
    foreach ($line in Get-Content -LiteralPath $checksumsPath) {
        if ($line -match '^\s*(?<hash>[0-9a-fA-F]{64})\s+\*?(?<name>.+?)\s*$' -and $Matches.name -eq $asset) {
            $expectedHash = $Matches.hash.ToLowerInvariant()
            break
        }
    }

    if (-not $expectedHash) {
        throw "Could not find a checksum for $asset."
    }

    $actualHash = (Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actualHash -ne $expectedHash) {
        throw 'The downloaded archive failed SHA256 verification.'
    }

    Expand-Archive -LiteralPath $archivePath -DestinationPath $extractDirectory -Force
    $binaryPath = Join-Path $extractDirectory 'stoker.exe'
    if (-not (Test-Path -LiteralPath $binaryPath -PathType Leaf)) {
        throw 'The downloaded archive does not contain stoker.exe.'
    }

    New-Item -ItemType Directory -Path $installDirectory -Force | Out-Null
    Copy-Item -LiteralPath $binaryPath -Destination (Join-Path $installDirectory 'stoker.exe') -Force

    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    $normalizedInstallDirectory = [IO.Path]::GetFullPath($installDirectory).TrimEnd('\')
    $pathEntries = @()
    if (-not [string]::IsNullOrWhiteSpace($userPath)) {
        $pathEntries = $userPath -split ';' | Where-Object { -not [string]::IsNullOrWhiteSpace($_) }
    }

    $pathContainsInstallDirectory = $false
    foreach ($entry in $pathEntries) {
        try {
            if ([IO.Path]::GetFullPath($entry).TrimEnd('\') -ieq $normalizedInstallDirectory) {
                $pathContainsInstallDirectory = $true
                break
            }
        }
        catch {
            if ($entry.TrimEnd('\') -ieq $normalizedInstallDirectory) {
                $pathContainsInstallDirectory = $true
                break
            }
        }
    }

    if (-not $pathContainsInstallDirectory) {
        $newUserPath = if ([string]::IsNullOrWhiteSpace($userPath)) {
            $installDirectory
        }
        else {
            "$userPath;$installDirectory"
        }
        [Environment]::SetEnvironmentVariable('Path', $newUserPath, 'User')
    }

    $env:Path = "$installDirectory;$env:Path"
    Write-Host "Stoker was installed to $installDirectory."
    Write-Host 'The install directory was added to your user PATH.'
    Write-Host "Try: stoker --version"
}
finally {
    if (Test-Path -LiteralPath $temporaryDirectory) {
        Remove-Item -LiteralPath $temporaryDirectory -Recurse -Force
    }
}
