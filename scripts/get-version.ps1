[CmdletBinding()]
param()

$projectDirectory = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
$cargoTomlPath = Join-Path $projectDirectory 'Cargo.toml'
$cargoToml = [System.IO.File]::ReadAllText($cargoTomlPath)

$packageMatches = [regex]::Matches(
    $cargoToml,
    '(?ms)^\[package\]\r?\n(?:(?!^\[).)*'
)
if ($packageMatches.Count -ne 1) {
    throw "Expected exactly one [package] section in Cargo.toml, found $($packageMatches.Count)."
}

$package = $packageMatches[0].Value
$nameMatch = [regex]::Match($package, '(?m)^name\s*=\s*"([^"]+)"')
if (-not $nameMatch.Success -or $nameMatch.Groups[1].Value -ne 'stoker-engine') {
    throw "Cargo.toml does not describe the stoker-engine package."
}

$versionMatch = [regex]::Match($package, '(?m)^version\s*=\s*"([^"]+)"')
if (-not $versionMatch.Success) {
    throw "Could not find stoker-engine version in Cargo.toml."
}

$version = $versionMatch.Groups[1].Value
$semverPattern = '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?(\+[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$'
if ($version -notmatch $semverPattern) {
    throw "Invalid stoker-engine version '$version' in Cargo.toml."
}

Write-Output $version
