<!-- SPDX-FileCopyrightText: Copyright (c) 2026 kanri-sync contributors -->
<!-- -->
<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

<!-- Makes sync problems impossible to miss: a red banner stays until the
     problem is fixed or dismissed; results of manual syncs show briefly. -->
<template>
  <div
    v-if="visible"
    class="fixed inset-x-0 bottom-4 z-[100] flex justify-end px-4 max-sm:bottom-20 max-sm:justify-center max-sm:px-3"
  >
    <div
      class="flex max-w-2xl flex-row items-start gap-3 rounded-lg px-4 py-3 text-sm text-white shadow-lg"
      :class="tone === 'error' ? 'bg-red-600' : tone === 'warn' ? 'bg-amber-600' : 'bg-green-700'"
      role="alert"
    >
      <div class="flex-1">
        <p class="font-semibold">{{ title }}</p>
        <p class="whitespace-pre-line opacity-90">{{ message }}</p>
      </div>
      <div class="flex shrink-0 flex-row gap-2">
        <button
          v-if="tone !== 'ok'"
          class="rounded-md bg-white/20 px-2 py-1 hover:bg-white/30"
          @click="sync.syncNow(true)"
        >
          Retry
        </button>
        <button
          v-if="tone !== 'ok'"
          class="rounded-md bg-white/20 px-2 py-1 hover:bg-white/30"
          @click="$router.push('/sync')"
        >
          Details
        </button>
        <button
          class="rounded-md bg-white/20 px-2 py-1 hover:bg-white/30"
          aria-label="Dismiss"
          @click="dismiss"
        >
          ✕
        </button>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
const sync = useSyncStore();
const route = useRoute();

const dismissedError = ref<string | null>(null);
const now = useNow({ interval: 1000 });

const manualRecent = computed(
  () => sync.manualResult && now.value.getTime() - sync.manualResult.at < (sync.manualResult.ok ? 3000 : 15000),
);

const persistentError = computed(() => {
  const e = sync.status?.lastError ?? null;
  return e && e !== dismissedError.value ? e : null;
});

const tone = computed<"error" | "warn" | "ok">(() => {
  if (persistentError.value) return "error";
  if (manualRecent.value && !sync.manualResult!.ok) {
    return sync.manualResult!.message.startsWith("No paired devices were reachable") ? "warn" : "error";
  }
  return "ok";
});

const visible = computed(() => {
  // The sync page shows the same information in full.
  if (route.path === "/sync") return false;
  return !!persistentError.value || !!manualRecent.value;
});

const title = computed(() => {
  if (tone.value === "error") return "Sync problem";
  if (tone.value === "warn") return "Couldn't reach your other devices";
  return "Sync complete";
});

const message = computed(() => {
  if (persistentError.value) return persistentError.value;
  return sync.manualResult?.message ?? "";
});

const dismiss = () => {
  dismissedError.value = sync.status?.lastError ?? null;
  sync.manualResult = null;
};
</script>
