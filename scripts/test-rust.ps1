$ErrorActionPreference = 'Stop'

. (Join-Path $PSScriptRoot 'build-environment.ps1')
Enter-TimeAlbumBuildEnvironment

cargo test --manifest-path (Join-Path $PSScriptRoot '..\src-tauri\Cargo.toml')
exit $LASTEXITCODE
