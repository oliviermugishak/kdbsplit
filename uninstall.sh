#!/usr/bin/env bash
set -euo pipefail

BIN_DIR="/usr/local/bin"
DESKTOP_FILE="/usr/local/share/applications/dev.kbdsplit.KbdSplit.desktop"
ICON_SVG="/usr/local/share/icons/hicolor/scalable/apps/dev.kbdsplit.KbdSplit.svg"
ICON_PNG="/usr/local/share/icons/hicolor/128x128/apps/dev.kbdsplit.KbdSplit.png"
UDEV_RULE="/etc/udev/rules.d/70-kbdsplit.rules"
SYSTEMD_UNIT="/etc/systemd/system/kbdsplitd.service"
SOCKET="/tmp/kbdsplit.sock"
PURGE_CONFIG=0
INSTALL_USER="${SUDO_USER:-$USER}"

usage() {
  cat <<EOF
Uninstall KbdSplit.

Usage:
  ./uninstall.sh [--purge-config]

Options:
  --purge-config  Also remove this user's KbdSplit profile/config directory.
  -h, --help      Show this help.
EOF
}

as_root() {
  if [[ "${EUID:-$(id -u)}" -eq 0 ]]; then
    "$@"
  else
    sudo "$@"
  fi
}

main() {
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --purge-config)
        PURGE_CONFIG=1
        ;;
      -h|--help)
        usage
        exit 0
        ;;
      *)
        echo "error: unknown option: $1" >&2
        usage
        exit 1
        ;;
    esac
    shift
  done

  echo "Stopping running daemon if present"
  pkill -x kbdsplitd >/dev/null 2>&1 || true

  # Stop, disable, and remove systemd service if installed
  if [[ -f "$SYSTEMD_UNIT" ]]; then
    echo "Removing systemd service"
    as_root systemctl stop kbdsplitd 2>/dev/null || true
    as_root systemctl disable kbdsplitd 2>/dev/null || true
    as_root rm -f "$SYSTEMD_UNIT"
    as_root systemctl daemon-reload 2>/dev/null || true
  fi

  echo "Removing installed files"
  as_root rm -f "$BIN_DIR/kbdsplitd" "$BIN_DIR/kbdsplit-gui"
  as_root rm -f "$DESKTOP_FILE"
  as_root rm -f "$ICON_SVG" "$ICON_PNG"
  as_root rm -f "$UDEV_RULE"
  rm -f "$SOCKET"

  # Remove user home desktop files
  local user_home
  user_home="$(getent passwd "$INSTALL_USER" | cut -d: -f6)"
  if [[ -n "$user_home" ]] && [[ -d "$user_home" ]]; then
    rm -f "$user_home/KbdSplit.desktop"
    rm -f "$user_home/.local/share/applications/dev.kbdsplit.KbdSplit.desktop"
  fi

  if command -v udevadm >/dev/null 2>&1; then
    echo "Reloading udev rules"
    as_root udevadm control --reload-rules || true
    as_root udevadm trigger --subsystem-match=input || true
  fi

  if command -v update-desktop-database >/dev/null 2>&1; then
    as_root update-desktop-database /usr/local/share/applications || true
  fi

  if [[ "$PURGE_CONFIG" -eq 1 ]]; then
    local config_home="${XDG_CONFIG_HOME:-}"
    if [[ -z "$config_home" ]]; then
      local home_dir
      home_dir="$(getent passwd "$INSTALL_USER" | cut -d: -f6)"
      config_home="$home_dir/.config"
    fi
    echo "Removing user config from $config_home/kbdsplit"
    rm -rf "$config_home/kbdsplit"
  fi

  cat <<EOF

KbdSplit uninstalled.

Note: uninstall does not remove $INSTALL_USER from the input group.
Group membership may be used by other input tools, so remove it manually only if you are sure.
EOF
}

main "$@"
