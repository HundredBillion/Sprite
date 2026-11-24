# Builds Sprite from this checkout and installs it for the current user.
#
#   packaging\windows\update.ps1
#
# Installs sprite.exe (GUI subsystem, no console flash from Start Menu) and
# sprite.com (console shim prioritized by %PATHEXT% for CLI commands) side by
# side into %LOCALAPPDATA%\Programs\Sprite, updates the user Path, and writes a
# Start Menu shortcut.

$ErrorActionPreference = "Stop"

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
Push-Location $repoRoot
try {
    if (-not $env:GPUI_FXC_PATH -and -not (Get-Command fxc.exe -ErrorAction SilentlyContinue)) {
        $kitsBin = Join-Path ${env:ProgramFiles(x86)} "Windows Kits\10\bin"
        if (Test-Path $kitsBin) {
            $fxc = Get-ChildItem -Path $kitsBin -Filter fxc.exe -Recurse -ErrorAction SilentlyContinue |
                Where-Object { $_.Directory.Name -eq "x64" } |
                Sort-Object FullName -Descending |
                Select-Object -First 1
            if ($fxc) {
                $env:GPUI_FXC_PATH = $fxc.FullName
            }
        }
    }

    Write-Host "==> building release binary"
    cargo build --release -p sprite-app --locked --offline
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    Write-Host "==> building console shim"
    rustc --edition 2024 -O -o target\release\sprite.com packaging\windows\sprite-shim.rs
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    $installDir = Join-Path $env:LOCALAPPDATA "Programs\Sprite"
    Write-Host "==> installing to $installDir"
    New-Item -ItemType Directory -Force -Path $installDir | Out-Null

    try {
        Copy-Item -Path target\release\sprite.exe -Destination (Join-Path $installDir "sprite.exe") -Force
        Copy-Item -Path target\release\sprite.com -Destination (Join-Path $installDir "sprite.com") -Force
    } catch {
        Write-Error "Failed to overwrite Sprite binaries in $installDir. Close any running Sprite windows and retry. Details: $($_.Exception.Message)"
        exit 1
    }

    foreach ($notice in @("LICENSE-MIT", "LICENSE-APACHE", "THIRD-PARTY-NOTICES.md")) {
        Copy-Item -Path $notice -Destination (Join-Path $installDir $notice) -Force
    }

    $normalizedInstall = $installDir.TrimEnd('\')
    $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
    $entries = @()
    if ($userPath) {
        $entries = $userPath -split ';' | Where-Object { $_ -and $_.Trim() -ne "" }
    }
    $alreadyInPath = $entries | Where-Object { $_.Trim().TrimEnd('\') -ieq $normalizedInstall }
    if (-not $alreadyInPath) {
        $updatedPath = ($entries + $installDir) -join ';'
        [Environment]::SetEnvironmentVariable("Path", $updatedPath, "User")
        Write-Host "==> added $installDir to user Path"
    }
    if (-not (($env:Path -split ';') | Where-Object { $_.Trim().TrimEnd('\') -ieq $normalizedInstall })) {
        $env:Path = "$installDir;$env:Path"
    }

    $startMenuDir = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs"
    New-Item -ItemType Directory -Force -Path $startMenuDir | Out-Null
    $shortcutPath = Join-Path $startMenuDir "Sprite.lnk"
    $wscript = New-Object -ComObject WScript.Shell
    $shortcut = $wscript.CreateShortcut($shortcutPath)
    $shortcut.TargetPath = Join-Path $installDir "sprite.exe"
    $shortcut.WorkingDirectory = $env:USERPROFILE
    $shortcut.IconLocation = "$(Join-Path $installDir 'sprite.exe'),0"
    $shortcut.Description = "Sprite Terminal"
    $shortcut.Save()

    $installedVersion = (& (Join-Path $installDir "sprite.com") --version | Select-Object -First 1)
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    Write-Host "==> installed: $installDir ($installedVersion)"
} finally {
    Pop-Location
}
