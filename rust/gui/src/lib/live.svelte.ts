// Things several screens show, kept fresh in the background.

import { get, type Database, type Hardware, type RunSummary, type Usage } from './api';
import { invoke, isDesktop, openLink } from './desktop';
import { toasts } from './toast.svelte';

/** Whether a newer version of the app is out (GitHub releases). */
export interface AppUpdate {
  current: string;
  latest: string | null;
  newer: boolean;
  url: string;
  /** the points under "What is new" in the newest release's notes */
  whats_new?: { title: string; text: string }[];
}

class Live {
  usage = $state<Usage | null>(null);
  hardware = $state<Hardware | null>(null);
  runs = $state<RunSummary[]>([]);
  databases = $state<Database[]>([]);
  online = $state(true);
  /** the newest release, once looked up (at every start, and from Settings) */
  update = $state<AppUpdate | null>(null);
  #lookedForUpdate = false;
  /** the desktop app is downloading or installing an update; the percentage once the size is known */
  updating = $state(false);
  updatePercent = $state<number | null>(null);

  running = $derived(this.runs.filter((r) => r.status === 'running' || r.status === 'queued').length);
  missingDb = $derived(this.databases.filter((d) => !d.installed));

  async refreshUsage() {
    try {
      this.usage = await get<Usage>('/api/usage');
      this.online = true;
      // every start: say so when a newer version is out (quietly skipped with no internet)
      if (!this.#lookedForUpdate) {
        this.#lookedForUpdate = true;
        // Windows: the app gave WSL its memory by itself (nothing was set): say so
        if (isDesktop())
          invoke<number | null>('memory_notice')
            .then((gb) => {
              if (gb)
                toasts.show(
                  `The tools were given ${gb} GB of this computer's memory (4 GB stay for Windows). It is used after the next Stop and Start; change it under Settings › Memory for the tools.`,
                  'info',
                  undefined,
                  20000,
                );
            })
            .catch(() => {});
        this.checkUpdate()
          .then((u) => {
            if (!u.newer) return;
            const action = isDesktop() ? { label: 'Update now', run: () => void this.installUpdate() } : { label: 'Download', run: () => openLink(u.url) };
            toasts.show(`Version ${u.latest} is out (you have ${u.current}).`, 'info', action, 20000);
          })
          .catch(() => {});
      }
    } catch {
      this.online = false;
    }
  }
  /**
   * Desktop app: download the newest version, install it over this one and open
   * it again (the app closes by itself). When that can't be done, the download
   * page is offered instead.
   */
  async installUpdate() {
    if (this.updating) return;
    this.updating = true;
    this.updatePercent = null;
    const watch = setInterval(async () => {
      const p = await invoke<{ done: number; total: number }>('update_progress').catch(() => null);
      if (p && p.total > 0) this.updatePercent = Math.min(100, Math.round((p.done / p.total) * 100));
    }, 400);
    try {
      await invoke('update_install');
    } catch (e) {
      const page = this.update?.url ?? 'https://github.com/TinusTK/GenomicsWorkflowMicrobials/releases/latest';
      toasts.show(String(e), 'error', { label: 'Download page', run: () => openLink(page) }, 10000);
    } finally {
      clearInterval(watch);
      this.updating = false;
      this.updatePercent = null;
    }
  }
  /** Ask GitHub for the newest release. */
  async checkUpdate() {
    this.update = await get<AppUpdate>('/api/update');
    return this.update;
  }
  async refreshRuns() {
    try {
      this.runs = await get<RunSummary[]>('/api/runs');
    } catch {
      /* shown by the online flag */
    }
  }
  async refreshHardware() {
    try {
      this.hardware = await get<Hardware>('/api/hardware');
    } catch {
      /* shown by the online flag */
    }
  }
  async refreshDatabases() {
    try {
      this.databases = await get<Database[]>('/api/databases');
    } catch {
      /* shown by the online flag */
    }
  }

  start() {
    this.refreshUsage();
    this.refreshRuns();
    this.refreshHardware();
    this.refreshDatabases();
    setInterval(() => this.refreshUsage(), 4000);
    setInterval(() => this.refreshRuns(), 5000);
    // until the containers are ready, keep looking (Docker can take a minute to start)
    setInterval(() => {
      if (!this.hardware?.docker && !this.hardware?.apptainer) this.refreshHardware();
    }, 15000);
  }
}

export const live = new Live();

/** "12 Nov 09:54", or "Today 09:54" */
export function when(iso: string | null | undefined) {
  if (!iso) return '';
  const d = new Date(iso);
  if (isNaN(+d)) return iso;
  const t = d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  const today = new Date();
  const y = new Date(today);
  y.setDate(today.getDate() - 1);
  if (d.toDateString() === today.toDateString()) return `Today ${t}`;
  if (d.toDateString() === y.toDateString()) return `Yesterday ${t}`;
  return `${d.toLocaleDateString([], { day: 'numeric', month: 'short' })} ${t}`;
}

export function duration(a: string | null | undefined, b?: string | null) {
  if (!a) return '';
  const ms = (b ? +new Date(b) : Date.now()) - +new Date(a);
  if (!(ms > 0)) return '';
  const m = Math.round(ms / 60000);
  return m < 60 ? `${m} min` : `${Math.floor(m / 60)} h ${m % 60} min`;
}

export function size(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 ** 2) return `${(bytes / 1024).toFixed(0)} KB`;
  if (bytes < 1024 ** 3) return `${(bytes / 1024 ** 2).toFixed(1)} MB`;
  return `${(bytes / 1024 ** 3).toFixed(2)} GB`;
}
