# Lares Android client

Thin fast-loop client: CameraX preview, keyframe capture, protojson POST to the
Rust server, Compose overlay of normalized chore boxes, and tap-to-done chore
lifecycle.

## Prerequisites (one-time)

1. Install missing SDK packages:
   ```bash
   export JAVA_HOME=/home/randozart/brief-tools/jdk-17.0.20+8
   SDKMANAGER=/home/randozart/Android/Sdk/cmdline-tools/latest/bin/sdkmanager
   yes | $SDKMANAGER --licenses
   $SDKMANAGER "platforms;android-35" "build-tools;35.0.0"
   ```
2. Point Gradle at the SDK (either export or create `android/local.properties`):
   ```bash
   echo "sdk.dir=/home/randozart/Android/Sdk" > android/local.properties
   ```
3. Enable USB debugging on the phone and accept the prompt for device `a90682c5`.

## Build & install

```bash
./gradlew -p android assembleDebug
adb install -r android/app/build/outputs/apk/debug/app-debug.apk
```

## Dev loop with adb reverse (recommended)

The app defaults to `http://localhost:8787`. Forward the phone's localhost to
your workstation so no Wi-Fi config is needed:

```bash
adb reverse tcp:8787 tcp:8787
```

Then run the server (`make run-server-mock` or `LARES_ENGINE=gemini`).

## How it maps to the architecture

- Fast loop: `MainActivity` + `CameraController` + `ChoreOverlay`.
- Slow loop: the Rust server at `/v1/analyze`; the phone only renders boxes.
- Contract: `.proto` files compile to Java via the Gradle protobuf plugin and
  are serialized with protobuf-java-util `JsonFormat` (protojson).
- Phase G: notification channel + WorkManager are declared here; a future
  policy surfaces chores when the phone is idle.

## MVP caveats (documented in PLAN.md)

- Boxes overlay the frozen keyframe, not live-tracked video.
- `PreviewView` uses FILL_CENTER; if the captured aspect differs, box alignment
  is approximate.
- Cleartext HTTP is enabled for LAN development only.