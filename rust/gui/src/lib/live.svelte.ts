// Things several screens show, kept fresh in the background.

import { get, post, type Database, type Hardware, type Job, type MlstDb, type RunDetail, type RunSummary, type Usage } from './api';
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

/** Something the app is working on right now, for the bar at the top of every page. */
export interface Activity {
  label: string;
  /** 0 to 100; null while it can't be told yet */
  percent: number | null;
  /** the page that shows it in full */
  to: string;
}

/** in `dbJobs`: the job that makes a database's Kun-peng copy is kept under this + the database id */
export const KUN_PENG = 'kun_peng:';

/**
 * The built-in typing tools that can have an up-to-date PubMLST database of
 * the user's own (their PubMLST key), by tool id, and how each is named on
 * screen. Each tool has its own key and database. Keep in step with `KINDS`
 * in gwm-server/src/mlstdb.rs.
 */
export const KEY_DBS = {
  mlst_typing: {
    tool: 'MLST typing',
    db: 'MLST',
    newer: 'alleles and sequence types',
    time: 'about 1.5 hours',
    builtin: 'PubMLST data to 31 December 2024, plus the current EnteroBase schemes.',
    /** Home asks the user to choose until they have */
    askOnHome: true,
  },
  spyogenes_typing: {
    tool: 'S. pyogenes typing',
    db: 'S. pyogenes',
    newer: 'emm, MLST, resistance and FCT alleles and sequence types',
    time: 'a few minutes',
    builtin: 'PubMLST data to 31 December 2024, with the emm cluster, emm pattern and pilin type tables.',
    askOnHome: false,
  },
} as const;
export type KeyDbTool = keyof typeof KEY_DBS;
export const KEY_DB_TOOLS = Object.keys(KEY_DBS) as KeyDbTool[];
export const isKeyDbTool = (id: string): id is KeyDbTool => id in KEY_DBS;

const TASK_DONE = ['COMPLETED', 'CACHED', 'SKIPPED'];

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
  /** Databases still to install. Of a set of alternatives (the three Kraken2 sizes) one is enough:
   *  once one is installed the others are no longer asked for (they stay on the Databases page).
   *  A Kraken2 database that was removed after its Kun-peng copy was made counts as there. */
  missingDb = $derived.by(() => {
    const have = (d: Database) => d.installed || !!d.kun_peng?.prepared;
    return this.databases.filter(
      (d) => !have(d) && !(d.alternatives && this.databases.some((o) => have(o) && o.alternatives === d.alternatives)),
    );
  });

  /** database id -> its install, going or just ended (kept here so it shows on every page) */
  dbJobs = $state<Record<string, Job>>({});
  /** per typing tool, its own PubMLST database: the choice, the key (never the key itself) and its download */
  keyDb = $state<Partial<Record<KeyDbTool, MlstDb>>>({});
  /** run id -> how far it is, 0 to 100 */
  runPercent = $state<Record<string, number>>({});

  /** What is being worked on right now: runs, installs, downloads, the app's own update. */
  activities = $derived.by(() => {
    const out: Activity[] = [];
    for (const r of this.runs.filter((r) => r.status === 'running' || r.status === 'queued'))
      out.push({ label: `Run "${r.name}"`, percent: this.runPercent[r.id] ?? null, to: `runs/${r.id}` });
    for (const j of Object.values(this.dbJobs).filter((j) => j.status === 'running'))
      out.push({
        label: j.title.replace(/^Install /, 'Installing ').replace(/^Prepare /, 'Preparing '),
        percent: j.progress && j.progress[1] ? Math.round((j.progress[0] / j.progress[1]) * 100) : null,
        to: 'databases',
      });
    for (const tool of KEY_DB_TOOLS) {
      const m = this.keyDb[tool]?.job;
      if (m?.status === 'running' && m.kind !== 'sign-in')
        out.push({
          label: m.kind === 'check' ? `Checking for a newer ${KEY_DBS[tool].db} database` : `Downloading the ${KEY_DBS[tool].db} database`,
          percent: m.progress && m.progress[1] ? Math.round((m.progress[0] / m.progress[1]) * 100) : null,
          to: 'databases',
        });
    }
    if (this.updating) out.push({ label: 'Updating the app', percent: this.updatePercent, to: 'settings' });
    return out;
  });

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
      return;
    }
    // how far each run that is going has got, for the bar at the top
    const percent: Record<string, number> = {};
    for (const r of this.runs.filter((r) => r.status === 'running')) {
      try {
        const d = await get<RunDetail>(`/api/runs/${encodeURIComponent(r.id)}`);
        let total = 0;
        for (const s of d.plan_steps ?? []) if (s.runnable) total += s.per_sample ? d.samples.length : 1;
        let done = 0;
        for (const s of Object.values(d.steps ?? {}))
          for (const t of Object.values(s.tasks ?? {})) if (TASK_DONE.includes(t.status)) done++;
        if (total) percent[r.id] = Math.min(100, Math.round((done / total) * 100));
      } catch {
        /* the bar shows it without a number */
      }
    }
    this.runPercent = percent;
  }

  /** Download and unpack a reference database (Databases page); it carries on when the page changes. */
  async installDatabase(id: string) {
    this.dbJobs[id] = await post<Job>(`/api/databases/${id}/install`);
  }
  /** Make the copy of an installed Kraken2 database that Kun-peng reads; followed like an install. */
  async prepareForKunPeng(id: string) {
    this.dbJobs[KUN_PENG + id] = await post<Job>(`/api/databases/${id}/kun-peng`);
  }
  async #followDbJobs() {
    for (const [db, j] of Object.entries(this.dbJobs)) {
      if (j.status !== 'running') continue;
      try {
        const next = await get<Job>(`/api/jobs/${j.id}`);
        this.dbJobs[db] = next;
        if (next.status === 'done') {
          toasts.show(
            db.startsWith(KUN_PENG)
              ? `${next.title.replace(/^Prepare /, '').replace(/ for Kun-peng$/, '')} is ready for Kun-peng.`
              : `${next.title.replace(/^Install /, '')} is installed.`,
          );
          this.refreshDatabases();
        } else if (next.status === 'failed') {
          toasts.show(next.log.at(-1) ?? 'The download failed.', 'error', undefined, 12000);
        }
      } catch {
        /* try again next tick */
      }
    }
  }

  /** Every typing tool's own PubMLST database. */
  refreshKeyDbs() {
    for (const tool of KEY_DB_TOOLS) this.refreshKeyDb(tool);
  }

  /** One typing tool's own PubMLST database; says so once when a download or a check ends. */
  async refreshKeyDb(tool: KeyDbTool) {
    let next: MlstDb;
    try {
      next = await get<MlstDb>(`/api/pubmlst-database/${tool}`);
    } catch {
      return;
    }
    const db = KEY_DBS[tool].db;
    const was = this.keyDb[tool]?.job;
    const now = next.job;
    this.keyDb[tool] = next;
    if (was?.status !== 'running' || !now || now.status === 'running') return;
    if (now.status === 'failed') {
      toasts.show(`${db} database: ${now.message ?? 'it did not work.'}`, 'error', undefined, 12000);
    } else if (now.kind === 'check') {
      toasts.show(
        now.result?.newer
          ? `A newer ${db} database is available (${now.result.schemes_newer} scheme${now.result.schemes_newer === 1 ? '' : 's'} changed). Update it on the Databases page.`
          : `Nothing is newer than your ${db} database: no need to update.`,
        now.result?.newer ? 'info' : 'ok',
        undefined,
        10000,
      );
    } else if (now.kind === 'download') {
      toasts.show(
        now.result?.changed
          ? `The up-to-date ${db} database is downloaded and in use.`
          : now.result?.own_database
            ? `Your ${db} database was already up to date: nothing to download.`
            : 'PubMLST gave nothing newer than the pre-2025 database, which stays in use.',
        'ok',
        undefined,
        10000,
      );
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
    this.refreshKeyDbs();
    setInterval(() => this.refreshUsage(), 4000);
    setInterval(() => this.refreshRuns(), 5000);
    setInterval(() => this.#followDbJobs(), 1500);
    // a download is followed closely; otherwise a look now and then is enough
    let tick = 0;
    setInterval(() => {
      const look = ++tick % 15 === 0;
      for (const tool of KEY_DB_TOOLS) if (look || this.keyDb[tool]?.job?.status === 'running') this.refreshKeyDb(tool);
    }, 2000);
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
