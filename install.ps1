# MorphArch installer for Windows
# Usage: irm https://raw.githubusercontent.com/onplt/morpharch/main/install.ps1 | iex
#
# Parameters (via environment variables when piped):
#   $env:MORPHARCH_VERSION   - specific version (default: latest)
#   $env:MORPHARCH_INSTALL   - install directory (default: $HOME\.morpharch\bin)

$ErrorActionPreference = "Stop"
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$Repo = "onplt/morpharch"
$Binary = "morpharch.exe"
$AssetName = "morpharch-windows-x86_64.zip"

# ── Determine version ──

if ($env:MORPHARCH_VERSION) {
    $Version = $env:MORPHARCH_VERSION
} else {
    Write-Host "Fetching latest version..."
    try {
        $Release = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest"
        $Version = $Release.tag_name -replace '^v', ''
    } catch {
        Write-Error "Could not determine latest version: $_"
        exit 1
    }
}

if (-not $Version) {
    Write-Error "Version could not be determined."
    exit 1
}

# ── Setup paths ──

if ($env:MORPHARCH_INSTALL) {
    $InstallDir = $env:MORPHARCH_INSTALL
} else {
    $InstallDir = Join-Path $env:USERPROFILE ".morpharch\bin"
}

$DownloadUrl = "https://github.com/$Repo/releases/download/v$Version/$AssetName"
$ChecksumUrl = "https://github.com/$Repo/releases/download/v$Version/SHA256SUMS.txt"

Write-Host "Installing morpharch v$Version (windows/x86_64)..."
Write-Host "  From: $DownloadUrl"
Write-Host "  To:   $InstallDir\$Binary"

# ── Download ──

$TmpDir = Join-Path ([System.IO.Path]::GetTempPath()) ("morpharch-install-" + [System.Guid]::NewGuid().ToString("N").Substring(0, 8))
New-Item -ItemType Directory -Path $TmpDir -Force | Out-Null

try {
    $ZipPath = Join-Path $TmpDir $AssetName

    Write-Host "Downloading..."
    Invoke-WebRequest -Uri $DownloadUrl -OutFile $ZipPath -UseBasicParsing

    # ── Verify checksum ──

    try {
        $ChecksumPath = Join-Path $TmpDir "SHA256SUMS.txt"
        Invoke-WebRequest -Uri $ChecksumUrl -OutFile $ChecksumPath -UseBasicParsing

        $Checksums = Get-Content $ChecksumPath
        $ExpectedLine = $Checksums | Where-Object { $_ -match $AssetName }
        if ($ExpectedLine) {
            $Expected = ($ExpectedLine -split '\s+')[0]
            $Actual = (Get-FileHash -Path $ZipPath -Algorithm SHA256).Hash.ToLower()

            if ($Actual -ne $Expected) {
                Write-Error "Checksum verification failed!`n  Expected: $Expected`n  Actual:   $Actual"
                exit 1
            }
            Write-Host "Checksum verified."
        }
    } catch {
        Write-Warning "Could not verify checksum, skipping verification."
    }

    # ── Extract ──

    Write-Host "Extracting..."
    $ExtractDir = Join-Path $TmpDir "extract"
    Expand-Archive -Path $ZipPath -DestinationPath $ExtractDir -Force

    # ── Install ──

    if (-not (Test-Path $InstallDir)) {
        New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    }

    $SourceBinary = Join-Path $ExtractDir $Binary
    if (-not (Test-Path $SourceBinary)) {
        # Try looking in subdirectories
        $SourceBinary = Get-ChildItem -Path $ExtractDir -Filter $Binary -Recurse | Select-Object -First 1 -ExpandProperty FullName
    }

    Copy-Item -Path $SourceBinary -Destination (Join-Path $InstallDir $Binary) -Force

    Write-Host ""
    Write-Host "morpharch v$Version installed successfully to $InstallDir\$Binary" -ForegroundColor Green

    # ── Update PATH ──

    $UserPath = [Environment]::GetEnvironmentVariable("Path", [EnvironmentVariableTarget]::User)
    if ($UserPath -notlike "*$InstallDir*") {
        Write-Host ""
        Write-Host "Adding $InstallDir to your PATH..."
        $NewPath = "$InstallDir;$UserPath"
        [Environment]::SetEnvironmentVariable("Path", $NewPath, [EnvironmentVariableTarget]::User)
        $env:Path = "$InstallDir;$env:Path"
        Write-Host "PATH updated. Restart your terminal for changes to take effect."
    }

    Write-Host ""
    Write-Host "Run 'morpharch --version' to verify."

} finally {
    # Cleanup
    if (Test-Path $TmpDir) {
        Remove-Item -Path $TmpDir -Recurse -Force -ErrorAction SilentlyContinue
    }
}
