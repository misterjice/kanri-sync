<!-- SPDX-FileCopyrightText: Copyright (c) 2026 kanri-sync contributors -->
<!-- -->
<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

<!-- Sync button with a coloured status dot. Tap: sync now. Long-press or
     right-click: open sync settings. -->
<template>
  <Tooltip :label="label">
    <template #trigger>
      <button
        class="bg-elevation-2-hover transition-button relative rounded-md p-2"
        :aria-label="label"
        @click="onClick"
        @contextmenu.prevent="$router.push('/sync')"
        @touchstart.passive="startPress"
        @touchend="cancelPress"
        @touchmove.passive="cancelPress"
      >
        <PhArrowsClockwise
          class="size-7"
          :class="{ 'animate-spin': busy }"
        />
        <span
          class="border-bg-primary absolute right-1 top-1 size-3 rounded-full border-2"
          :class="dotClass"
        />
      </button>
    </template>
  </Tooltip>
</template>

<script setup lang="ts">
import { PhArrowsClockwise } from "@phosphor-icons/vue";

const sync = useSyncStore();
const router = useRouter();

const busy = computed(() => sync.manualRunning || sync.state === "syncing");

const dotClass = computed(() => {
  switch (sync.state) {
    case "idle":
      return "bg-green-500";
    case "syncing":
      return "bg-sky-500";
    case "offline":
      return "bg-amber-400";
    case "error":
      return "bg-red-500";
    default:
      return "bg-gray-500";
  }
});

const label = computed(() => {
  switch (sync.state) {
    case "idle":
      return "Synced — tap to sync now";
    case "syncing":
      return "Syncing…";
    case "offline":
      return "Other devices not reachable — tap to retry";
    case "error":
      return "Sync error — tap to retry, hold for details";
    case "disabled":
      return "Sync is off — tap to open sync settings";
    default:
      return "Not paired — tap to set up sync";
  }
});

let pressTimer: ReturnType<typeof setTimeout> | null = null;
let longPressed = false;

const startPress = () => {
  longPressed = false;
  pressTimer = setTimeout(() => {
    longPressed = true;
    router.push("/sync");
  }, 600);
};

const cancelPress = () => {
  if (pressTimer) clearTimeout(pressTimer);
  pressTimer = null;
};

const onClick = () => {
  if (longPressed) {
    longPressed = false;
    return;
  }
  if (sync.state === "unpaired" || sync.state === "disabled") {
    router.push("/sync");
    return;
  }
  sync.syncNow(true);
};
</script>
