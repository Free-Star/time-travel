$ErrorActionPreference = 'Stop'

. (Join-Path $PSScriptRoot 'build-environment.ps1')
Enter-TimeAlbumBuildEnvironment

pnpm tauri dev
exit $LASTEXITCODE
