#!/usr/bin/env bash
# Package the lares-hud-client cdylib into an installable APK.
#
# No Gradle: aapt2 links the manifest, the .so drops into lib/armeabi-v7a,
# zipalign + apksigner seal it with the debug key. Deterministic by design.
set -euo pipefail

SDK="${ANDROID_HOME:-$HOME/Android/Sdk}"
export JAVA_HOME="${JAVA_HOME:-$HOME/brief-tools/jdk-17.0.20+8}"
export PATH="$JAVA_HOME/bin:$PATH"
BT="$(ls -d "$SDK"/build-tools/* | sort -V | tail -1)"
PLATFORM="$SDK/platforms/android-35/android.jar"
CRATE_DIR="$(cd "$(dirname "$0")" && pwd)"
OUT="$CRATE_DIR/target/lares-hud.apk"
SO="$CRATE_DIR/../target/armv7-linux-androideabi/release/liblares_hud_client.so"
KEYSTORE="$HOME/.android/debug.keystore"

[ -f "$SO" ] || { echo "missing $SO — run cargo apk build first" >&2; exit 1; }

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# 1. Link the manifest into a base APK.
"$BT/aapt2" link \
    -I "$PLATFORM" \
    --manifest "$CRATE_DIR/AndroidManifest.xml" \
    --min-sdk-version 21 \
    --target-sdk-version 28 \
    -o "$WORK/base.apk"

# 2. Add the native library for armeabi-v7a (stored, aligned later).
mkdir -p "$WORK/lib/armeabi-v7a"
cp "$SO" "$WORK/lib/armeabi-v7a/"
python3 - "$WORK/base.apk" "$WORK/lib/armeabi-v7a/liblares_hud_client.so" <<'PYEOF'
import sys, zipfile
apk, so = sys.argv[1], sys.argv[2]
with zipfile.ZipFile(apk, 'a') as z:
    z.write(so, 'lib/armeabi-v7a/liblares_hud_client.so', compress_type=zipfile.ZIP_STORED)
PYEOF

# 3. Align and sign with the debug key.
[ -f "$KEYSTORE" ] || keytool -genkeypair -keystore "$KEYSTORE" \
    -alias androiddebugkey -storepass android -keypass android \
    -dname "CN=Android Debug,O=Android,C=US" \
    -keyalg RSA -keysize 2048 -validity 10000 2>/dev/null

mkdir -p "$(dirname "$OUT")"
"$BT/zipalign" -f 4 "$WORK/base.apk" "$OUT"
"$BT/apksigner" sign --ks "$KEYSTORE" --ks-pass pass:android --out "$WORK/signed.apk" "$OUT"
mv "$WORK/signed.apk" "$OUT"

echo "built: $OUT ($(stat -c %s "$OUT") bytes)"
