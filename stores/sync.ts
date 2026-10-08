/* SPDX-FileCopyrightText: Copyright (c) 2026 kanri-sync contributors

SPDX-License-Identifier: GPL-3.0-or-later
*/

import { defineStore } from "pinia";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export type SyncState = "disabled" | "unpaired" | "idle" | "syncing" | "offline" | "error";

export interface SyncPeer {
  id: string;
  name: string;
  addrs: string[];
  state: "ok" | "offline" | "error" | "unknown";
  lastOk: number | null;
  lastAttempt: number | null;
  lastError: string | null;
}

export interface SyncLogEntry {
  t: number;
  level: "info" | "warn" | "error";
  msg: string;
}

export interface SyncStatus {
  enabled: boolean;
  paired: boolean;
  deviceId: string;
  deviceName: string;
  port: number;
  addrs: string[];
  state: SyncState;
  lastSync: number | null;
  lastError: string | null;
  discoveryError: string | null;
  peers: SyncPeer[];
  log: SyncLogEntry[];
}

export interface PairingInfo {
  code: string;
  qrSvg: string;
  expiresAt: number;
  addrs: string[];
}

export const useSyncStore = defineStore("sync", {
  state: () => ({
    status: null as SyncStatus | null,
    /** Result of the last sync the user started with the sync button. */
    manualResult: null as { ok: boolean; message: string; at: number } | null,
    manualRunning: false,
    lastPairedWith: null as string | null,
    initialized: false,
  }),
  getters: {
    state: (s): SyncState => s.status?.state ?? "unpaired",
    hasError: (s) => !!s.status?.lastError,
  },
  actions: {
    async init() {
      if (this.initialized) return;
      this.initialized = true;

      await listen<SyncStatus>("sync://status", (e) => {
        this.status = e.payload;
      });
      await listen("sync://data-changed", async () => {
        const boards = useBoardsStore();
        await boards.reloadFromStorage();
      });
      await listen<string>("sync://paired", (e) => {
        this.lastPairedWith = e.payload;
      });

      await this.refresh();

      // Sync when the app comes back to the foreground (e.g. phone unlocked).
      document.addEventListener("visibilitychange", () => {
        if (document.visibilityState === "visible" && this.status?.paired) {
          this.syncNow(false).catch(() => {});
        }
      });
    },
    async refresh() {
      try {
        this.status = await invoke<SyncStatus>("sync_status");
      } catch (e) {
        console.error("Failed to read sync status", e);
      }
    },
    /** Runs a sync with all paired devices. `manual` shows the result to the user. */
    async syncNow(manual = true) {
      if (manual) this.manualRunning = true;
      try {
        const message = await invoke<string>("sync_now");
        if (manual) this.manualResult = { ok: true, message, at: Date.now() };
      } catch (e) {
        if (manual) this.manualResult = { ok: false, message: String(e), at: Date.now() };
      } finally {
        if (manual) this.manualRunning = false;
        await this.refresh();
      }
    },
    async notifyLocalChange() {
      try {
        await invoke("sync_local_change");
      } catch (e) {
        console.error("Sync could not record local change", e);
      }
    },
    startPairing() {
      return invoke<PairingInfo>("sync_start_pairing");
    },
    stopPairing() {
      return invoke("sync_stop_pairing");
    },
    async join(code: string) {
      const name = await invoke<string>("sync_join", { code });
      await this.refresh();
      return name;
    },
    async rename(name: string) {
      await invoke("sync_rename_device", { name });
      await this.refresh();
    },
    async setEnabled(enabled: boolean) {
      await invoke("sync_set_enabled", { enabled });
      await this.refresh();
    },
    async forget(id: string) {
      await invoke("sync_forget_device", { id });
      await this.refresh();
    },
    async leave() {
      await invoke("sync_leave_group");
      await this.refresh();
    },
    async clearError() {
      this.manualResult = null;
      await invoke("sync_clear_error");
      await this.refresh();
    },
  },
});
