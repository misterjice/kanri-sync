<!-- SPDX-FileCopyrightText: Copyright (c) 2026 kanri-sync contributors -->
<!-- -->
<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# Kanri Sync

A fork of [Kanri](https://github.com/kanriapp/kanri) that also runs on Android and keeps boards
in sync between your PC, phone and tablet **over your own Wi-Fi**. There is no cloud, no account
and no server: the devices talk to each other directly.

## Installing

Every push builds installers with GitHub Actions (**Actions → Build Kanri Sync → latest run →
Artifacts**). Pushing a tag such as `v0.9.0` also publishes them as a GitHub release.

| Device | File |
|---|---|
| Windows PC | `kanri-sync-windows` → `Kanri Sync_x.y.z_x64-setup.exe` |
| Linux | `kanri-sync-linux` → `.deb` / `.AppImage` |
| Android phone / tablet (arm64) | `kanri-sync-android` → `KanriSync-android-arm64.apk` |

On Android, open the APK and allow “install unknown apps” for your browser/file manager when asked.

If you already use the original Kanri on the same PC, your boards are copied over automatically on
first launch. On Android you can also bring boards over with **Import/Export**.

### Stable Android signing key (do this once)

Android only installs an update if it is signed with the same key as the installed version. Without
the secrets below, every build gets a throwaway key and you must uninstall before installing a new
build (your boards come back from the PC via sync, but it is annoying). To set a permanent key:

```sh
keytool -genkeypair -v -keystore kanrisync.jks -alias kanrisync -keyalg RSA -keysize 2048 -validity 10000
# Linux/macOS:
base64 -w0 kanrisync.jks > kanrisync.jks.b64
# Windows PowerShell:
[Convert]::ToBase64String([IO.File]::ReadAllBytes("kanrisync.jks")) | Out-File kanrisync.jks.b64
```

Then in GitHub: **Settings → Secrets and variables → Actions → New repository secret**:

- `ANDROID_KEYSTORE_BASE64`: contents of `kanrisync.jks.b64`
- `ANDROID_KEYSTORE_PASSWORD`: the password you chose
- `ANDROID_KEY_ALIAS`: `kanrisync`

Keep `kanrisync.jks` somewhere safe and **never commit it**.

## Pairing (once per device)

1. Put both devices on the same Wi-Fi and open **Sync** (the ⟳ button, long-press or right-click
   it, or the button shown when not yet paired).
2. On the PC, press **Show pairing code**.
3. On the phone, press **Scan pairing code** and point the camera at the QR code.
   (No camera? Use **Enter code manually** and paste the text code.)
4. Repeat for the tablet. You can scan the PC's code again, or the phone's code: any paired device
   can add a new one, and all devices learn about each other automatically.

The first time the PC app starts, Windows may ask whether Kanri Sync may use the network. Allow it
for **private networks**, otherwise the phone cannot reach the PC.

## Day to day

- **Automatic:** changes are sent about 1.5 seconds after you make them, every device checks in once a
  minute, and a phone syncs as soon as you open the app.
- **Sync button (⟳):** tap it to sync right now. The dot shows the state:

  | Dot | Meaning |
  |---|---|
  | green | in sync |
  | blue (spinning) | syncing |
  | amber | no other device reachable right now (e.g. PC off). Changes are kept and sent later |
  | red | **an error.** A red banner explains it, with Retry / Details |
  | grey | not paired or sync turned off |

- **Errors are never silent:** a red banner stays on screen until the problem is fixed or you
  dismiss it, and the Sync page lists the last error per device plus an activity log.

### How edits are merged

Boards, columns and cards are synced individually, not as one big file, so:

- adding cards on the phone and on the PC while apart keeps **both**;
- renaming a card on one device and moving it on another keeps **both** changes;
- if the same field is changed on two devices, the most recent change wins;
- deleted items stay deleted; an item edited *after* it was deleted on another device comes back.

## What is and isn't synced

| Synced | Not synced (per device) |
|---|---|
| Boards, columns, cards, descriptions, checklists, tags, due dates, pins | Theme, language, zoom and other settings |
| Board background pictures (set them on the PC; the image is copied to the other devices) | |

## Limits worth knowing

- Devices must be on the same network at the same time. Android does not let apps stay active in
  the background, so the phone and tablet sync while Kanri Sync is open (or briefly after). An
  always-on PC acts as the natural meeting point.
- Guest Wi-Fi networks and some mesh/router setups block devices from talking to each other
  (“AP/client isolation”). Use your main network.
- Data between devices is encrypted (XChaCha20-Poly1305) with a key exchanged by the QR code;
  devices that were never paired cannot read or change your boards.
- **Remove** on the Sync page only hides a device from the list. To take a device out of the group
  completely, use **Stop syncing this device** *on that device*. If a device is lost, leave the
  group on all remaining devices and pair them again; that creates a new key.

## Troubleshooting

| Symptom | Fix |
|---|---|
| Amber dot, “not reachable” | Same Wi-Fi? Kanri Sync open on the other device? On Windows, allow the app through the firewall (private networks). |
| “belongs to a different sync group” | One device was re-paired or left the group. Pair it again. |
| “Pairing is not open on that device” | The code expired (10 min) or the dialog was closed. Show a new code. |
| Discovery unavailable notice | Some networks block mDNS. Sync still works with the addresses learned while pairing; if a device's address changes, pair again. |

## For developers

- Rust sync engine: `src-tauri/src/sync/` (`doc.rs` merge model, `crypto.rs`, `net.rs` transport,
  `discovery.rs` mDNS, `mod.rs` orchestration). Tests: `cd src-tauri && cargo test --lib sync`
  (includes end-to-end tests that pair and sync several app instances over real sockets).
- Frontend: `stores/sync.ts`, `components/SyncButton.vue`, `components/SyncBanner.vue`, `pages/sync.vue`.
- Android project is generated in CI (`tauri android init`) and patched by `scripts/android-patch.mjs`
  (multicast lock for mDNS, Wi-Fi permissions).
