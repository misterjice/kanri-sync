<!-- SPDX-FileCopyrightText: Copyright (c) 2026 kanri-sync contributors -->
<!-- -->
<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

<template>
  <main id="sync" class="max-w-3xl overflow-auto px-8 pb-24 pt-6 max-sm:px-4">
    <h1 class="text-4xl font-bold">Sync</h1>
    <p class="text-dim-2 mt-1">
      Keeps your boards identical on your computer, phone and tablet over your home Wi-Fi.
      Nothing is sent to the internet.
    </p>

    <!-- Status -->
    <section class="bg-elevation-1 mt-6 rounded-lg p-4">
      <div class="flex flex-row flex-wrap items-center justify-between gap-3">
        <div class="flex flex-row items-center gap-3">
          <span class="size-3 rounded-full" :class="dot(sync.state)" />
          <div>
            <p class="text-lg font-semibold">{{ stateText }}</p>
            <p class="text-dim-2 text-sm">
              Last successful sync: {{ sync.status?.lastSync ? ago(sync.status.lastSync) : "never" }}
            </p>
          </div>
        </div>
        <button
          class="bg-accent text-buttons flex flex-row items-center gap-2 rounded-md px-4 py-2 font-semibold disabled:opacity-60"
          :disabled="sync.manualRunning || !sync.status?.paired"
          @click="sync.syncNow(true)"
        >
          <PhArrowsClockwise class="size-5" :class="{ 'animate-spin': sync.manualRunning }" />
          Sync now
        </button>
      </div>

      <div
        v-if="sync.manualResult && !sync.manualRunning"
        class="mt-3 rounded-md px-3 py-2 text-sm"
        :class="sync.manualResult.ok ? 'bg-green-700/30' : 'bg-red-600/30'"
      >
        {{ sync.manualResult.message }}
      </div>
      <div v-if="sync.status?.lastError" class="mt-3 rounded-md bg-red-600/30 px-3 py-2 text-sm">
        <p class="font-semibold">Last error</p>
        <p class="whitespace-pre-line">{{ sync.status.lastError }}</p>
        <button class="mt-2 underline" @click="sync.clearError()">Dismiss</button>
      </div>
      <div v-if="sync.status?.discoveryError" class="mt-3 rounded-md bg-amber-600/30 px-3 py-2 text-sm">
        Automatic discovery is unavailable on this device ({{ sync.status.discoveryError }}).
        Devices will still sync using their last known addresses.
      </div>
    </section>

    <!-- Devices -->
    <section class="mt-8">
      <h2 class="mb-2 text-2xl font-bold">Paired devices</h2>
      <p v-if="!sync.status?.peers.length" class="text-dim-2">
        No devices paired yet. Use “Pair a new device” below.
      </p>
      <ul class="flex flex-col gap-2">
        <li
          v-for="p in sync.status?.peers ?? []"
          :key="p.id"
          class="bg-elevation-1 flex flex-row items-start justify-between gap-3 rounded-lg p-3"
        >
          <div class="flex flex-row items-start gap-3">
            <span class="mt-2 size-3 shrink-0 rounded-full" :class="peerDot(p.state)" />
            <div>
              <p class="font-semibold">{{ p.name }}</p>
              <p class="text-dim-2 text-sm">
                {{ peerText(p) }}
              </p>
              <p v-if="p.lastError" class="text-sm text-red-400">{{ p.lastError }}</p>
              <p class="text-dim-3 text-xs">{{ p.addrs.join(", ") || "address unknown" }}</p>
            </div>
          </div>
          <button class="text-dim-2 shrink-0 text-sm underline" @click="confirmForget(p)">
            {{ pendingForget === p.id ? "Tap again to remove" : "Remove" }}
          </button>
        </li>
      </ul>
    </section>

    <!-- Pairing -->
    <section class="mt-8">
      <h2 class="mb-2 text-2xl font-bold">Pair a new device</h2>
      <p class="text-dim-2 mb-3 text-sm">
        Pairing is needed once per device. Open this page on both devices (same Wi-Fi), show the
        code on one and scan it with the other. After that, syncing is automatic.
      </p>
      <div class="flex flex-row flex-wrap gap-3">
        <button class="bg-elevation-1 bg-elevation-2-hover rounded-md px-4 py-2 font-semibold" @click="showCode">
          Show pairing code
        </button>
        <button
          v-if="isMobile"
          class="bg-elevation-1 bg-elevation-2-hover rounded-md px-4 py-2 font-semibold"
          :disabled="joining"
          @click="scanCode"
        >
          Scan pairing code
        </button>
        <button
          class="bg-elevation-1 bg-elevation-2-hover rounded-md px-4 py-2 font-semibold"
          @click="pasteOpen = !pasteOpen"
        >
          Enter code manually
        </button>
      </div>

      <div v-if="pairing" class="bg-elevation-1 mt-4 flex flex-col items-center gap-3 rounded-lg p-4">
        <p class="font-semibold">Scan this with the other device</p>
        <!-- eslint-disable-next-line vue/no-v-html -- SVG generated locally by the qrcode crate -->
        <div class="rounded-md bg-white p-2" v-html="pairing.qrSvg" />
        <p class="text-dim-2 text-sm">Expires in {{ expiresIn }}. Can be used for several devices.</p>
        <details class="w-full text-sm">
          <summary class="cursor-pointer">Can't scan? Copy the code as text</summary>
          <textarea readonly class="bg-elevation-2 mt-2 h-24 w-full rounded-md p-2 font-mono text-xs" :value="pairing.code" />
          <button class="bg-elevation-2 mt-1 rounded-md px-3 py-1" @click="copyCode">Copy</button>
        </details>
        <p v-if="sync.lastPairedWith" class="text-green-400">Paired with {{ sync.lastPairedWith }} ✓</p>
        <button class="underline" @click="hideCode">Done</button>
      </div>

      <div v-if="pasteOpen" class="bg-elevation-1 mt-4 flex flex-col gap-2 rounded-lg p-4">
        <label class="font-semibold" for="pair-code">Pairing code from the other device</label>
        <textarea
          id="pair-code"
          v-model="pasted"
          class="bg-elevation-2 h-24 w-full rounded-md p-2 font-mono text-xs"
          placeholder="kanrisync1:…"
        />
        <button
          class="bg-accent text-buttons self-start rounded-md px-4 py-2 font-semibold disabled:opacity-60"
          :disabled="joining || !pasted.trim()"
          @click="join(pasted)"
        >
          {{ joining ? "Pairing…" : "Pair" }}
        </button>
      </div>
      <p v-if="joinMessage" class="mt-3 rounded-md px-3 py-2 text-sm" :class="joinOk ? 'bg-green-700/30' : 'bg-red-600/30'">
        {{ joinMessage }}
      </p>
    </section>

    <!-- This device -->
    <section class="mt-8">
      <h2 class="mb-2 text-2xl font-bold">This device</h2>
      <div class="bg-elevation-1 flex flex-col gap-3 rounded-lg p-4">
        <label class="flex flex-col gap-1">
          <span class="text-dim-2 text-sm">Name shown on your other devices</span>
          <div class="flex flex-row gap-2">
            <input v-model="name" class="bg-elevation-2 flex-1 rounded-md px-3 py-2" @keyup.enter="saveName">
            <button class="bg-elevation-2 rounded-md px-3" @click="saveName">Save</button>
          </div>
        </label>
        <label class="flex flex-row items-center gap-3">
          <input
            type="checkbox"
            :checked="sync.status?.enabled"
            class="size-5"
            @change="sync.setEnabled(($event.target as HTMLInputElement).checked)"
          >
          <span>Sync enabled on this device</span>
        </label>
        <p class="text-dim-3 text-xs">
          Network addresses: {{ sync.status?.addrs.join(", ") || "not connected" }}
        </p>
      </div>
    </section>

    <!-- Activity -->
    <section class="mt-8">
      <h2 class="mb-2 text-2xl font-bold">Activity</h2>
      <ul class="bg-elevation-1 max-h-72 overflow-auto rounded-lg p-3 font-mono text-xs">
        <li v-if="!sync.status?.log.length" class="text-dim-2">Nothing yet.</li>
        <li
          v-for="(e, i) in sync.status?.log ?? []"
          :key="i"
          :class="e.level === 'error' ? 'text-red-400' : e.level === 'warn' ? 'text-amber-400' : ''"
        >
          {{ new Date(e.t).toLocaleTimeString() }} — {{ e.msg }}
        </li>
      </ul>
    </section>

    <section v-if="sync.status?.paired" class="mt-8">
      <button class="text-sm text-red-400 underline" @click="confirmLeave">
        {{
          pendingLeave
            ? "Tap again to confirm — boards stay on this device, but stop syncing"
            : "Stop syncing this device (leave sync group)"
        }}
      </button>
      <p class="text-dim-3 mt-1 text-xs">
        “Remove” only hides a device from this list; if that device is still in your sync group it
        will reappear. To take a device out of the group, use this button on that device.
      </p>
    </section>
  </main>
</template>

<script setup lang="ts">
import { PhArrowsClockwise } from "@phosphor-icons/vue";
import { platform } from "@tauri-apps/plugin-os";
import type { PairingInfo, SyncPeer, SyncState } from "@/stores/sync";

const sync = useSyncStore();
const layout = useLayoutStore();

const isMobile = ["android", "ios"].includes(platform());
const pairing = ref<PairingInfo | null>(null);
const pasteOpen = ref(false);
const pasted = ref("");
const joining = ref(false);
const joinMessage = ref("");
const joinOk = ref(false);
const name = ref("");
const now = useNow({ interval: 1000 });

onMounted(async () => {
  layout.onHomePageLeave();
  await sync.refresh();
  name.value = sync.status?.deviceName ?? "";
});

onBeforeUnmount(() => {
  if (pairing.value) sync.stopPairing();
});

const stateText = computed(() => {
  switch (sync.state) {
    case "idle":
      return "Up to date";
    case "syncing":
      return "Syncing…";
    case "offline":
      return "Other devices not reachable right now";
    case "error":
      return "Sync problem";
    case "disabled":
      return "Sync is turned off";
    default:
      return "Not paired yet";
  }
});

const dot = (s: SyncState) =>
  ({
    idle: "bg-green-500",
    syncing: "bg-sky-500",
    offline: "bg-amber-400",
    error: "bg-red-500",
    disabled: "bg-gray-500",
    unpaired: "bg-gray-500",
  })[s];

const peerDot = (s: SyncPeer["state"]) =>
  ({ ok: "bg-green-500", offline: "bg-amber-400", error: "bg-red-500", unknown: "bg-gray-500" })[s];

const ago = (t: number) => {
  const s = Math.max(0, Math.round((now.value.getTime() - t) / 1000));
  if (s < 60) return `${s}s ago`;
  if (s < 3600) return `${Math.round(s / 60)} min ago`;
  if (s < 86400) return `${Math.round(s / 3600)} h ago`;
  return new Date(t).toLocaleString();
};

const peerText = (p: SyncPeer) => {
  const last = p.lastOk ? `last synced ${ago(p.lastOk)}` : "not synced yet";
  switch (p.state) {
    case "ok":
      return `Online — ${last}`;
    case "offline":
      return `Not reachable — ${last}`;
    case "error":
      return `Error — ${last}`;
    default:
      return last;
  }
};

const expiresIn = computed(() => {
  if (!pairing.value) return "";
  const s = Math.max(0, Math.round((pairing.value.expiresAt - now.value.getTime()) / 1000));
  if (s === 0) return "0s (expired — press Show pairing code again)";
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
});

const showCode = async () => {
  joinMessage.value = "";
  sync.lastPairedWith = null;
  try {
    pairing.value = await sync.startPairing();
  } catch (e) {
    joinOk.value = false;
    joinMessage.value = String(e);
  }
};

const hideCode = async () => {
  pairing.value = null;
  await sync.stopPairing();
};

const copyCode = async () => {
  if (pairing.value) await navigator.clipboard.writeText(pairing.value.code);
};

const join = async (code: string) => {
  joining.value = true;
  joinMessage.value = "";
  try {
    const other = await sync.join(code);
    joinOk.value = true;
    joinMessage.value = `Paired with ${other}. Your boards will now sync automatically.`;
    pasted.value = "";
    pasteOpen.value = false;
  } catch (e) {
    joinOk.value = false;
    joinMessage.value = `Pairing failed: ${e}`;
  } finally {
    joining.value = false;
  }
};

const scanCode = async () => {
  joinMessage.value = "";
  try {
    const scanner = await import("@tauri-apps/plugin-barcode-scanner");
    let perm = await scanner.checkPermissions();
    if (perm !== "granted") perm = await scanner.requestPermissions();
    if (perm !== "granted") {
      joinOk.value = false;
      joinMessage.value = "Camera permission is needed to scan the code. You can also enter the code manually.";
      return;
    }
    const result = await scanner.scan({ windowed: false, formats: [scanner.Format.QRCode] });
    if (result?.content) await join(result.content);
  } catch (e) {
    joinOk.value = false;
    joinMessage.value = `Scanning failed: ${e}`;
  }
};

const saveName = async () => {
  try {
    await sync.rename(name.value);
  } catch (e) {
    joinOk.value = false;
    joinMessage.value = String(e);
  }
};

// Two-tap confirmation (native confirm() dialogs are unreliable in mobile webviews).
const pendingForget = ref<string | null>(null);
const pendingLeave = ref(false);

const confirmForget = async (p: SyncPeer) => {
  if (pendingForget.value !== p.id) {
    pendingForget.value = p.id;
    return;
  }
  pendingForget.value = null;
  await sync.forget(p.id);
};

const confirmLeave = async () => {
  if (!pendingLeave.value) {
    pendingLeave.value = true;
    return;
  }
  pendingLeave.value = false;
  await sync.leave();
};
</script>
