<!-- SPDX-FileCopyrightText: Copyright (c) 2026 kanri-sync contributors -->
<!-- -->
<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# Kanri Sync: project briefing

Fork of [kanriapp/kanri](https://github.com/kanriapp/kanri) (Tauri v2 + Nuxt 4 / Vue 3 kanban app)
that adds an **Android build** and **encrypted peer-to-peer LAN sync** between the owner's
Windows PC, Galaxy S22/S26 phone and Galaxy Tab. No cloud, no server. User-facing docs: `SYNC.md`.

## Owner / working style
- Not a developer. Give short, numbered, click-by-click steps; no jargon. Windows PC.
- Ship fixes as releases they can install; they test on real devices and send screenshots.

## Branches and releases
- Work on **`feature/android-lan-sync`**. `main` is still upstream Kanri (never merged).
- CI: `.github/workflows/build-sync.yml` builds Windows (.exe/.msi), Linux (.deb/.AppImage) and a
  signed arm64 APK on every push to `main` / `feature/**`.
- **To publish a release:** bump the version in `package.json`, `src-tauri/tauri.conf.json` and
  `src-tauri/Cargo.toml` (+ `cargo check` to update `Cargo.lock`), then commit with a message
  starting **`Release vX.Y.Z`** and push. CI creates the tag + GitHub release itself (pushing tags
  from the cloud environment is blocked). Pushing again cancels an in-flight build on the same branch.
- Android signing uses repo secrets `ANDROID_KEYSTORE_BASE64`, `ANDROID_KEYSTORE_PASSWORD`,
  `ANDROID_KEY_ALIAS` (already set). Same key every build, so updates install in place and keep
  pairing + data. Never commit a keystore.
- App identifier `io.github.misterjice.kanrisync` (installs beside upstream Kanri).

## Code map
- `src-tauri/src/sync/`: Rust sync engine
  - `doc.rs`: replicated model. Boards are flattened to board/column/card entities with LWW
    registers (body, parent, child order) + tombstones; merge is commutative/idempotent.
    `lastEdited` is device-local. Backgrounds sync in a portable form (see `blobs.rs`).
  - `blobs.rs`: background images are copied into `sync/blobs/<sha256>.<ext>` and synced by
    hash; missing images are fetched from peers via `/blob`. `null` background == no key, so a
    device's "none" never overwrites another's picture. UI never deletes a background whose file
    is missing (`useBackgroundImage` `mutateBoardOnMissingFile=false`).
  - `crypto.rs`: XChaCha20-Poly1305; group key shared via one-time QR pairing token.
  - `net.rs`: tiny_http server on port 47613 + ureq client; pairing code format `kanrisync1:...`.
  - `discovery.rs`: mDNS `_kanrisync._tcp` with hashed group tag.
  - `mod.rs`: state, scheduler (push ~1.5s after edits, poll 60s), Tauri commands `sync_*`,
    one-time import of upstream Kanri data on desktop, e2e tests (multiple app instances).
- Frontend: `stores/sync.ts`, `components/SyncButton.vue`, `components/SyncBanner.vue`,
  `pages/sync.vue`, `utils/device.ts`; `stores/boards.ts` calls `sync_local_change` after saves.
- Android: project is generated in CI (`tauri android init`), then patched by
  `scripts/android-patch.mjs` (Wi-Fi multicast lock for mDNS + permissions).
- Capabilities: `src-tauri/capabilities/main.json` (desktop), `mobile.json` (Android).

## Conventions / pitfalls (learned the hard way)
- Phone layout uses Tailwind `max-sm:` variants (<640px). Tablet/PC keep the desktop layout.
- **Desktop-only plugins (autostart, window-state, single-instance) are not registered on
  Android.** Guard any JS call to them; `layouts/default.vue` runs startup steps independently so
  one failure can't hide the nav bar or stop sync.
- Don't put `//` comments inside multi-line `:class="[...]"` template expressions.
- `@tauri-apps/plugin-barcode-scanner` (JS) must match the Rust crate's major.minor, or
  `tauri build` fails.
- Touch: drag starts after a 500ms hold (`touchDragDelay`); `v-dragscroll` is disabled on touch.
- smooth-dnd puts `touch-action: none` on `<body>` at touchstart and only clears it on mouseup;
  `installTouchFixes()` (utils/device.ts) clears it on touchend, or touch scrolling dies.
- Keyboard: Android 15+ edge-to-edge doesn't resize for the IME; MainActivity (android-patch.mjs)
  pads content by the IME inset. Don't push docs-only commits while a release build runs (it
  cancels it).

## Verify before pushing
```sh
pnpm install --frozen-lockfile
mkdir -p .output/public && (cd src-tauri && cargo test --lib sync && cargo clippy --all-targets)
pnpm generate            # frontend build
```
Linux build deps for cargo: `libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libayatana-appindicator3-dev`.
For UI checks without a device: serve `.output/public` and screenshot with Playwright at 412x915
(phone) / 915x412 (landscape), mocking `window.__TAURI_INTERNALS__.invoke` (make
`plugin:autostart|*` throw, like real Android). Pre-existing upstream lint/type errors exist in
`pages/kanban/[id].vue` and `components/kanban/Column.vue`; don't treat them as regressions.

## Open items / ideas
- Import/Export (file dialogs) on Android is untested; advise doing it on the PC.
- On-screen keyboard fix (v0.9.4) needs confirmation on real devices.
- If a sync arrives while the card editor is open, edits after the reload can be lost (rare).
- Optional: prefer 192.168.x addresses and skip virtual adapters (WSL/Hyper-V/Docker/VPN) when
  advertising sync addresses.
- Phone VPN without "allow LAN access" can block sync.
