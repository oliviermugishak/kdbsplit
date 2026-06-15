#!/usr/bin/env bash
set -euo pipefail

APP_NAME="KbdSplit"
BIN_DIR="/usr/local/bin"
DESKTOP_DIR="/usr/local/share/applications"
ICON_DIR="/usr/local/share/icons/hicolor/scalable/apps"
UDEV_RULES_DIR="/etc/udev/rules.d"
SYSTEMD_DIR="/etc/systemd/system"
BUILD_PROFILE="release"
REPO_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
TARGET_DIR="$REPO_DIR/target/$BUILD_PROFILE"
INSTALL_USER="${SUDO_USER:-$USER}"

usage() {
  cat <<EOF
Install $APP_NAME.

Usage:
  ./install.sh [--debug] [--no-build] [--no-input-group] [--with-systemd]

Options:
  --debug           Install debug binaries from target/debug.
  --no-build        Do not run cargo build before installing.
  --no-input-group  Do not add the current user to the input group.
  --with-systemd    Install and enable kbdsplitd as a systemd service.
  -h, --help        Show this help.
EOF
}

need_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "error: required command not found: $1" >&2
    exit 1
  fi
}

as_root() {
  if [[ "${EUID:-$(id -u)}" -eq 0 ]]; then
    "$@"
  else
    sudo "$@"
  fi
}

install_icon() {
  local svg_src="$REPO_DIR/assets/icon.svg"
  local png_src="$REPO_DIR/assets/icon.png"
  if [[ -f "$svg_src" ]]; then
    local dir="$ICON_DIR"
    echo "Installing app icon (SVG)"
    as_root install -d -m 0755 "$dir"
    as_root install -m 0644 "$svg_src" "$dir/dev.kbdsplit.KbdSplit.svg"
  fi
  if [[ -f "$png_src" ]]; then
    local dir="/usr/local/share/icons/hicolor/128x128/apps"
    echo "Installing app icon (PNG)"
    as_root install -d -m 0755 "$dir"
    as_root install -m 0644 "$png_src" "$dir/dev.kbdsplit.KbdSplit.png"
  fi
}

install_systemd_service() {
  local unit="kbdsplitd.service"
  local src="$REPO_DIR/packaging/systemd/$unit"
  local dst="$SYSTEMD_DIR/$unit"
  echo "Installing systemd service"
  as_root install -d -m 0755 "$SYSTEMD_DIR"
  as_root install -m 0644 "$src" "$dst"
  as_root systemctl daemon-reload
  as_root systemctl enable "$unit"
  as_root systemctl start "$unit" || true
}

main() {
  local do_build=1
  local add_input_group=1
  local with_systemd=0

  while [[ $# -gt 0 ]]; do
    case "$1" in
      --debug)
        BUILD_PROFILE="debug"
        TARGET_DIR="$REPO_DIR/target/debug"
        ;;
      --no-build)
        do_build=0
        ;;
      --no-input-group)
        add_input_group=0
        ;;
      --with-systemd)
        with_systemd=1
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

  need_cmd cargo
  need_cmd install

  if [[ "$do_build" -eq 1 ]]; then
    if [[ "$BUILD_PROFILE" == "release" ]]; then
      cargo build --workspace --release
    else
      cargo build --workspace
    fi
  fi

  if [[ ! -x "$TARGET_DIR/kbdsplitd" || ! -x "$TARGET_DIR/kbdsplit-gui" ]]; then
    echo "error: expected binaries not found in $TARGET_DIR" >&2
    echo "run cargo build --workspace --release first, or use ./install.sh --debug" >&2
    exit 1
  fi

  echo "Installing binaries to $BIN_DIR"
  as_root install -d -m 0755 "$BIN_DIR"
  as_root install -m 0755 "$TARGET_DIR/kbdsplitd" "$BIN_DIR/kbdsplitd"
  as_root install -m 0755 "$TARGET_DIR/kbdsplit-gui" "$BIN_DIR/kbdsplit-gui"

  echo "Installing desktop launcher"
  as_root install -d -m 0755 "$DESKTOP_DIR"
  as_root install -m 0644 \
    "$REPO_DIR/packaging/desktop/dev.kbdsplit.KbdSplit.desktop" \
    "$DESKTOP_DIR/dev.kbdsplit.KbdSplit.desktop"

  install_icon

  echo "Installing udev rules"
  as_root install -d -m 0755 "$UDEV_RULES_DIR"
  as_root install -m 0644 \
    "$REPO_DIR/packaging/udev/70-kbdsplit.rules" \
    "$UDEV_RULES_DIR/70-kbdsplit.rules"

  if [[ "$with_systemd" -eq 1 ]]; then
    install_systemd_service
  fi

  if [[ "$add_input_group" -eq 1 ]]; then
    if ! getent group input >/dev/null 2>&1; then
      echo "Creating input group"
      as_root groupadd --system input
    fi

    if id -nG "$INSTALL_USER" | tr ' ' '\n' | grep -qx input; then
      echo "User $INSTALL_USER is already in the input group"
    else
      echo "Adding $INSTALL_USER to the input group"
      as_root usermod -aG input "$INSTALL_USER"
      echo "You must log out and back in for input group permissions to apply."
    fi
  fi

  if command -v udevadm >/dev/null 2>&1; then
    echo "Reloading udev rules"
    as_root udevadm control --reload-rules || true
    as_root udevadm trigger --subsystem-match=input || true
  else
    echo "udevadm was not found; reconnect keyboards or reboot after installing rules."
  fi

  if command -v update-desktop-database >/dev/null 2>&1; then
    as_root update-desktop-database "$DESKTOP_DIR" || true
  fi

  cat <<EOF

$APP_NAME installed.

Run it with:
  kbdsplit-gui

If keyboards or /dev/uinput are not visible, log out and back in, then reconnect keyboards.
To uninstall:
  ./uninstall.sh
EOF
}

main "$@"
