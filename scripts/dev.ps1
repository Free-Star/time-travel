$ErrorActionPreference = 'Stop'

. (Join-Path $PSScriptRoot 'build-environment.ps1')
Enter-TimeAlbumBuildEnvironment

corepack pnpm tauri dev
exit $LASTEXITCODE
