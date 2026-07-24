$ErrorActionPreference = 'Stop'

$corepackCommand = Get-Command corepack.cmd -ErrorAction SilentlyContinue
$corepackPath = if ($corepackCommand) { $corepackCommand.Source } else { $null }

if (-not $corepackPath) {
    $nodeCommand = Get-Command node.exe -ErrorAction SilentlyContinue
    if ($nodeCommand) {
        $candidate = Join-Path (Split-Path -Parent $nodeCommand.Source) 'corepack.cmd'
        if (Test-Path -LiteralPath $candidate) {
            $corepackPath = $candidate
        }
    }
}

if (-not $corepackPath) {
    $candidates = @(
        'C:\Soft\Environment\nodejs\corepack.cmd',
        (Join-Path $env:ProgramFiles 'nodejs\corepack.cmd')
    )
    foreach ($candidate in $candidates) {
        if (Test-Path -LiteralPath $candidate) {
            $corepackPath = $candidate
            break
        }
    }
}

if (-not $corepackPath) {
    throw 'Corepack was not found. Install Node.js with Corepack, then run start.cmd again.'
}

$nodeDirectory = Split-Path -Parent $corepackPath
$env:Path = "$nodeDirectory;$env:Path"

& $corepackPath pnpm desktop:dev
exit $LASTEXITCODE
