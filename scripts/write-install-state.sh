#!/usr/bin/env bash
# Write install.toml for agnes-aigc-gen (sourced by install scripts).

write_install_state() {
  local binary_path="$1"
  local installed_version="$2"
  local config_dir

  if [[ -z "$binary_path" || -z "$installed_version" ]]; then
    echo "write_install_state: binary path and version are required" >&2
    return 1
  fi

  if [[ "$(uname -s)" == "Darwin" ]]; then
    config_dir="${HOME}/Library/Application Support/agnes-aigc-gen"
  else
    config_dir="${XDG_CONFIG_HOME:-${HOME}/.config}/agnes-aigc-gen"
  fi
  mkdir -p "$config_dir"

  {
    echo "version = 1"
    echo "installed_version = \"${installed_version}\""
    echo "binary_path = \"${binary_path}\""
    if [[ "${SKIP_SKILL:-0}" != "1" ]]; then
      while IFS= read -r parent; do
        [[ -z "$parent" ]] && continue
        echo "[[skill_targets]]"
        echo "parent_dir = \"${parent}\""
      done < <(skill_resolve_target_dirs)
    fi
  } > "${config_dir}/install.toml"

  echo "==> Wrote ${config_dir}/install.toml"
}
