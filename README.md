<!--
SPDX-FileCopyrightText: Copyright (c) 2022-2026 trobonox <hello@trobo.dev>
SPDX-FileCopyrightText: Copyright (c) 2026 kanri-sync contributors

SPDX-License-Identifier: Apache-2.0
-->

# Kanri Sync

**Kanri Sync** is a fork of [Kanri](https://github.com/kanriapp/kanri), the offline Kanban board
app, that adds two things the original does not have:

1. **An Android version** (phones and tablets), with a layout made for touch screens.
2. **Automatic, encrypted sync between your own devices over your home Wi-Fi.** No cloud, no
   account, no server: your PC, phone and tablet talk to each other directly.

Everything else (boards, cards, themes, backgrounds, import/export) is Kanri as you know it.

- **Download:** [Releases](https://github.com/misterjice/kanri-sync/releases) (Windows, Linux, Android)
- **Install, pair devices, troubleshooting:** [SYNC.md](SYNC.md)
- **What changed in each version:** [CHANGELOG.md](CHANGELOG.md)
- **Bugs / ideas for this fork:** [Issues](https://github.com/misterjice/kanri-sync/issues). Please
  do **not** report Kanri Sync problems to the upstream Kanri project.

> [!NOTE]
> Development happens on the `feature/android-lan-sync` branch. `main` tracks upstream Kanri.

## What this fork adds

| Area | What was done |
|---|---|
| Android app | Same app built for Android (arm64 phones and tablets). Installs from a signed APK; updates install over the old version and keep your boards and pairing. |
| Phone layout | Bottom navigation bar, full-width columns, press-and-hold (0.5 s) to drag so swiping scrolls, a drag-free strip for sideways scrolling, dialogs and settings that fit the screen, keyboard and system bars never cover fields. Tablets and PCs keep the desktop layout. |
| LAN sync | Changes are sent ~1.5 s after you make them, every device checks in once a minute, and the phone syncs as soon as you open the app. Devices find each other automatically (mDNS) on the same network. |
| Smart merging | Boards, columns and cards are synced one by one, not as one file: cards added on two devices while apart are both kept; a rename on one device and a move on another both survive; deletions stick. |
| Security | One-time QR-code pairing shares a group key; all traffic is encrypted with XChaCha20-Poly1305. Unpaired devices can neither read nor change your boards. |
| Background pictures | Board background images are copied between devices (by content hash), so a picture set on the PC shows up on the phone and tablet. |
| Clear status | Sync button with a coloured status dot, a red banner that stays until an error is fixed, and a Sync page with per-device status and an activity log. |
| Side-by-side install | Own app identity (`io.github.misterjice.kanrisync`), so it installs next to the original Kanri. On first launch on a PC it copies your existing Kanri boards over. |
| Automated builds | GitHub Actions builds Windows (`.exe`/`.msi`), Linux (`.deb`/`.AppImage`) and a signed Android APK, runs the sync test suite, and publishes releases. |

### How it fits together

```
 PC (Windows) <--- encrypted HTTP, port 47613 ---> Phone (Android)
      ^                                                  ^
      |            same Wi-Fi, found via mDNS            |
      +-------------------> Tablet (Android) <-----------+
```

There is no central copy: each device holds the full set of boards and merges what the others send.
An always-on PC is a convenient meeting point, because Android pauses apps in the background.

### Code added by the fork

- `src-tauri/src/sync/`: Rust sync engine (`doc.rs` merge model, `blobs.rs` background images,
  `crypto.rs`, `net.rs` transport, `discovery.rs` mDNS, `mod.rs` scheduler and app commands),
  with unit tests and end-to-end tests that pair and sync several app instances over real sockets.
- `stores/sync.ts`, `components/SyncButton.vue`, `components/SyncBanner.vue`, `pages/sync.vue`,
  `utils/device.ts`: sync UI and touch/phone fixes.
- `scripts/android-patch.mjs`: patches the generated Android project (Wi-Fi multicast for
  discovery, permissions, keyboard handling).
- `.github/workflows/build-sync.yml`: builds and releases.

---

*The rest of this page is the original Kanri README, kept for reference. Download links and
badges below point to upstream Kanri, not to Kanri Sync.*

## Demo
![showcase_gif_kanri](https://github.com/user-attachments/assets/14d26751-cb5e-4164-a2f9-84e2b7dc200c)


## Download
Kanri is available for Windows, macOS and Linux (only supporting more recent versions of the respective operating systems).

There are several ways to download Kanri:
- **Recommended**: Select the installer for your operating system on the [official download page](https://kanriapp.com/download)
- The same downloads can also be found under the [GitHub releases](https://github.com/kanriapp/kanri/releases/)
- For macOS, you can also use Homebrew:
  ```bash
  brew install kanriapp/cask/kanri
  ```
  
<details>
    <summary>Note for Apple Silicon macOS users:</summary>Because Kanri is not signed (there is no funding which would sponsor the required Apple Developer membership), you need to run the following command to prevent errors saying the app is broken:
    </summary>

    xattr -cr /Applications/kanri.app
</details>


## Why Kanri?
At it's core, Kanri has the philosophy "do one thing, and do it well". Kanri is supposed to be a simpler, more user-friendly *offline* alternative to other cloud-based Kanban applications.

No matter what features get added, your data will always be yours, and there will never be any cloud sync built in. In the future, there will most likely be an option to save individual boards to different file paths, which will make the usage of tools such as Syncthing easier, but it's up to you what you do with your data.

### Core Features
- Familiar Kanban board layout with customizable columns, cards, including rich-text descriptions, sub-tasks, due-dates and tags
- Customization options such as custom themes and board background images
- Offline data storage in one simple `.json` file
- Keyboard shortcuts for faster board navigation
- Basic data import from Trello, granular or full export for backups

## Roadmap
Long term vision for the project:
- 👷‍♂️ Improve current features and refactor to avoid tech debt
- ➕ Add additional small/mid-sized features with high impact (reminders, card images, etc.)
- 🚚 Work towards 1.0 release with features from the backlog like internationalization or a widget panel
- 🔍 After 1.0: Maintenance mode, smaller releases featuring fixes, new languages and simple features/UX improvements

A granular list of open tasks can be found [in the roadmap](https://github.com/orgs/kanriapp/projects/2).

This project is open for any contributions or feature requests as long as they are polite, provide enough context and remain patient (replies might take a few days). If you want to work on a feature which is in the backlog, please ask in the corresponding issue (or open one if there is none) about the status of the feature before implementing it.

> [!NOTE]
> This project is still in active development and is provided "AS IS". Please make regular backups/exports to prevent any data loss.

## 🛠 Contributing & Build Setup
*(Upstream Kanri. For Kanri Sync, see "For developers" in [SYNC.md](SYNC.md); Android builds run in CI.)*

If you want to contribute, please take a look at the [Contribution Guidelines](https://github.com/trobonox/kanri/blob/main/CONTRIBUTING.md).
The `main` branch is equivalent to a `dev` branch where development is done on - submit PRs here. The `release` branch is similar to a `stable` branch with the code of latest release.

**Build Setup**:
If you want to build the app, you need to install Node.js (latest LTS version recommended), a package manager like yarn and the [&nearr;&nbsp;Tauri development environment](https://tauri.app/start/prerequisites/).
Then, depending on your use case you can run the commands below:

```bash
# Install dependencies
pnpm install

# Start debug tauri build
pnpm tauri dev

# Build tauri for production
pnpm generate
pnpm tauri build
```

---
**Copyright (c) 2022-2026 trobonox (trobo@kanriapp.com)**. Licensed under GPL v3 (with some files under Apache 2.0 or other licenses stated in the files themselves).
The Kanri logo, name and other branding are **NOT** open source, full copyright belongs to trobonox.

Kanri Sync additions: Copyright (c) 2026 kanri-sync contributors, under the same licenses. Kanri Sync is
an independent fork and is not affiliated with or endorsed by the Kanri project.
