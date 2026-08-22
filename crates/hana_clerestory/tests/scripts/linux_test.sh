#!/bin/bash
# Launch Linux integration tests in XWayland Konsole
#
# Usage: ./linux_test.sh [single-monitor]
#
# Arguments:
#   single-monitor  Only run tests that work on a single monitor
#
# This script:
# 1. Launches Konsole in XWayland mode (required for xdotool position detection)
# 2. Runs Claude with the /test linux command
# 3. Claude will auto-move the terminal between monitors and run all tests

set -e

# Check if xdotool is available
if ! command -v xdotool &> /dev/null; then
    echo "Error: xdotool is required but not installed"
    echo "Install with: sudo dnf install xdotool"
    exit 1
fi

# Get the project directory (two levels up from this script)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"

# Check for single-monitor mode
SINGLE_MONITOR_FLAG=""
if [ "$1" = "single-monitor" ]; then
    SINGLE_MONITOR_FLAG=" single-monitor"
    echo "Single-monitor mode enabled"
fi

# Launch Konsole in XWayland mode with Claude running the test
# Using nohup and & to fully detach from parent shell
# --add-dir allows Claude to access the RON config directory
QT_QPA_PLATFORM=xcb nohup konsole -e bash -c "cd '$PROJECT_DIR' && unset CLAUDECODE && claude '/test${SINGLE_MONITOR_FLAG}' --add-dir ~/.config/restore_window" &>/dev/null &
KONSOLE_PID=$!

# Resize-and-move workaround: on KDE Wayland HiDPI, XWayland Konsole launches
# with a 1x1 invisible window and ignores --geometry. xdotool can see the
# window and forcibly resize/reposition it after Konsole maps it.
# Coords inside eDP-1's XWayland range (laptop screen).
(
    # Wait up to 5s for the Konsole main window to appear
    for _ in $(seq 1 50); do
        WID=$(WAYLAND_DISPLAY= xdotool search --pid "$KONSOLE_PID" --class "konsole" 2>/dev/null | tail -1)
        [ -n "$WID" ] && break
        sleep 0.1
    done
    if [ -n "$WID" ]; then
        WAYLAND_DISPLAY= xdotool windowsize "$WID" 1400 900 2>/dev/null || true
        WAYLAND_DISPLAY= xdotool windowmove "$WID" 2200 3200 2>/dev/null || true
    fi
) &

echo "Launched Linux test runner in XWayland Konsole"
echo "The test will run autonomously - check the new Konsole window"
