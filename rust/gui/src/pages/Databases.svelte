<script lang="ts">
  import { onDestroy } from 'svelte';
  import PageHeader from '../lib/PageHeader.svelte';
  import Button from '../lib/ui/Button.svelte';
  import Pill from '../lib/ui/Pill.svelte';
  import { get, post, type Job } from '../lib/api';
  import { live } from '../lib/live.svelte';
  import { errorText, toasts } from '../lib/toast.svelte';

  let jobs = $state<Record<string, Job>>({});
  live.refreshDatabases();

  const timer = setInterval(async () => {
    for (const [db, j] of Object.entries(jobs)) {
      if (j.status !== 'running') continue;
      try {
        const next = await get<Job>(`/api/jobs/${j.id}`);
        jobs[db] = next;
        if (next.status === 'done') {
          toasts.show(`${next.title.replace(/^Install /, '')} is installed.`);
          live.refreshDatabases();
        } else if (next.status === 'failed') {
          toasts.show(next.log.at(-1) ?? 'The download failed.', 'error');
        }
      } catch {
        /* try again next tick */
      }
    }
  }, 1500);
  onDestroy(() => clearInterval(timer));

  async function install(id: string) {
    try {
      jobs[id] = await post<Job>(`/api/databases/${id}/install`);
    } catch (e) {
      toasts.show(errorText(e), 'error');
    }
  }
  const pct = (j: Job) => (j.progress && j.progress[1] ? Math.round((j.progress[0] / j.progress[1]) * 100) : null);
</script>

<PageHeader title="Databases" />

<div class="flex w-full max-w-[1000px] flex-col gap-5 px-8 pt-7 pb-10 max-[720px]:px-4">
  <p class="m-0 max-w-[720px] text-sm text-muted">
    Some tools compare your genomes with a reference database. They are downloaded once and kept on this computer. If a
    download is interrupted, press Install again: it carries on where it stopped.
  </p>
  {#each live.databases as db (db.id)}
    {@const j = jobs[db.id]}
    <article class="card flex flex-col gap-3 p-5">
      <div class="flex flex-wrap items-center gap-3">
        <h2 class="m-0 text-base font-semibold">{db.name}</h2>
        <Pill status={db.installed ? 'installed' : 'missing'} />
        <span class="text-sm text-muted">Version {db.version} · {db.size_gb} GB</span>
        <div class="ml-auto">
          {#if !db.installed}
            <Button variant="primary" icon="download" busy={j?.status === 'running'} onclick={() => install(db.id)}>
              {j?.status === 'failed' ? 'Try again' : j?.status === 'running' ? 'Installing' : 'Install'}
            </Button>
          {/if}
        </div>
      </div>
      {#if db.description}<p class="m-0 text-[13px] leading-relaxed text-ink-2">{db.description}</p>{/if}
      {#if db.galaxy_note}<p class="m-0 rounded-lg bg-surface-2 px-3 py-2 text-[13px] text-ink-2"><strong>Compared with Galaxy:</strong> {db.galaxy_note}</p>{/if}
      {#if j && j.status !== 'done'}
        <div class="flex flex-col gap-1.5">
          <div class="h-2 overflow-hidden rounded-full bg-line" role="progressbar" aria-label="Download progress" aria-valuenow={pct(j) ?? 0} aria-valuemin={0} aria-valuemax={100}>
            <div class="h-full rounded-full {j.status === 'failed' ? 'bg-danger' : 'bg-brand'}" style="width: {pct(j) ?? 5}%"></div>
          </div>
          <span class="text-xs text-muted">
            {#if j.progress}{(j.progress[0] / 1e9).toFixed(2)} of {(j.progress[1] / 1e9).toFixed(2)} GB ·{/if}
            {j.log.at(-1) ?? ''}
          </span>
        </div>
      {/if}
      {#if db.installed}<span class="text-xs break-all text-muted">{db.path}</span>{/if}
    </article>
  {/each}
</div>
