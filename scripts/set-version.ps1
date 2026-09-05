[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Version
)

$semverPattern = '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?(\+[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$'
if ($Version -notmatch $semverPattern) {
    throw "Invalid version '$Version'. Expected a semantic version such as 1.2.2."
}

$projectDirectory = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
$cargoTomlPath = Join-Path $projectDirectory 'Cargo.toml'
$cargoLockPath = Join-Path $projectDirectory 'Cargo.lock'
$packageName = 'stoker-engine'

$cargoToml = [System.IO.File]::ReadAllText($cargoTomlPath)
$cargoLock = [System.IO.File]::ReadAllText($cargoLockPath)

$tomlPattern = '(?ms)(^\[package\]\r?\n(?:(?!^\[).)*?^version\s*=\s*")[^"]+(")'
$lockPattern = '(?ms)(^\[\[package\]\]\r?\nname\s*=\s*"' + [regex]::Escape($packageName) + '"\r?\nversion\s*=\s*")[^"]+(")'

$tomlMatches = [regex]::Matches($cargoToml, $tomlPattern)
$lockMatches = [regex]::Matches($cargoLock, $lockPattern)
if ($tomlMatches.Count -ne 1) {
    throw "Expected exactly one [package] version in Cargo.toml, found $($tomlMatches.Count)."
}
if ($lockMatches.Count -ne 1) {
    throw "Expected exactly one $packageName package version in Cargo.lock, found $($lockMatches.Count)."
}

$tomlMatch = $tomlMatches[0]
$lockMatch = $lockMatches[0]
$tomlReplacement = $tomlMatch.Groups[1].Value + $Version + $tomlMatch.Groups[2].Value
$lockReplacement = $lockMatch.Groups[1].Value + $Version + $lockMatch.Groups[2].Value
$updatedCargoToml = $cargoToml.Replace($tomlMatch.Value, $tomlReplacement)
$updatedCargoLock = $cargoLock.Replace($lockMatch.Value, $lockReplacement)

$utf8NoBom = New-Object System.Text.UTF8Encoding($false)
[System.IO.File]::WriteAllText($cargoTomlPath, $updatedCargoToml, $utf8NoBom)
[System.IO.File]::WriteAllText($cargoLockPath, $updatedCargoLock, $utf8NoBom)

Write-Host "Updated $packageName version to $Version in Cargo.toml and Cargo.lock."
