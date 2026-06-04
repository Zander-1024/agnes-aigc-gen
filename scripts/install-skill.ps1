# Skill install helpers for install-remote.ps1
$script:SkillName = if ($env:SKILL_NAME) { $env:SKILL_NAME } else { "agnes-aigc-gen" }
$script:DefaultAgentsSkillRoot = if ($env:DEFAULT_AGENTS_SKILL_ROOT) {
    $env:DEFAULT_AGENTS_SKILL_ROOT
} else {
    Join-Path $env:USERPROFILE ".agents\skills"
}

function Get-SkillAgentParentDir {
    param([string]$Agent)
    switch ($Agent.ToLower()) {
        "agents" { return Join-Path $env:USERPROFILE ".agents\skills" }
        "cursor" { return Join-Path $env:USERPROFILE ".cursor\skills" }
        "claude" { return Join-Path $env:USERPROFILE ".claude\skills" }
        "codex" { return Join-Path $env:USERPROFILE ".codex\skills" }
        "openclaw" { return Join-Path $env:USERPROFILE ".openclaw\skills" }
        "hermes" { return Join-Path $env:USERPROFILE ".hermes\skills" }
        default { throw "unknown agent: $Agent (supported: agents,cursor,claude,codex,openclaw,hermes,all)" }
    }
}

function Get-SkillTargetDirs {
    if ($env:INSTALL_SKILL_DIR) {
        return @($env:INSTALL_SKILL_DIR)
    }

    $dirs = @($script:DefaultAgentsSkillRoot)
    $spec = $env:INSTALL_AGENTS
    if (-not $spec) { return $dirs }

    if ($spec.ToLower() -eq "all") {
        $spec = "cursor,claude,codex,openclaw,hermes"
    }

    foreach ($agent in ($spec -split ",")) {
        $agent = $agent.Trim().ToLower()
        if (-not $agent -or $agent -eq "agents") { continue }
        $parent = Get-SkillAgentParentDir $agent
        if ($dirs -notcontains $parent) { $dirs += $parent }
    }
    return $dirs
}

function Install-SkillFromRemote {
    param(
        [string]$Repo,
        [string]$Tag
    )
    foreach ($parent in (Get-SkillTargetDirs)) {
        $dest = Join-Path $parent $script:SkillName
        Write-Host "==> Installing skill to $dest"
        New-Item -ItemType Directory -Force -Path $dest | Out-Null
        foreach ($file in @("SKILL.md", "SETUP.md")) {
            $uri = "https://raw.githubusercontent.com/$Repo/$Tag/skills/$($script:SkillName)/$file"
            Invoke-WebRequest -Uri $uri -OutFile (Join-Path $dest $file)
        }
    }
}

function Write-SkillInstallSummary {
    Write-Host "Skill install targets:"
    foreach ($parent in (Get-SkillTargetDirs)) {
        Write-Host "  $(Join-Path $parent "$($script:SkillName)\SKILL.md")"
    }
    Write-Host ""
    Write-Host "Optional: INSTALL_AGENTS=cursor,claude,codex,openclaw,hermes or INSTALL_AGENTS=all"
}
