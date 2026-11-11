[CmdletBinding()]
param(
    [string]$Prefix = (Join-Path $env:LOCALAPPDATA 'Programs\Axiom Chronos'),
    [switch]$NoPath,
    [switch]$NoShortcut,
    [switch]$NoPlugin
)
$ErrorActionPreference = 'Stop'
$Prefix = [IO.Path]::GetFullPath($Prefix)

function Path-Key([string]$Value) {
    [Environment]::ExpandEnvironmentVariables($Value.Trim().Trim('"')).TrimEnd('\', '/').ToLowerInvariant()
}

# Check both destinations before changing either binary. Windows locks running EXEs.
foreach ($name in @('axiom.exe', 'chronos.exe', 'uninstall.ps1')) {
    $source = Join-Path $PSScriptRoot $name
    $target = Join-Path $Prefix $name
    if (-not (Test-Path -LiteralPath $source -PathType Leaf)) {
        throw "Missing package file: $source"
    }
    if ((Path-Key $source) -eq (Path-Key $target)) {
        throw 'Extract the package outside the installation folder before installing.'
    }
    if (Test-Path -LiteralPath $target) {
        try {
            $handle = [IO.File]::Open($target, 'Open', 'Write', 'None')
            $handle.Dispose()
        } catch {
            throw "Cannot replace $target. Close axiom.exe and chronos.exe, then retry. Check folder permissions if neither is running. $($_.Exception.Message)"
        }
    }
}
New-Item -ItemType Directory -Path $Prefix -Force | Out-Null
foreach ($name in @('axiom.exe', 'chronos.exe', 'uninstall.ps1')) {
    try {
        Copy-Item -LiteralPath (Join-Path $PSScriptRoot $name) -Destination (Join-Path $Prefix $name) -Force
    } catch {
        throw "Cannot replace $name in $Prefix. Close axiom.exe and chronos.exe and check folder permissions, then retry. $($_.Exception.Message)"
    }
}

if (-not $NoPath) {
    $key = Path-Key $Prefix
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    $parts = @($userPath -split ';' | Where-Object { $_ -and (Path-Key $_) -ne $key })
    # First in the user PATH, so an older axiom (for example from cargo install) does not win.
    [Environment]::SetEnvironmentVariable('Path', ((@($Prefix) + $parts) -join ';'), 'User')
    $parts = @($env:Path -split ';' | Where-Object { $_ -and (Path-Key $_) -ne $key })
    $env:Path = (@($Prefix) + $parts) -join ';'
    $found = Get-Command axiom -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($found -and (Path-Key (Split-Path $found.Source)) -ne $key) {
        Write-Warning "Another axiom at $($found.Source) comes first on PATH (probably the system PATH). Remove it, or run axiom from $Prefix."
    }
}
if (-not $NoShortcut) {
    $programs = [Environment]::GetFolderPath('Programs')
    $shell = New-Object -ComObject WScript.Shell
    $shortcut = $shell.CreateShortcut((Join-Path $programs 'Axiom Chronos.lnk'))
    $shortcut.TargetPath = Join-Path $Prefix 'chronos.exe'
    $shortcut.WorkingDirectory = $Prefix
    $shortcut.Save()
}

function Invoke-Claude([string[]]$CliArgs) {
    $output = & claude @CliArgs
    if ($LASTEXITCODE -ne 0) { throw "claude $($CliArgs -join ' ') failed (exit $LASTEXITCODE)." }
    $output
}

$pluginFailed = $false
if (-not $NoPlugin -and (Get-Command claude -ErrorAction SilentlyContinue)) {
    try {
        $markets = @(Invoke-Claude -CliArgs @('plugin', 'marketplace', 'list', '--json') | ConvertFrom-Json)
        if ($markets | Where-Object { $_.name -eq 'axiom-chronos' }) {
            Invoke-Claude -CliArgs @('plugin', 'marketplace', 'update', 'axiom-chronos')
        } else {
            Invoke-Claude -CliArgs @('plugin', 'marketplace', 'add', 'xinhi6716/axiom-chronos')
        }
        $plugins = @(Invoke-Claude -CliArgs @('plugin', 'list', '--json') | ConvertFrom-Json)
        if ($plugins | Where-Object { $_.id -eq 'chronos@axiom-chronos' -and $_.scope -eq 'user' }) {
            Invoke-Claude -CliArgs @('plugin', 'update', 'chronos@axiom-chronos', '--scope', 'user')
        } else {
            Invoke-Claude -CliArgs @('plugin', 'install', 'chronos@axiom-chronos', '--scope', 'user')
        }
    } catch {
        $pluginFailed = $true
        Write-Warning "Programs installed, but the Claude plugin setup failed: $_"
    }
}

Write-Host "Installed Axiom Chronos in $Prefix"
Write-Host 'Health check (axiom doctor):'
try {
    & (Join-Path $Prefix 'axiom.exe') doctor
    if ($LASTEXITCODE -ne 0) { Write-Warning "axiom doctor reported problems (exit $LASTEXITCODE). The programs are installed." }
} catch {
    Write-Warning "Could not run axiom doctor: $_. The programs are installed."
}
if (-not $NoPath) { Write-Host 'Open a new terminal to use the updated user PATH.' }
if ($pluginFailed) { exit 1 }
exit 0
