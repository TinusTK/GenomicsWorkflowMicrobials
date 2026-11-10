<script lang="ts">
  // One run: live progress, the pipeline with a square per sample (click a step
  // for its own output), one results table per tool, files, details.
  import { onDestroy, untrack } from 'svelte';
  import PageHeader from '../lib/PageHeader.svelte';
  import Button from '../lib/ui/Button.svelte';
  import Dialog from '../lib/ui/Dialog.svelte';
  import Icon from '../lib/ui/Icon.svelte';
  import Pill from '../lib/ui/Pill.svelte';
  import { del, fileText, fileUrl, get, post, type PlanStep, type RunDetail, type Tool } from '../lib/api';
  import { openFolder } from '../lib/desktop';
  import { duration, live, size, when } from '../lib/live.svelte';
  import { asTable, combine, fileKind, stepFiles, stepViews, titleOf, type Table } from '../lib/results';
  import { go, href } from '../lib/router.svelte';
  import { errorText, toasts } from '../lib/toast.svelte';

  let { id }: { id: string } = $props();

  let run = $state<RunDetail | null>(null);
  let error = $state('');
  let tab = $state<'overview' | 'results' | 'files' | 'details'>('overview');
  /** the summary tables, by file */
  let tables = $state<Record<string, Table>>({});
  /** {tool: outputs switched off on the Tools page} */
  let hidden = $state<Record<string, string[]>>({});
  /** the picked step's tables, by view */
  let selTables = $state<Record<string, Table>>({});
  let selFile = $state('');
  let selText = $state<string | null>(null);
  let provenance = $state<Record<string, unknown> | null>(null);
  let selected = $state<string | null>(null);
  let confirmStop = $state(false);
  let confirmDelete = $state(false);
  let busy = $state(false);

  const active = $derived(run?.status === 'running' || run?.status === 'queued');

  async function load() {
    try {
      const next = await get<RunDetail>(`/api/runs/${encodeURIComponent(id)}`);
      const filesChanged = JSON.stringify(next.files?.map((f) => f.path)) !== JSON.stringify(run?.files?.map((f) => f.path));
      run = next;
      error = '';
      if (filesChanged) refreshResults();
    } catch (e) {
      error = errorText(e);
    }
  }
  async function refreshResults() {
    if (!run) return;
    asked.clear();
    tables = {};
    const files = (run.files ?? []).map((f) => f.path);
    if (files.includes('pipeline_info/provenance.json')) {
      const t = await fileText(run.id, 'pipeline_info/provenance.json');
      provenance = t ? JSON.parse(t) : null;
    }
  }
  get<Tool[]>('/api/tools')
    .then((t) => (hidden = Object.fromEntries(t.map((x) => [x.id, x.hidden_outputs]))))
    .catch(() => {});

  load();
  const timer = setInterval(() => active && load(), 3000);
  onDestroy(() => clearInterval(timer));

  // ---------------------------------------------------------------- pipeline drawing

  const COLORS: Record<string, string> = {
    COMPLETED: 'bg-ok',
    CACHED: 'bg-ok',
    RUNNING: 'bg-info animate-pulse',
    SUBMITTED: 'bg-info/40',
    FAILED: 'bg-danger',
    ABORTED: 'bg-warn',
    SKIPPED: 'bg-transparent border border-dashed border-muted',
  };
  const dot = (status?: string) => COLORS[status ?? ''] ?? 'bg-line-strong';

  /** columns of steps: each step one column right of the steps it reads from */
  const columns = $derived.by(() => {
    const steps = run?.plan_steps ?? [];
    const depth = new Map<string, number>();
    const byId = new Map(steps.map((s) => [s.id, s]));
    const d = (s: PlanStep, seen = new Set<string>()): number => {
      if (depth.has(s.id)) return depth.get(s.id)!;
      if (seen.has(s.id)) return 0;
      seen.add(s.id);
      let best = 1;
      for (const src of s.inputs) {
        const up = byId.get(src.split('.')[0]);
        if (up && up.runnable) best = Math.max(best, d(up, seen) + 1);
      }
      depth.set(s.id, best);
      return best;
    };
    steps.forEach((s) => d(s));
    const cols: PlanStep[][] = [];
    for (const s of steps) {
      const c = s.runnable ? depth.get(s.id)! : 1;
      (cols[c] ??= []).push(s);
    }
    return cols.filter(Boolean);
  });

  function tasksOf(s: PlanStep) {
    const t = run?.steps?.[s.id]?.tasks ?? {};
    return s.per_sample ? (run?.samples ?? []).map((smp) => ({ tag: smp, ...t[smp] })) : [{ tag: 'all', ...t['all'] }];
  }

  const progress = $derived.by(() => {
    let total = 0;
    let done = 0;
    for (const s of run?.plan_steps ?? []) {
      if (!s.runnable) continue;
      for (const t of tasksOf(s)) {
        total++;
        if (['COMPLETED', 'CACHED', 'SKIPPED'].includes(t.status ?? '')) done++;
      }
    }
    return total ? Math.round((done / total) * 100) : 0;
  });

  const sel = $derived(run?.plan_steps?.find((s) => s.id === selected) ?? null);
  const paths = $derived((run?.files ?? []).map((f) => f.path));
  /** one table per tool: the tables of the steps that write into the run's summary folder */
  const summaryFiles = $derived.by(() => {
    const steps = run?.plan_steps ?? [];
    if (!steps.some((s) => s.publish)) return paths.filter((p) => p.startsWith('summary/') && p.endsWith('.tsv')); // runs of older versions
    return steps
      .filter((s) => (s.publish ?? '').startsWith('summary'))
      .flatMap((s) => stepFiles(s, 'all', paths, hidden[s.tool_id ?? '']))
      .filter((p) => p.endsWith('.tsv'))
      .sort((a, b) => a.split('/').length - b.split('/').length); // the main tables first
  });
  /** one calm colour per results table, so each tool is easy to find again */
  const TINTS = ['#0f766e', '#6d28d9', '#b45309', '#1d4ed8', '#be185d', '#15803d', '#0e7490', '#a16207'];
  const tint = (i: number) => TINTS[i % TINTS.length];
  const asked = new Set<string>();
  $effect(() => {
    const rid = run?.id;
    if (!rid) return;
    for (const p of summaryFiles) {
      if (asked.has(p)) continue;
      asked.add(p);
      fileText(rid, p).then((text) => {
        if (text) tables = { ...tables, [p]: asTable(p, text) };
      });
    }
  });

  // the step picked in the pipeline, and its own output
  const selViews = $derived(
    sel && run ? stepViews(sel, run.samples, paths, hidden[sel.tool_id ?? '']) : { tables: [], others: [] },
  );
  function pick(sid: string) {
    selected = selected === sid ? null : sid;
    selFile = '';
    selText = null;
  }
  // every sample's rows of each output, in one table
  let loaded = '';
  $effect(() => {
    const rid = run?.id;
    const views = selViews.tables;
    const key = JSON.stringify(views);
    if (!rid || key === loaded) return;
    loaded = key;
    selTables = {};
    for (const v of views) {
      Promise.all(v.files.map(async (f) => ({ ...f, text: (await fileText(rid, f.path)) ?? '' }))).then((parts) => {
        if (loaded === key) selTables = { ...selTables, [v.key]: combine(parts) };
      });
    }
  });
  // the step's other files (reports, logs): one at a time
  async function showFile(path: string) {
    selFile = path;
    selText = null;
    if (!run || fileKind(path) !== 'text') return;
    const text = await fileText(run.id, path);
    if (selFile === path && text !== null) selText = text.split('\n').slice(0, 400).join('\n');
  }
  $effect(() => {
    // a step with no tables opens on its report page (else its first text file)
    const { tables, others } = selViews;
    if (others.includes(untrack(() => selFile))) return;
    const first = tables.length ? undefined : (others.find((p) => fileKind(p) === 'page') ?? others.find((p) => fileKind(p) === 'text'));
    untrack(() => (first ? showFile(first) : ((selFile = ''), (selText = null))));
  });

  // ---------------------------------------------------------------- actions

  async function stop() {
    busy = true;
    try {
      await post(`/api/runs/${encodeURIComponent(id)}/stop`);
      toasts.show('Stopping: the running steps are being stopped.', 'info');
      confirmStop = false;
      load();
    } catch (e) {
      toasts.show(errorText(e), 'error');
    } finally {
      busy = false;
    }
  }
  async function resume() {
    busy = true;
    try {
      await post(`/api/runs/${encodeURIComponent(id)}/resume`);
      toasts.show('Resumed: finished steps are kept, the rest runs now.');
      setTimeout(load, 800);
      live.refreshRuns();
    } catch (e) {
      toasts.show(errorText(e), 'error');
    } finally {
      busy = false;
    }
  }
  async function remove() {
    busy = true;
    try {
      await del(`/api/runs/${encodeURIComponent(id)}`);
      toasts.show(`Deleted "${run?.name}"`);
      live.refreshRuns();
      go('runs');
    } catch (e) {
      toasts.show(errorText(e), 'error');
      busy = false;
    }
  }
  const tools = $derived(Object.entries((provenance?.tools as Record<string, string>) ?? {}));
  const notes = $derived(Object.entries((provenance?.notes as Record<string, string>) ?? {}));
</script>

{#if error && !run}
  <PageHeader title="Run" crumbs={[{ label: 'Runs', href: href('runs') }]} />
  <p class="m-8 rounded-xl bg-danger-soft px-4 py-3 text-sm text-danger">{error}</p>
{:else if run}
  <PageHeader title={run.name} crumbs={[{ label: 'Runs', href: href('runs') }]}>
    {#snippet extra()}<Pill status={run!.status} />{/snippet}
    {#snippet actions()}
      {#if run!.folder}<Button icon="folder" onclick={() => openFolder(run!.folder!)}>Open run folder</Button>{/if}
      {#if active}
        <Button variant="danger" icon="stop" onclick={() => (confirmStop = true)}>Stop run</Button>
      {:else}
        {#if run!.status !== 'succeeded'}<Button variant="primary" icon="refresh" {busy} onclick={resume}>Resume</Button>{/if}
        <Button variant="danger" icon="trash" onclick={() => (confirmDelete = true)}>Delete</Button>
      {/if}
    {/snippet}
  </PageHeader>

  <div class="flex flex-col gap-3 border-b border-line bg-surface px-8 pb-0 max-[720px]:px-4">
    <div class="flex flex-wrap items-center gap-4">
      <div
        class="h-2.5 flex-[1_1_320px] overflow-hidden rounded-full bg-line"
        role="progressbar"
        aria-label="Run progress"
        aria-valuenow={progress}
        aria-valuemin={0}
        aria-valuemax={100}
      >
        <div class="h-full rounded-full transition-[width] duration-500 {run.status === 'failed' ? 'bg-danger' : 'bg-brand'}" style="width: {progress}%"></div>
      </div>
      <span class="text-sm font-semibold">{progress}%</span>
      <span class="text-sm text-muted">
        {active ? `Running for ${duration(run.started)}` : run.finished ? `Took ${duration(run.started, run.finished)}` : ''}
        · started {when(run.started)} · {run.workflow.name} · {run.samples.length} sample{run.samples.length === 1 ? '' : 's'}
      </span>
    </div>
    <nav class="flex gap-1" aria-label="Run sections">
      {#each [['overview', 'Overview'], ['results', 'Results'], ['files', 'Files'], ['details', 'Details']] as [t, label]}
        <button
          class="h-[42px] cursor-pointer border-0 bg-transparent px-3.5 text-sm {tab === t ? 'font-bold text-brand shadow-[inset_0_-3px_0_var(--brand)]' : 'font-medium text-muted hover:text-ink'}"
          onclick={() => (tab = t as typeof tab)}
          aria-current={tab === t ? 'page' : undefined}>{label}</button
        >
      {/each}
    </nav>
  </div>

  <div class="flex w-full max-w-[1240px] flex-col gap-5.5 px-8 pt-6 pb-10 max-[720px]:px-4">
    {#if run.error && !active}
      <section class="flex flex-col gap-2 rounded-xl border border-danger-line bg-danger-soft px-4 py-3.5 text-danger">
        <span class="flex items-center gap-2 font-semibold"><Icon name="alert" /> {run.status === 'stopped' ? 'The run was stopped' : 'Something went wrong'}</span>
        <pre class="m-0 max-h-56 overflow-auto font-mono text-xs leading-relaxed whitespace-pre-wrap text-ink-2">{run.error}</pre>
        <span class="text-[13px] text-ink-2">Fix the cause, then press <strong>Resume</strong>: the steps that finished are kept.</span>
      </section>
    {/if}

    {#if tab === 'overview'}
      <section class="card flex flex-col gap-4 px-5.5 py-5">
        <div class="flex flex-wrap items-center gap-3.5 text-xs text-muted">
          <h2 class="m-0 mr-2 text-base font-semibold text-ink">Pipeline</h2>
          <span class="flex items-center gap-1.5"><span class="h-2.5 w-2.5 rounded-[3px] bg-ok"></span>Done</span>
          <span class="flex items-center gap-1.5"><span class="h-2.5 w-2.5 rounded-[3px] bg-info"></span>Running</span>
          <span class="flex items-center gap-1.5"><span class="h-2.5 w-2.5 rounded-[3px] bg-line-strong"></span>Waiting</span>
          <span class="flex items-center gap-1.5"><span class="h-2.5 w-2.5 rounded-[3px] bg-danger"></span>Failed</span>
          <span class="ml-auto">One square per sample. Click a step to see its output.</span>
        </div>
        <div class="overflow-x-auto pb-1">
          <div class="flex min-w-max items-center">
            {#each columns as col, ci}
              <div class="flex w-[180px] flex-col justify-center gap-2">
                {#each col as s (s.id)}
                  <button
                    onclick={() => pick(s.id)}
                    class="flex w-[180px] cursor-pointer items-center gap-2 rounded-[10px] bg-surface px-2.5 py-2 text-left text-ink transition-shadow hover:shadow-[0_0_0_3px_var(--brand-soft)]
                      {selected === s.id ? 'border-2 border-brand' : s.runnable ? 'border border-line' : 'border border-dashed border-line-strong opacity-70'}"
                  >
                    <span class="flex min-w-0 flex-col">
                      <span class="truncate text-[13px] font-semibold">{s.label}</span>
                      <span class="truncate text-[11px] text-muted">{s.runnable ? `${s.tool} ${s.version}` : 'off'}</span>
                    </span>
                    {#if s.runnable}
                      <span class="ml-auto flex max-w-[60px] flex-wrap justify-end gap-[3px]">
                        {#each tasksOf(s) as t}<span class="h-2.5 w-2.5 rounded-[3px] {dot(t.status)}" title="{t.tag}: {t.status ?? 'waiting'}"></span>{/each}
                      </span>
                    {/if}
                  </button>
                {/each}
              </div>
              {#if ci < columns.length - 1}
                <svg width="34" height="20" viewBox="0 0 34 20" aria-hidden="true" class="shrink-0 text-line-strong"><path d="M4 10h24M22 4l6 6-6 6" stroke="currentColor" stroke-width="2" fill="none" stroke-linecap="round" stroke-linejoin="round" /></svg>
              {/if}
            {/each}
          </div>
        </div>
        {#if sel}
          <div class="flex flex-col gap-3 rounded-xl border border-brand/25 bg-brand-softer p-4 pl-5 shadow-[inset_4px_0_0_var(--brand)]">
            <div class="flex flex-wrap items-center gap-2">
              <span class="rounded-full bg-brand px-2.5 py-0.5 text-[11px] font-bold tracking-wide text-on-brand uppercase">Selected step</span>
              <strong class="text-[15px]">{sel.label}</strong><span class="text-sm text-muted">{sel.tool} {sel.version}</span>
              <Button size="sm" variant="quiet" class="ml-auto" icon="x" label="Close this step" onclick={() => pick(sel.id)} />
            </div>
            <table class="data">
              <thead><tr><th>Sample</th><th>Status</th><th>Started</th><th>Took</th></tr></thead>
              <tbody>
                {#each tasksOf(sel) as t}
                  <tr><td>{t.tag}</td><td>{t.status ? t.status.toLowerCase() : 'waiting'}</td><td>{when(t.started_at)}</td><td>{t.started_at ? duration(t.started_at, t.finished_at) : ''}</td></tr>
                {/each}
              </tbody>
            </table>
            <strong class="text-sm">Output{sel.per_sample && run.samples.length > 1 ? ' · all samples' : ''}</strong>
            {#each selViews.tables as v (v.key)}
              {@const t = selTables[v.key]}
              <div class="flex flex-col gap-1.5">
                <div class="flex flex-wrap items-center gap-2 text-[13px]">
                  <span class="font-semibold text-ink">{titleOf(v.title)}</span>
                  {#if v.files.length === 1}
                    <Button size="sm" variant="quiet" class="ml-auto" icon="download" href={fileUrl(run.id, v.files[0].path, true)}>Download</Button>
                  {/if}
                </div>
                {#if t}
                  <div class="max-h-[420px] overflow-auto rounded-xl border border-line bg-surface">
                    <table class="data">
                      {#if t.header.length}<thead class="sticky top-0"><tr>{#each t.header as h}<th>{h}</th>{/each}</tr></thead>{/if}
                      <tbody>
                        {#each t.rows.slice(0, 500) as r}<tr>{#each r as c}<td class="whitespace-nowrap">{c}</td>{/each}</tr>{/each}
                        {#if !t.rows.length}<tr><td class="text-muted" colspan={Math.max(1, t.header.length)}>Nothing found.</td></tr>{/if}
                      </tbody>
                    </table>
                  </div>
                {:else}
                  <p class="m-0 text-[13px] text-muted">Loading…</p>
                {/if}
              </div>
            {/each}
            {#if selViews.others.length}
              <div class="flex flex-wrap items-center gap-1.5" role="tablist" aria-label="Other files of this step">
                {#if selViews.tables.length}<span class="mr-1 text-[13px] text-muted">Other files</span>{/if}
                {#each selViews.others as p}
                  <button
                    role="tab"
                    aria-selected={selFile === p}
                    class="h-9 cursor-pointer rounded-full border px-3.5 text-[13px] font-semibold {selFile === p ? 'border-brand bg-brand-soft text-brand' : 'border-line bg-surface text-ink-2 hover:bg-hover'}"
                    onclick={() => showFile(p)}>{p.split('/').pop()}</button
                  >
                {/each}
              </div>
              {#if selFile}
                <div class="flex justify-end gap-2">
                  {#if fileKind(selFile) !== 'other'}<Button size="sm" variant="quiet" href={fileUrl(run.id, selFile)}>Open</Button>{/if}
                  <Button size="sm" icon="download" href={fileUrl(run.id, selFile, true)}>Download</Button>
                </div>
                {#if selText !== null}
                  <pre class="m-0 max-h-[420px] overflow-auto rounded-xl border border-line bg-surface p-3 font-mono text-xs leading-relaxed whitespace-pre-wrap text-ink-2">{selText}</pre>
                {:else if fileKind(selFile) === 'page'}
                  <iframe src={fileUrl(run.id, selFile)} title={selFile.split('/').pop()} sandbox="allow-scripts" class="h-[520px] w-full rounded-xl border border-line bg-white"></iframe>
                {:else if fileKind(selFile) === 'other'}
                  <p class="m-0 text-[13px] text-ink-2">This file can't be shown here: press <strong>Download</strong>.</p>
                {/if}
              {/if}
            {/if}
            {#if !selViews.tables.length && !selViews.others.length}
              <p class="m-0 text-[13px] text-muted">
                {sel.runnable ? 'No output files yet (or they are switched off for this tool on the Tools page).' : 'This step was off.'}
              </p>
            {/if}
          </div>
        {/if}
      </section>
    {/if}

    {#if tab === 'overview' || tab === 'results'}
      <section class="mt-2 flex flex-col gap-3.5">
        <div class="flex flex-wrap items-center gap-3">
          <h2 class="m-0 text-lg font-semibold">{active ? 'Results so far' : 'Results'}</h2>
          <span class="text-[13px] text-muted">One table per tool, every sample together</span>
          <span class="h-px min-w-10 flex-1 bg-line"></span>
        </div>
        {#each summaryFiles as p, i (p)}
          <div class="card flex flex-col gap-3 px-5.5 py-4.5" style="border-left: 4px solid {tint(i)}">
            <div class="flex flex-wrap items-center gap-2.5">
              <h3
                class="m-0 rounded-full px-3 py-1 text-[14px] font-semibold"
                style="color: {tint(i)}; background: color-mix(in srgb, {tint(i)} 11%, transparent)"
              >
                {titleOf(p)}
              </h3>
              <span class="text-xs text-muted">{p}</span>
              <Button size="sm" class="ml-auto" icon="download" href={fileUrl(run.id, p, true)}>Download</Button>
            </div>
            {#if tables[p]}
              <div class="max-h-[380px] overflow-auto rounded-xl border border-line">
                <table class="data">
                  {#if tables[p].header.length}<thead class="sticky top-0"><tr>{#each tables[p].header as h}<th>{h}</th>{/each}</tr></thead>{/if}
                  <tbody>{#each tables[p].rows.slice(0, 500) as r}<tr>{#each r as c}<td class="whitespace-nowrap">{c}</td>{/each}</tr>{/each}</tbody>
                </table>
              </div>
            {:else}
              <p class="m-0 text-[13px] text-muted">Loading…</p>
            {/if}
          </div>
        {:else}
          <p class="card m-0 px-5.5 py-4.5 text-sm text-muted">
            {active
              ? 'Each tool gets its own table here when the run finishes. Until then, click a step in the pipeline to see its output.'
              : 'No summary tables in this run. Click a step in the pipeline to see its output.'}
          </p>
        {/each}
      </section>
    {/if}

    {#if tab === 'files'}
      <section class="card flex flex-col gap-2 px-5.5 py-5">
        <div class="flex items-center gap-3">
          <h2 class="m-0 text-base font-semibold">Files</h2>
          <span class="text-[13px] text-muted">in the run folder's Results</span>
          {#if run.folder}<Button size="sm" class="ml-auto" icon="folder" onclick={() => openFolder(run!.folder!)}>Open run folder</Button>{/if}
        </div>
        <div class="overflow-x-auto">
          <table class="data">
            <tbody>
              {#each run.files ?? [] as f}
                <tr>
                  <td class="font-mono text-[13px] break-all">{f.path}</td>
                  <td class="whitespace-nowrap text-muted">{size(f.size)}</td>
                  <td class="whitespace-nowrap text-right">
                    {#if /\.(html|tsv|txt|tab|json|yml|log|csv)$/.test(f.path)}<Button size="sm" variant="quiet" href={fileUrl(run.id, f.path)}>View</Button>{/if}
                    <Button size="sm" variant="quiet" icon="download" label="Download {f.path}" href={fileUrl(run.id, f.path, true)} />
                  </td>
                </tr>
              {:else}
                <tr><td class="text-muted">No result files yet.</td></tr>
              {/each}
            </tbody>
          </table>
        </div>
      </section>
    {/if}

    {#if tab === 'details'}
      <section class="card flex flex-col gap-4 px-5.5 py-5">
        <h2 class="m-0 text-base font-semibold">Details for the lab record</h2>
        <div class="grid grid-cols-[repeat(auto-fit,minmax(240px,1fr))] gap-2.5 text-sm">
          <div class="rounded-[10px] bg-surface-2 px-3.5 py-3"><span class="block text-xs text-muted">App</span>Genomics Workflow Microbials {run.engine?.version ?? ''}</div>
          <div class="rounded-[10px] bg-surface-2 px-3.5 py-3"><span class="block text-xs text-muted">Containers</span>{run.engine?.containers ?? ''}</div>
          <div class="rounded-[10px] bg-surface-2 px-3.5 py-3"><span class="block text-xs text-muted">Computer budget</span>{run.limits ? `${run.limits.cpus} threads, ${run.limits.memory_gb} GB` : ''}</div>
          <div class="rounded-[10px] bg-surface-2 px-3.5 py-3 break-all"><span class="block text-xs text-muted">Reads</span>{run.input}</div>
          <div class="rounded-[10px] bg-surface-2 px-3.5 py-3"><span class="block text-xs text-muted">Steps switched off</span>{run.steps_off?.length ? run.steps_off.join(', ') : 'none'}</div>
          {#each notes as [k, v]}<div class="rounded-[10px] bg-surface-2 px-3.5 py-3"><span class="block text-xs text-muted">{k}</span>{v}</div>{/each}
        </div>
        {#if tools.length}
          <h3 class="m-0 mt-2 text-sm font-semibold">Tool versions</h3>
          <div class="overflow-x-auto rounded-xl border border-line">
            <table class="data"><tbody>{#each tools as [k, v]}<tr><td class="font-semibold">{k}</td><td>{v}</td></tr>{/each}</tbody></table>
          </div>
        {/if}
        <div class="flex flex-wrap gap-2">
          <Button size="sm" icon="file" href={fileUrl(run.id, 'pipeline_info/provenance.json')}>Provenance file</Button>
          <Button size="sm" icon="file" href={fileUrl(run.id, 'pipeline_info/tool_versions.yml')}>Tool versions file</Button>
        </div>
      </section>
    {/if}
  </div>

  <Dialog bind:open={confirmStop} title="Stop this run?">
    The running steps are stopped now. Finished steps are kept, so you can <strong>Resume</strong> later.
    {#snippet actions()}
      <Button onclick={() => (confirmStop = false)}>Keep running</Button>
      <Button variant="danger-solid" {busy} onclick={stop}>Stop run</Button>
    {/snippet}
  </Dialog>
  <Dialog bind:open={confirmDelete} title={`Delete the run "${run.name}"?`}>
    Its folder goes: results, run details and working files. Your read files are not touched. This can't be undone.
    {#snippet actions()}
      <Button onclick={() => (confirmDelete = false)}>Keep it</Button>
      <Button variant="danger-solid" {busy} onclick={remove}>Delete run</Button>
    {/snippet}
  </Dialog>
{:else}
  <PageHeader title="Run" crumbs={[{ label: 'Runs', href: href('runs') }]} />
  <p class="m-8 text-sm text-muted">Loading…</p>
{/if}
