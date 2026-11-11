[CmdletBinding()]
param(
    [string]$Prefix = (Join-Path $env:LOCALAPPDATA 'Programs\Axiom Chronos'),
    [switch]$NoPath,
    [switch]$NoShortcut
)
$ErrorActionPreference = 'Stop'
$Prefix = [IO.Path]::GetFullPath($Prefix)
function Path-Key([string]$Value) {
    [Environment]::ExpandEnvironmentVariables($Value.Trim().Trim('"')).TrimEnd('\', '/').ToLowerInvariant()
}

foreach ($name in @('axiom.exe', 'chronos.exe')) {
    $target = Join-Path $Prefix $name
    if (Test-Path -LiteralPath $target) {
        try {
            $handle = [IO.File]::Open($target, 'Open', 'Write', 'None')
            $handle.Dispose()
        } catch {
            throw "Cannot remove $target. Close axiom.exe and chronos.exe, then retry. $($_.Exception.Message)"
        }
    }
}
if (-not $NoPath) {
    $key = Path-Key $Prefix
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    $parts = @($userPath -split ';' | Where-Object { $_ -and (Path-Key $_) -ne $key })
    [Environment]::SetEnvironmentVariable('Path', ($parts -join ';'), 'User')
    $env:Path = (@($env:Path -split ';' | Where-Object { $_ -and (Path-Key $_) -ne $key }) -join ';')
}
if (-not $NoShortcut) {
    $link = Join-Path ([Environment]::GetFolderPath('Programs')) 'Axiom Chronos.lnk'
    if (Test-Path -LiteralPath $link) {
        $shell = New-Object -ComObject WScript.Shell
        if ((Path-Key ($shell.CreateShortcut($link).TargetPath)) -eq (Path-Key (Join-Path $Prefix 'chronos.exe'))) {
            Remove-Item -LiteralPath $link
        }
    }
}
foreach ($name in @('axiom.exe', 'chronos.exe', 'uninstall.ps1')) {
    $target = Join-Path $Prefix $name
    if (Test-Path -LiteralPath $target) { Remove-Item -LiteralPath $target -Force }
}
if ((Test-Path -LiteralPath $Prefix) -and -not (Get-ChildItem -LiteralPath $Prefix -Force)) {
    Remove-Item -LiteralPath $Prefix
}
Write-Host "Removed Axiom Chronos from $Prefix. ~/.axiom and the Claude plugin were left alone."
