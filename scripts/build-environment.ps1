function Enter-TimeAlbumBuildEnvironment {
    $projectDrive = [System.IO.Path]::GetPathRoot($PSScriptRoot)
    $env:CARGO_TARGET_DIR = Join-Path $projectDrive 'TimeAlbumBuild\time-album'
    # Windows linker can retain incompatible LLVM objects in Cargo's incremental
    # cache. Keep project builds deterministic across every startup entry point.
    $env:CARGO_INCREMENTAL = '0'

    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    if (-not (Test-Path -LiteralPath $vswhere)) {
        throw 'Visual Studio Installer was not found.'
    }

    $installPath = & $vswhere `
        -latest `
        -products * `
        -requires Microsoft.VisualStudio.Workload.NativeDesktop `
        -property installationPath

    if (-not $installPath) {
        throw 'The Desktop development with C++ workload is not installed.'
    }

    $devShellModule = Join-Path $installPath 'Common7\Tools\Microsoft.VisualStudio.DevShell.dll'
    Import-Module $devShellModule
    Enter-VsDevShell `
        -VsInstallPath $installPath `
        -SkipAutomaticLocation `
        -DevCmdArguments '-arch=x64 -host_arch=x64'
}
