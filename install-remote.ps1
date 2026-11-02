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
    try {
        $latest = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest" -TimeoutSec 20
        $tag = [string]$latest.tag_name
        if ($tag -match '^v[0-9]+\.[0-9]+\.[0-9]+$') {
            return $tag.Substring(1)
        }
    } catch {
        # Public API requests may be rate limited; the release page is independent.
    }
    try {
        $response = Invoke-WebRequest -Uri "https://github.com/$Repo/releases/latest" -Method Head -MaximumRedirection 10 -TimeoutSec 20 -UseBasicParsing
        $baseResponse = $response.BaseResponse
        $finalUri = $null
        if ($baseResponse.PSObject.Properties['ResponseUri']) {
            $finalUri = $baseResponse.ResponseUri
        } elseif ($baseResponse.PSObject.Properties['RequestMessage']) {
            $finalUri = $baseResponse.RequestMessage.RequestUri
        }
        $releasePrefix = "https://github.com/$Repo/releases/tag/"
        if ($finalUri -and $finalUri.AbsoluteUri.StartsWith($releasePrefix, [System.StringComparison]::Ordinal)) {
            $tag = $finalUri.AbsoluteUri.Substring($releasePrefix.Length)
            if ($tag -match '^v[0-9]+\.[0-9]+\.[0-9]+$') {
                return $tag.Substring(1)
            }
        }
    } catch {
        # A pinned version still allows installation when both lookup paths fail.
    }
    throw 'Unable to resolve the latest GitHub release. Retry later or set $env:AGNES_AIGC_VERSION to a known release version.'
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
