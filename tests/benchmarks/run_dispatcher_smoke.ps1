param(
    [string]$Exe = "$env:TEMP/chronos-assembler-target/debug/axiom.exe",
    [string]$Out = "E:/codex/chronos-audit-20261031/dispatcher-repair-live-smoke"
)
$ErrorActionPreference = 'Stop'
$harness = Join-Path $PSScriptRoot 'dispatcher_smoke.py'
if (-not (Test-Path -LiteralPath $Exe -PathType Leaf)) { throw "Missing verified binary: $Exe" }
# The Python harness copies Exe to Out before execution; never runs a rebuild target in place.
python $harness --exe $Exe --out $Out
exit $LASTEXITCODE
