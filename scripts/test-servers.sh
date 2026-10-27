#!/bin/sh
# SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Starts one headless display server, runs its backend's end-to-end test
# against it, and stops it again.
#
#   sh scripts/test-servers.sh sway|xorg|kwin|mutter
#
# Needs the server installed: sway; Xorg with the dummy video driver
# (root, or SUDO=sudo); kwin_wayland; mutter with dbus-daemon and dbus-send.
# Each run gets its own XDG_RUNTIME_DIR, so it does not touch a running
# desktop session. POSIX sh on purpose (Steelbore Standard §7.1).

set -eu

server=${1:-}
case $server in
	sway | xorg | kwin | mutter) ;;
	*)
		echo "usage: sh scripts/test-servers.sh sway|xorg|kwin|mutter" >&2
		exit 2
		;;
esac

root=$(cd "$(dirname "$0")/.." && pwd)
runtime=$(mktemp -d "${TMPDIR:-/tmp}/vernier-$server.XXXXXX")
chmod 700 "$runtime"
XDG_RUNTIME_DIR=$runtime
export XDG_RUNTIME_DIR
log=$runtime/server.log
pids=

cleanup() {
	for pid in $pids; do
		${SUDO:-} kill "$pid" 2>/dev/null || true
	done
	if [ "${status:-1}" -ne 0 ] && [ -f "$log" ]; then
		echo "--- $server log (last 40 lines)" >&2
		tail -n 40 "$log" >&2 || true
	fi
	rm -rf "$runtime"
}
trap cleanup EXIT INT TERM

# wait_for <description> <command...>: retry the command for up to 30 s.
wait_for() {
	what=$1
	shift
	tries=0
	until "$@" >/dev/null 2>&1; do
		tries=$((tries + 1))
		if [ "$tries" -ge 60 ]; then
			echo "error: $server did not become ready ($what)" >&2
			exit 1
		fi
		sleep 0.5
	done
}

socket_exists() {
	[ -S "$1" ]
}

# first_socket <glob>: print the first Unix socket matching the glob.
first_socket() {
	for candidate in $1; do
		if [ -S "$candidate" ]; then
			echo "$candidate"
			return 0
		fi
	done
	return 1
}

run_test() {
	crate=$1
	name=$2
	(cd "$root" && cargo test -p "$crate" --locked --test "$name" -- --ignored)
}

case $server in
sway)
	WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 \
		sway -c /dev/null >"$log" 2>&1 &
	pids="$pids $!"
	wait_for "IPC socket" first_socket "$runtime/sway-ipc.*.sock"
	SWAYSOCK=$(first_socket "$runtime/sway-ipc.*.sock")
	export SWAYSOCK
	# The headless backend starts with one output; add a second.
	swaymsg create_output >/dev/null
	wait_for "Wayland socket" first_socket "$runtime/wayland-*"
	display=$(basename "$(first_socket "$runtime/wayland-*")")
	VNR_WLR_TEST_DISPLAY=$display run_test vnr-wlr sway_headless
	# Screen contents: screencopy everywhere, ext-image-copy-capture-v1
	# too on sway 1.11 and later.
	VNR_PREVIEW_TEST_DISPLAY=$display run_test vnr-gui sway_previews
	;;
xorg)
	number=${VNR_XORG_DISPLAY:-99}
	${SUDO:-} Xorg ":$number" -noreset -nolisten tcp \
		-config "$root/crates/vnr-x11/tests/dummy.conf" \
		-logfile "$runtime/xorg.log" >"$log" 2>&1 &
	pids="$pids $!"
	wait_for "X socket" socket_exists "/tmp/.X11-unix/X$number"
	VNR_X11_TEST_DISPLAY=":$number" run_test vnr-x11 xorg_dummy
	;;
kwin)
	# A copy drops the file capabilities kwin_wayland is installed with,
	# which a container or unprivileged runner cannot grant.
	cp "$(command -v kwin_wayland)" "$runtime/kwin_wayland"
	QT_QPA_PLATFORM=offscreen "$runtime/kwin_wayland" --virtual \
		--width 1920 --height 1080 --output-count 2 --socket vnr-kwin \
		>"$log" 2>&1 &
	pids="$pids $!"
	wait_for "Wayland socket" socket_exists "$runtime/vnr-kwin"
	sleep 1
	VNR_KDE_TEST_DISPLAY=vnr-kwin run_test vnr-kde kwin_virtual
	;;
mutter)
	# Mutter saves `monitors.xml` here; keep it away from the real one.
	XDG_CONFIG_HOME=$runtime/config
	export XDG_CONFIG_HOME
	mkdir -p "$XDG_CONFIG_HOME"
	bus=$(dbus-daemon --session --fork --print-address=1 --print-pid=1 >"$runtime/bus" && head -n 1 "$runtime/bus")
	pids="$pids $(sed -n 2p "$runtime/bus")"
	DBUS_SESSION_BUS_ADDRESS=$bus mutter --headless --wayland --no-x11 \
		--wayland-display vnr-mutter \
		--virtual-monitor 1920x1080 --virtual-monitor 1920x1080 >"$log" 2>&1 &
	pids="$pids $!"
	wait_for "DisplayConfig on the bus" env DBUS_SESSION_BUS_ADDRESS="$bus" \
		sh -c 'dbus-send --session --print-reply --dest=org.freedesktop.DBus / \
			org.freedesktop.DBus.NameHasOwner string:org.gnome.Mutter.DisplayConfig | grep -q true'
	VNR_MUTTER_TEST_BUS=$bus VNR_MUTTER_TEST_CONFIG=$XDG_CONFIG_HOME \
		run_test vnr-mutter mutter_headless
	;;
esac
status=0
