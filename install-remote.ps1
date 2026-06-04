# Install agnes-aigc-gen from GitHub Releases (Windows).
# One-liner (PowerShell):
#   irm https://raw.githubusercontent.com/Zander-1024/agnes-aigc-gen/master/install-remote.ps1 | iex
$ErrorActionPreference = "Stop"

$Repo = if ($env:AGNES_AIGC_REPO) { $env:AGNES_AIGC_REPO } else { "Zander-1024/agnes-aigc-gen" }
$BinName = "agnes-aigc-gen"
$InstallBinDir = if ($env:INSTALL_BIN_DIR) { $env:INSTALL_BIN_DIR } else { Join-Path $env:USERPROFILE ".local\bin" }
$Platform = "windows-x86_64"

function Get-ReleaseVersion {
    if ($env:AGNES_AIGC_VERSION) {
        return ($env:AGNES_AIGC_VERSION -replace '^v', '')
    }
    $latest = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest"
    return ($latest.tag_name -replace '^v', '')
}

$Version = Get-ReleaseVersion
$Tag = "v$Version"
$Archive = "$BinName-$Version-$Platform.zip"
$BaseUrl = "https://github.com/$Repo/releases/download/$Tag"
$Tmp = Join-Path $env:TEMP "agnes-aigc-gen-install"
New-Item -ItemType Directory -Force -Path $Tmp, $InstallBinDir | Out-Null

Write-Host "==> Installing $BinName $Tag ($Platform)"
Invoke-WebRequest -Uri "$BaseUrl/$Archive" -OutFile (Join-Path $Tmp $Archive)
Expand-Archive -Path (Join-Path $Tmp $Archive) -DestinationPath $Tmp -Force
Copy-Item -Path (Join-Path $Tmp "$BinName.exe") -Destination (Join-Path $InstallBinDir "$BinName.exe") -Force

if ($env:SKIP_SKILL -ne "1") {
    $SkillScriptUrl = "https://raw.githubusercontent.com/$Repo/$Tag/scripts/install-skill.ps1"
    $SkillScriptPath = Join-Path $Tmp "install-skill.ps1"
    Invoke-WebRequest -Uri $SkillScriptUrl -OutFile $SkillScriptPath
    . $SkillScriptPath
    Install-SkillFromRemote -Repo $Repo -Tag $Tag
    Write-Host ""
    Write-SkillInstallSummary
}

Write-Host ""
Write-Host "Done."
Write-Host "  Binary: $(Join-Path $InstallBinDir "$BinName.exe")"
Write-Host "  Version: $Tag"
Write-Host ""
Write-Host "Add to PATH if needed (User):"
Write-Host "  $InstallBinDir"
Write-Host ""
Write-Host "Next steps:"
Write-Host "  $BinName config set api-key YOUR_API_KEY"
Write-Host "  $BinName config show"
