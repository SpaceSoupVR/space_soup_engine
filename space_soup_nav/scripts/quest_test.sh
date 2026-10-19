#!/bin/sh
# Runs this crate's tests ON THE QUEST: cross-compiles the test binaries for aarch64 Android,
# pushes them and tests/data to the headset, and runs each there. The research's rule is to
# profile and verify on the device from phase 1, because a Mac's results (and its floating
# point) are not the headset's.
#
#   scripts/quest_test.sh [--build-only] [-- <test filter / libtest args>]
#
# Needs the Android NDK (ANDROID_NDK_ROOT, or the newest under the SDK, found the way
# quest_app/run.sh finds it), `rustup target add aarch64-linux-android`, and adb with one device.
set -eu
cd "$(dirname "$0")/.."

BUILD_ONLY=0
while [ $# -gt 0 ]; do
    case "$1" in
        --build-only) BUILD_ONLY=1; shift ;;
        --) shift; break ;;
        *) echo "usage: $0 [--build-only] [-- <libtest args>]"; exit 2 ;;
    esac
done

case "$(uname -s)" in
    Darwin) PREBUILT=darwin-x86_64; SDK="${ANDROID_HOME:-$HOME/Library/Android/sdk}" ;;
    *)      PREBUILT=linux-x86_64;  SDK="${ANDROID_HOME:-$HOME/Android/Sdk}" ;;
esac
NDK="${ANDROID_NDK_ROOT:-${ANDROID_NDK_HOME:-}}"
if [ -z "$NDK" ] || [ ! -d "$NDK" ]; then
    # Newest NDK with a toolchain for this host. No `sort -V`: macOS does not have it.
    newest=""
    for dir in "$SDK"/ndk/*; do
        if [ -d "$dir/toolchains/llvm/prebuilt/$PREBUILT/bin" ]; then
            newest="$newest$(basename "$dir")
"
        fi
    done
    if [ -n "$newest" ]; then
        NDK="$SDK/ndk/$(printf '%s' "$newest" | sort -t. -k1,1n -k2,2n -k3,3n | tail -1)"
    fi
fi
if [ -z "$NDK" ] || [ ! -d "$NDK" ]; then
    echo "Android NDK not found: set ANDROID_NDK_ROOT or install one under $SDK/ndk" >&2
    exit 1
fi
API=29 # quest_app's minSdk (android/app/build.gradle)
BIN="$NDK/toolchains/llvm/prebuilt/$PREBUILT/bin"
export CC_aarch64_linux_android="$BIN/aarch64-linux-android$API-clang"
export AR_aarch64_linux_android="$BIN/llvm-ar"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$CC_aarch64_linux_android"

# Release, as the game ships; tests keep their assertions (debug-assertions are off, `assert!` is not).
TESTS=$(cargo test --release --no-run --target aarch64-linux-android --message-format=json 2>/dev/null \
    | python3 -I -c '
import json, sys
for line in sys.stdin:
    try:
        m = json.loads(line)
    except ValueError:
        continue
    if m.get("reason") == "compiler-artifact" and m.get("profile", {}).get("test") and m.get("executable"):
        print(m["executable"])
')
if [ -z "$TESTS" ]; then
    echo "cross-compiling the tests failed; rerun without the JSON filter to see why:" >&2
    echo "  cargo test --release --no-run --target aarch64-linux-android" >&2
    exit 1
fi
echo "built:"; printf '  %s\n' $TESTS
[ "$BUILD_ONLY" = 1 ] && exit 0

if [ "$(adb devices | awk 'NR>1 && $2=="device"' | wc -l | tr -d ' ')" != 1 ]; then
    echo "need exactly one adb device:" >&2; adb devices >&2; exit 1
fi
REMOTE=/data/local/tmp/space_soup_nav
adb shell "rm -rf $REMOTE && mkdir -p $REMOTE/bin"
adb push tests/data "$REMOTE/data" >/dev/null
for t in $TESTS; do adb push "$t" "$REMOTE/bin/" >/dev/null; done

failed=0
for t in $TESTS; do
    name=$(basename "$t")
    echo "== $name"
    # adb shell's exit status is the remote command's, so a failing test fails this script.
    if ! adb shell "cd $REMOTE && chmod 755 bin/$name && SPACE_SOUP_NAV_TEST_DATA=$REMOTE/data ./bin/$name $*"; then
        failed=1
    fi
done
[ "$failed" = 0 ] && echo "all test binaries passed on the headset" || { echo "FAILED on the headset" >&2; exit 1; }
