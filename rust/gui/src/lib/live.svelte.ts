// Things several screens show, kept fresh in the background.

import { get, type Database, type Hardware, type RunSummary, type Usage } from './api';

class Live {
  usage = $state<Usage | null>(null);
  hardware = $state<Hardware | null>(null);
  runs = $state<RunSummary[]>([]);
  databases = $state<Database[]>([]);
  online = $state(true);

  running = $derived(this.runs.filter((r) => r.status === 'running' || r.status === 'queued').length);
  missingDb = $derived(this.databases.filter((d) => !d.installed));

  async refreshUsage() {
    try {
      this.usage = await get<Usage>('/api/usage');
      this.online = true;
    } catch {
      this.online = false;
    }
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
