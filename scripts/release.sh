#!/bin/bash

set -o errexit
set -o nounset
set -o pipefail

PWD="$(dirname "$(readlink -f -- "$0")")"

CHANNEL=""
DO_SIGN="false"
VERSION=""
TARGET="${CARGO_BUILD_TARGET:-}"

function help() {
	local to
	to="$1"

	echo "Usage: $0 <flags>" 1>&"$to"
	echo 1>&"$to"
	echo "flags:" 1>&"$to"
	echo "	--version											release version." 1>&"$to"
	echo "	--dist												path to store artifacts in." 1>&"$to"
	echo "	--sign												if set, will sign the app." 1>&"$to"
	echo "	--channel											the channel to use for the release (release | nightly)." 1>&"$to"
	echo "	--help												display this message." 1>&"$to"
}

function error() {
	echo "error: $*" 1>&2
	echo 1>&2
	help 2
	exit 1
}

function info() {
	echo "$@"
}

function os() {
	local os
	os="$(uname -s)"
	case "$os" in
	Darwin)
		echo "macos"
		;;
	Linux)
		echo "linux"
		;;
	Windows | MSYS* | MINGW*)
		echo "windows"
		;;
	*)
		error "$os: unsupported"
		;;
	esac
}

function arch() {
	local arch

	# If TARGET is specified, extract architecture from it
	if [ -n "${TARGET:-}" ]; then
		case "$TARGET" in
		*aarch64* | *arm64*)
			echo "aarch64"
			return
			;;
		*x86_64* | *amd64*)
			echo "x86_64"
			return
			;;
		esac
	fi

	# Otherwise, detect from system
	arch="$(uname -m)"
	case "$arch" in
	arm64 | aarch64)
		echo "aarch64"
		;;
	x86_64)
		echo "x86_64"
		;;
	*)
		error "$arch: unsupported architecture"
		;;
	esac
}

ARCH="$(arch)"
OS="$(os)"
DIST="release"

# the OS is used for certain build decisions and signing
export OS

function tauri() {
	(cd "$PWD/.." && pnpm tauri-for-release "$@")
}

while [[ $# -gt 0 ]]; do
	case "$1" in
	--help)
		help 1
		exit 1
		;;
	--version)
		VERSION="$2"
		shift
		shift
		;;
	--dist)
		DIST="$2"
		shift
		shift
		;;
	--sign)
		DO_SIGN="true"
		shift
		;;
	--channel)
		CHANNEL="$2"
		shift
		shift
		;;
	*)
		error "unknown flag $1"
		;;
	esac
done

# Recalculate ARCH after TARGET is set
ARCH="$(arch)"

[ -z "${VERSION-}" ] && error "--version is not set"

if [ "$CHANNEL" != "release" ] && [ "$CHANNEL" != "nightly" ]; then
	error "--channel must be either 'release' or 'nightly'"
fi

if [ "$DO_SIGN" = "true" ]; then
	if [ "$OS" = "macos" ]; then
		[ -z "${APPLE_CERTIFICATE-}" ] && error "$APPLE_CERTIFICATE is not set"
		[ -z "${APPLE_CERTIFICATE_PASSWORD-}" ] && error "$APPLE_CERTIFICATE_PASSWORD is not set"
		[ -z "${APPLE_ID-}" ] && error "$APPLE_ID is not set"
		[ -z "${APPLE_TEAM_ID-}" ] && error "$APPLE_TEAM_ID is not set"
		[ -z "${APPLE_PASSWORD-}" ] && error "$APPLE_PASSWORD is not set"
		export APPLE_CERTIFICATE="$APPLE_CERTIFICATE"
		export APPLE_CERTIFICATE_PASSWORD="$APPLE_CERTIFICATE_PASSWORD"
		export APPLE_ID="$APPLE_ID"
		export APPLE_TEAM_ID="$APPLE_TEAM_ID"
		export APPLE_PASSWORD="$APPLE_PASSWORD"
	elif [ "$OS" == "linux" ]; then
		[ -z "${APPIMAGE_KEY_ID-}" ] && error "$APPIMAGE_KEY_ID is not set"
		[ -z "${APPIMAGE_KEY_PASSPHRASE-}" ] && error "$APPIMAGE_KEY_PASSPHRASE is not set"
		export SIGN=1
		export SIGN_KEY="$APPIMAGE_KEY_ID"
		export APPIMAGETOOL_SIGN_PASSPHRASE="$APPIMAGE_KEY_PASSPHRASE"
	elif [ "$OS" == "windows" ]; then
		: # nothing to do on windows
	else
		error "signing is not supported on $(uname -s)"
	fi
fi

info "building:"
info "	channel: $CHANNEL"
info "	version: $VERSION"
info "	os: $OS"
info "	arch: $ARCH"
info "	dist: $DIST"
info "	sign: $DO_SIGN"
info "	target: ${TARGET:-default}"

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' exit

CONFIG_PATH=$(readlink -f "$PWD/../crates/gitbutler-tauri/tauri.conf.$CHANNEL.json")

if [ "$OS" = "windows" ]; then
	FEATURES="offline"
elif [ "$OS" = "linux" ]; then
	FEATURES="offline"
elif [ "$OS" = "macos" ]; then
	FEATURES="offline"
else
	echo "Unsupported OS: $OS"
	exit 1
fi


# update the version in the tauri release config
jq  --arg version "$VERSION"\
  '.version = $version | del(.bundle.externalBin)' "$CONFIG_PATH" >"$TMP_DIR/tauri.conf.json"

# Useful for understanding exactly what goes into the tauri build/bundle.
cat "$TMP_DIR/tauri.conf.json"

# Set build metadata for the desktop bundle. No CLI or Askpass sidecar is
# built in the offline product.
export VERSION
export CHANNEL

# Build the app with release config
if [ -n "$TARGET" ]; then
	# Export TARGET for cargo to use
	export CARGO_BUILD_TARGET="$TARGET"

	# Build with specified target
	# Note: passing --target is necessary to let tauri find the binaries,
	# it ignores CARGO_BUILD_TARGET and is more of a hack.
	tauri build \
		--verbose \
		--features "$FEATURES" \
		--config "$TMP_DIR/tauri.conf.json" \
		--target "$TARGET"

  BUNDLE_DIR=$(readlink -f "$PWD/../target/$TARGET/release/bundle")
  BUILD_DIR=$(readlink -f "$PWD/../target/$TARGET/release")
else
	# Build with default target
	tauri build \
		--verbose \
		--features "$FEATURES" \
		--config "$TMP_DIR/tauri.conf.json"

	BUNDLE_DIR=$(readlink -f "$PWD/../target/release/bundle")
	BUILD_DIR=$(readlink -f "$PWD/../target/release")
fi

RELEASE_DIR="$DIST/$OS/$ARCH"
mkdir -p "$RELEASE_DIR"

if [ "$OS" = "macos" ]; then
	MACOS_DMG="$(find "$BUNDLE_DIR/dmg" -depth 1 -type f -name "*.dmg")"

	cp "$MACOS_DMG" "$RELEASE_DIR"

	info "built:"
	info "	- $RELEASE_DIR/$(basename "$MACOS_DMG")"
elif [ "$OS" = "linux" ]; then
	APPIMAGE="$(find "$BUNDLE_DIR/appimage" -name \*.AppImage)"
	DEB="$(find "$BUNDLE_DIR/deb" -name \*.deb)"
	RPM="$(find "$BUNDLE_DIR/rpm" -name \*.rpm)"

	cp "$APPIMAGE" "$RELEASE_DIR"
	cp "$DEB" "$RELEASE_DIR"
	cp "$RPM" "$RELEASE_DIR"

	info "built:"
	info "	- $RELEASE_DIR/$(basename "$APPIMAGE")"
	info "	- $RELEASE_DIR/$(basename "$DEB")"
	info "	- $RELEASE_DIR/$(basename "$RPM")"
elif [ "$OS" = "windows" ]; then
	WINDOWS_INSTALLER="$(find "$BUNDLE_DIR/msi" -name \*.msi)"

	cp "$WINDOWS_INSTALLER" "$RELEASE_DIR"

	info "built:"
	info "	- $RELEASE_DIR/$(basename "$WINDOWS_INSTALLER")"
else
	error "unsupported os: $OS"
fi

info "done! bye!"
