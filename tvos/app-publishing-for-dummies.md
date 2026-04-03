# AudioViz tvOS — TestFlight Publishing Guide

Everything you need to get a build on TestFlight. One-time setup is marked **[once]**.

---

## Prerequisites **[once]**

1. **Paid Apple Developer account** ($99/year) — free accounts can only run Simulator builds.
2. Log in to Xcode: **Xcode → Settings → Accounts → +** → sign in with your Apple ID.
3. Log in to App Store Connect: `appstoreconnect.apple.com` → same Apple ID.

---

## App Store Connect setup **[once]**

1. Go to **My Apps → +** → **New App**
   - Platform: **tvOS**
   - Name: `AudioViz`
   - Primary language: English
   - Bundle ID: `com.audioviz.AudioViz` (create it in the Identifiers section first if it doesn't exist)
   - SKU: `audioviz-tvos` (anything unique, internal only)
2. Enable the **MusicKit** capability for your bundle ID:
   - **Certificates, Identifiers & Profiles → Identifiers → com.audioviz.AudioViz**
   - Tick **MusicKit** → Save

---

## Build the Rust static library **[each release]**

```bash
cd tvos
./build-rust.sh          # produces bridge/libaudio_viz.a for aarch64-apple-tvos
```

Requires: Rust nightly + `aarch64-apple-tvos` target.
See the top of `build-rust.sh` for one-time toolchain setup commands.

---

## Generate placeholder icons (if not already done) **[once]**

```bash
cd tvos
./make-placeholder-icons.sh
```

Replace the generated PNGs in `AudioViz/Assets.xcassets/` with real artwork before App Store review. For TestFlight, the placeholders are fine.

---

## Regenerate the Xcode project (if project.yml changed) **[as needed]**

```bash
cd tvos
xcodegen generate
```

---

## Archive and upload **[each release]**

### Option A — Xcode GUI (recommended)

1. Open `tvos/AudioViz.xcodeproj` in Xcode.
2. Set the scheme destination to **Any Apple TV Device (arm64)** (top of Xcode window).
3. **Product → Archive** — waits a few minutes.
4. When the Organizer opens, select the archive → **Distribute App**.
5. Choose **TestFlight & App Store** → **Upload**.
6. Leave all checkboxes at defaults → **Next → Upload**.

### Option B — command line

```bash
cd tvos

# Archive
xcodebuild archive \
  -project AudioViz.xcodeproj \
  -scheme AudioViz \
  -destination 'generic/platform=tvOS' \
  -archivePath build/AudioViz.xcarchive \
  CODE_SIGN_STYLE=Automatic \
  DEVELOPMENT_TEAM=<YOUR_10_CHAR_TEAM_ID>

# Export for TestFlight
xcodebuild -exportArchive \
  -archivePath build/AudioViz.xcarchive \
  -exportPath build/AudioViz-export \
  -exportOptionsPlist ExportOptions.plist
```

`ExportOptions.plist` (create once in `tvos/`):
```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>method</key>
  <string>app-store-connect</string>
  <key>destination</key>
  <string>upload</string>
  <key>teamID</key>
  <string>YOUR_10_CHAR_TEAM_ID</string>
</dict>
</plist>
```

Your team ID is visible at `developer.apple.com/account` → Membership.

---

## Add testers in App Store Connect

1. Go to your app → **TestFlight** tab.
2. The build appears within ~10 minutes (processing notification via email).
3. **Internal Testing**: add up to 100 people by Apple ID — no Apple review needed, available immediately.
4. **External Testing**: add a group, submit for TestFlight review (~1 business day).

Testers install via the **TestFlight app** on their Apple TV.

---

## Bump the build number for each upload

Xcode rejects duplicate build numbers. Increment `CFBundleVersion` in `project.yml` (or `Info.plist`) before each archive:

```yaml
# project.yml — under info.properties:
CFBundleShortVersionString: "1.0"   # user-visible version
CFBundleVersion: "2"                # increment each upload
```

Then re-run `xcodegen generate` so the change lands in the `.xcodeproj`.

---

## Common errors

| Error | Fix |
|---|---|
| `No profiles for 'com.audioviz.AudioViz'` | Sign in to Xcode with your dev account; let Automatic Signing create the profile |
| `MusicKit entitlement missing` | Enable MusicKit in the bundle ID at developer.apple.com |
| `Archive requires a device destination` | Switch destination from Simulator to **Any Apple TV Device** |
| `CFBundleVersion already exists` | Bump the build number (see above) |
| `libaudio_viz.a not found` | Run `./build-rust.sh` first |
