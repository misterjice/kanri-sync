<!-- SPDX-FileCopyrightText: Copyright (c) 2026 kanri-sync contributors -->
<!-- -->
<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# Kanri Sync changelog

Changes made in this fork on top of [Kanri](https://github.com/kanriapp/kanri) 0.8.x.
Installers for each version are on the [Releases](https://github.com/misterjice/kanri-sync/releases)
page. How to install and pair devices: [SYNC.md](SYNC.md).

## v0.9.8 (2026-10-09)
Full check of every feature on PC, phone and tablet. Fixed:
- **Duplicate card** did nothing (also affected the original Kanri).
- **Duplicate board**: the copy shared its columns and cards with the original, so after a sync
  the two boards could take each other's cards. Copies now get their own ids, and the sync engine
  repairs any duplicated ids it finds (e.g. from older copies or importing the same board twice).
- **New board with example columns**: boards made this way shared one card list, so a card added
  to one showed up in the others.
- **Import** (Kanri board, Trello, GitHub Project): imported boards did not appear until a restart
  and the next edit wiped them. They now appear at once and are synced.
- **Move to**: could move the wrong card if the card had just been changed on another device.
- "Delete all data" now warns that it also deletes the boards on synced devices.

## v0.9.7 (2026-10-09)
- Pop-up windows (e.g. **Edit tags**) no longer slide under the phone's status bar or navigation
  bar; their last rows can be scrolled into view.
- Includes v0.9.6, which was not published separately.

## v0.9.6 (not released on its own; shipped in v0.9.7)
- Synced background pictures now also show **inside** boards on Android, not only on the home
  screen preview.

## v0.9.5 (2026-10-09)
- **Board background pictures sync.** Images are copied into the app's own storage and sent to
  other devices (encrypted); a picture set on the PC appears on the phone and tablet.
- A device without a background can no longer erase another device's picture.
- A board no longer drops its background just because the image is still being transferred.

## v0.9.4 (2026-10-09)
- Fixed touch scrolling that sometimes stopped working after touching a card or column.
- The on-screen keyboard no longer hides the field you are typing in (Android 15+).
- Includes v0.9.3, whose build was cancelled.

## v0.9.3 (2026-10-09)
- Card editor, settings and every pop-up fit phone screens in portrait and landscape, and scroll
  when taller than the screen.
- Help shows the Kanri Sync version and the Kanri version it is based on.
- Fixed a bug where a missing background image on one device removed the background everywhere.

## v0.9.2 (2026-10-09)
- Easier board scrolling on phones: an empty strip above the bottom menu always scrolls sideways,
  mouse-style drag scrolling is off on touch screens, and press-and-hold to pick up a card is
  0.5 s so a short pause before swiping no longer grabs it.

## v0.9.1 (2026-10-09)
- Fixed Android startup: the navigation bar (with the sync button) was missing and sync never
  started, because a desktop-only feature failed on Android.
- **Sync & pair devices** added to the Settings page as a second way to reach sync.

## v0.9.0 (2026-10-09): first Kanri Sync release
- **Android app** (arm64 phones and tablets) with a touch layout: bottom navigation bar,
  full-width columns, hold-to-drag so swiping scrolls, safe-area handling.
- **Encrypted peer-to-peer sync over the local network**, no cloud or server:
  - QR-code pairing (or a text code); any paired device can add a new one.
  - Per-item merging: concurrent card adds, an edit on one device plus a move on another, and
    deletions all merge correctly.
  - Automatic: push ~1.5 s after edits, check-in every minute, sync on app open; mDNS discovery.
  - XChaCha20-Poly1305 encryption with a group key from pairing.
- Sync button with status dot, persistent error banner (Retry / Details), Sync page with device
  list and activity log.
- Installs next to the original Kanri; existing Kanri boards on the PC are copied in on first launch.
- GitHub Actions builds Windows, Linux and a signed Android APK, runs the sync tests and
  publishes releases.
