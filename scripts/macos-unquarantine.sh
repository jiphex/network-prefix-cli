#!/bin/sh
#
# Undo what macOS does to a downloaded binary.
#
# A browser tags anything it saves with com.apple.quarantine, and Gatekeeper
# then refuses to run it, because these binaries are ad-hoc signed rather than
# notarized with an Apple Developer ID. This clears that flag, and repairs the
# ad-hoc signature if it has been damaged along the way.
#
# Unpacking the tarball from Terminal avoids all of this in the first place;
# the flag is only ever set on files a browser wrote.
#
# With no arguments it does every binary the archive ships. Name one to do
# only that one.
#
# Usage: ./macos-unquarantine.sh [path-to-binary ...]

set -eu

case $(uname -s) in
Darwin) ;;
*)
	echo "error: this is only needed on macOS (this is $(uname -s))" >&2
	exit 1
	;;
esac

dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)

unquarantine() {
	bin=$1
	echo "==> $bin"

	if xattr -p com.apple.quarantine "$bin" >/dev/null 2>&1; then
		xattr -d com.apple.quarantine "$bin"
		echo "    cleared the quarantine flag"
	else
		echo "    no quarantine flag set"
	fi

	chmod +x "$bin"

	# arm64 macOS kills a binary whose signature does not match its contents,
	# which looks like an unexplained "killed: 9" rather than a signing error.
	if codesign --verify --strict "$bin" >/dev/null 2>&1; then
		echo "    signature is intact"
	else
		codesign --force --sign - --timestamp=none "$bin"
		echo "    repaired the ad-hoc signature"
	fi

	# Proves the thing actually runs, rather than just claiming it will.
	echo "    $("$bin" --version)"
}

if [ "$#" -gt 0 ]; then
	for bin in "$@"; do
		if [ ! -e "$bin" ]; then
			echo "error: nothing at $bin" >&2
			exit 1
		fi
		unquarantine "$bin"
	done
else
	found=
	for name in prefixtool fabrictool; do
		if [ -e "$dir/$name" ]; then
			unquarantine "$dir/$name"
			found="$found $dir/$name"
		fi
	done
	if [ -z "$found" ]; then
		echo "error: no binaries found next to $0" >&2
		echo "usage: $0 [path-to-binary ...]" >&2
		exit 1
	fi
fi

echo
echo "Ready. To put them on your PATH:"
echo "    sudo mv \"$dir\"/prefixtool \"$dir\"/fabrictool /usr/local/bin/"
