<script lang="ts">
  // New run in four steps: the input files (reads or FASTA files), workflow, check, start.
  import { untrack } from 'svelte';
  import FolderPicker from '../lib/FolderPicker.svelte';
  import PageHeader from '../lib/PageHeader.svelte';
  import Button from '../lib/ui/Button.svelte';
  import Icon from '../lib/ui/Icon.svelte';
  import Pill from '../lib/ui/Pill.svelte';
  import Toggle from '../lib/ui/Toggle.svelte';
  import { get, post, type Found, type Warning, type WorkflowSummary, type WorkflowView } from '../lib/api';
  import { live } from '../lib/live.svelte';
  import { go, href } from '../lib/router.svelte';
  import { errorText, toasts } from '../lib/toast.svelte';

  let { workflow: startWorkflow = '' }: { workflow?: string } = $props();

  let step = $state(1);
  // 1 reads
  let input = $state('');
  let picking = $state(false);
  let found = $state<Found | null>(null);
  let problems = $state<string[]>([]);
  // what the files are: read pairs, or FASTA files (assemblies), one sample each
  let kind = $state<'paired_reads' | 'assemblies'>('paired_reads');
  const fasta = $derived(kind === 'assemblies');
  let custom = $state(false);
  let r1 = $state('_R1');
  let r2 = $state('_R2');
  // 2 workflow
  let workflows = $state<WorkflowSummary[]>([]);
  // the page is re-made when the address changes, so the first value is the one
  let wfId = $state(untrack(() => startWorkflow));
  let wfVersion = $state<number | null>(null);
  // 3 check
  let view = $state<WorkflowView | null>(null);
  let off = $state<Record<string, boolean>>({});
  let warnings = $state<Warning[]>([]);
  let runnable = $state<Record<string, boolean>>({});
  let checking = $state(false);
  // 4 start
  let name = $state('');
  let keepWork = $state(false);
  let starting = $state(false);

  get<WorkflowSummary[]>('/api/workflows')
    .then((w) => {
      workflows = w;
      // started from a workflow: the files are what that workflow takes
      const asked = w.find((x) => x.id === wfId);
      if (asked?.input_type === 'assemblies') kind = 'assemblies';
      if (!wfId && w.length) wfId = w[0].id;
    })
    .catch((e) => toasts.show(errorText(e), 'error'));

  const pairing = $derived(custom ? { mode: 'custom', r1, r2 } : { mode: 'auto', r1: '', r2: '' });

  async function pair() {
    if (!input.trim()) {
      found = null;
      return;
    }
    try {
      const res = await post<{ found: Found; problems: string[] }>('/api/reads', { input: input.trim(), pairing, input_type: kind });
      found = res.found;
      problems = res.problems;
      if (!custom && !fasta) {
        r1 = res.found.r1;
        r2 = res.found.r2;
      }
    } catch (e) {
      found = null;
      problems = [errorText(e)];
    }
  }

  const chosen = $derived(workflows.find((w) => w.id === wfId));
  /** A workflow runs on the kind of files chosen in step 1. */
  const fits = (w: WorkflowSummary) => (w.input_type === 'assemblies') === fasta;
  function setKind(k: 'paired_reads' | 'assemblies') {
    if (k === kind) return;
    kind = k;
    custom = false;
    // a workflow for the other kind of files no longer fits
    if (chosen && !fits(chosen)) wfId = workflows.find(fits)?.id ?? '';
    pair();
  }

  async function loadWorkflow() {
    if (!wfId) return;
    const v = wfVersion ?? chosen?.latest;
    view = await get<WorkflowView>(`/api/workflows/${encodeURIComponent(wfId)}${v ? `?version=${v}` : ''}`);
    // steps that can't run here (missing database) start switched off
    const next: Record<string, boolean> = {};
    for (const id of view.order) next[id] = off[id] ?? false;
    off = next;
    await check(true);
  }

  async function check(autoOff = false) {
    if (!view) return;
    checking = true;
    try {
      const res = await post<{ warnings: Warning[]; runnable: Record<string, boolean> | null }>('/api/workflows/check', {
        workflow: view.workflow,
        input: input.trim() || null,
        steps_off: Object.keys(off).filter((k) => off[k]),
        pairing,
      });
      if (autoOff) {
        const needsDb = res.warnings.filter((w) => w.level === 'error' && w.step && /database/.test(w.message)).map((w) => w.step!);
        if (needsDb.length) {
          for (const s of needsDb) off[s] = true;
          return check(false);
        }
      }
      warnings = res.warnings;
      runnable = res.runnable ?? {};
    } catch (e) {
      toasts.show(errorText(e), 'error');
    } finally {
      checking = false;
    }
  }

  const blockers = $derived(warnings.filter((w) => w.level === 'error'));
  const stepsOn = $derived(view ? view.order.filter((id) => runnable[id]).length : 0);

  const titles = [
    ['Files', 'Pick the folder'],
    ['Workflow', 'What to run'],
    ['Check', 'Steps and space'],
    ['Start', 'Name it, go'],
  ];
  const canNext = $derived(
    step === 1 ? !!found && found.samples.length > 0 && problems.length === 0 : step === 2 ? !!chosen && fits(chosen) : step === 3 ? blockers.length === 0 && !checking : true,
  );

  async function next() {
    if (step === 2) {
      try {
        await loadWorkflow();
      } catch (e) {
        toasts.show(errorText(e), 'error');
        return;
      }
    }
    if (step === 3 && !name) name = view?.workflow.name ?? '';
    step = Math.min(4, step + 1);
  }

  async function start() {
    starting = true;
    try {
      const run = await post<{ id: string }>('/api/runs', {
        workflow: wfId,
        version: wfVersion ?? chosen?.latest ?? null,
        input: input.trim(),
        name: name.trim() || null,
        cleanup: !keepWork,
        steps_off: Object.keys(off).filter((k) => off[k]),
        pairing,
      });
      live.refreshRuns();
      go(`runs/${encodeURIComponent(run.id)}`);
    } catch (e) {
      toasts.show(errorText(e), 'error');
      starting = false;
    }
  }

  const base = (p: string | null) => (p ? p.split(/[\\/]/).pop() : '');
  const folderPreview = $derived.by(() => {
    const d = new Date();
    const pad = (n: number) => String(n).padStart(2, '0');
    return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}-${pad(d.getMinutes())} ${name || view?.workflow.name || ''}`;
  });
</script>

<PageHeader title="New run" crumbs={[{ label: 'Home', href: href('home') }]} />

<div class="flex w-full max-w-[980px] flex-col gap-6 px-8 pt-7 pb-10 max-[720px]:px-4">
  <ol class="m-0 grid list-none grid-cols-4 gap-2.5 p-0" aria-label="Steps">
    {#each titles as [t, sub], i}
      {@const n = i + 1}
      <li class="flex flex-col gap-2">
        <span class="block h-1 rounded {n <= step ? 'bg-brand' : 'bg-line'}"></span>
        <button
          class="flex cursor-pointer items-center gap-2.5 border-0 bg-transparent p-0 text-left text-ink disabled:cursor-default"
          disabled={n > step}
          onclick={() => (step = n)}
          aria-current={n === step ? 'step' : undefined}
        >
          <span
            class="flex h-7 w-7 shrink-0 items-center justify-center rounded-full text-[13px] font-bold
              {n === step ? 'bg-brand text-on-brand' : n < step ? 'bg-brand-soft text-brand' : 'bg-surface-2 text-muted'}"
          >
            {#if n < step}<Icon name="check" size={15} stroke={3} />{:else}{n}{/if}
          </span>
          <span class="flex flex-col max-[720px]:hidden">
            <span class="text-sm {n === step ? 'font-bold' : 'font-semibold'} {n <= step ? 'text-ink' : 'text-muted'}">{t}</span>
            <span class="text-xs text-muted">{sub}</span>
          </span>
        </button>
      </li>
    {/each}
  </ol>

  <section class="card flex flex-col gap-5 rounded-2xl p-7 max-[720px]:p-4">
    {#if step === 1}
      <div>
        <h2 class="m-0 mb-1 text-xl font-semibold">Which files?</h2>
        <p class="m-0 text-sm text-muted">
          {fasta
            ? 'Pick the folder with the FASTA files (assemblies). Each file is one sample, named after the file.'
            : 'Pick the folder with the paired FASTQ files. The app pairs R1 and R2 for you.'}
        </p>
      </div>
      <div class="flex flex-wrap gap-2.5" role="radiogroup" aria-label="What the files are">
        {#each [['paired_reads', 'Paired reads', 'FASTQ files, two per sample'], ['assemblies', 'Assembled contigs', 'FASTA files, one per sample']] as [k, title, what] (k)}
          <button
            role="radio"
            aria-checked={kind === k}
            onclick={() => setKind(k as 'paired_reads' | 'assemblies')}
            class="flex min-w-[220px] flex-1 cursor-pointer flex-col items-start gap-0.5 rounded-xl border-2 px-4 py-3 text-left text-ink transition-colors
              {kind === k ? 'border-brand bg-brand-softer' : 'border-line bg-surface hover:border-brand/50'}"
          >
            <span class="text-sm font-semibold">{title}</span>
            <span class="text-[13px] text-muted">{what}</span>
          </button>
        {/each}
      </div>
      <div class="flex flex-wrap items-center gap-4 rounded-2xl border-2 border-dashed border-brand/40 bg-brand-softer p-5">
        <span class="text-brand"><Icon name="folder" size={36} stroke={1.6} /></span>
        <span class="flex min-w-0 flex-[1_1_300px] flex-col gap-1">
          {#if input}
            <span class="truncate text-[15px] font-semibold">{input}</span>
            <span class="text-[13px] text-muted">
              {found ? `${found.samples.length} sample${found.samples.length === 1 ? '' : 's'} found` : 'Looking…'}
            </span>
          {:else}
            <span class="text-[15px] font-semibold">No folder chosen yet</span>
            <span class="text-[13px] text-muted">Click Choose folder, or paste the folder's path below.</span>
          {/if}
        </span>
        <Button variant={input ? 'secondary' : 'primary'} icon="folder" onclick={() => (picking = true)}>
          {input ? 'Choose another folder' : 'Choose folder'}
        </Button>
      </div>
      <label class="flex flex-col gap-1.5 text-[13px] font-semibold text-ink-2">
        Folder path
        <input class="input" bind:value={input} onchange={pair} placeholder={fasta ? 'C:\\Lab\\Assemblies' : 'C:\\Lab\\Reads\\2026-11-10'} />
      </label>

      {#if found}
        <div class="flex flex-wrap items-center gap-2.5">
          {#if found.samples.length && !problems.length}
            <span class="inline-flex h-7 items-center rounded-full bg-ok-soft px-3 text-[13px] font-semibold text-ok-ink">
              {found.samples.length} sample{found.samples.length === 1 ? '' : 's'} {fasta ? 'found' : 'paired'}
            </span>
          {/if}
          {#if fasta}
            {#if !found.samples.length}
              <span class="text-[13px] text-muted">No FASTA files here (.fasta, .fa, .fna, .fas or .ffn, also with .gz).</span>
            {/if}
          {:else}
            <span class="text-[13px] text-muted">Paired on <strong>{found.r1}</strong> / <strong>{found.r2}</strong>{custom ? ' (your choice)' : ' (found automatically)'}</span>
            <Button variant="quiet" size="sm" class="ml-auto" onclick={() => (custom = !custom)}>{custom ? 'Use automatic pairing' : 'Change pairing'}</Button>
          {/if}
        </div>
        {#if custom && !fasta}
          <div class="flex flex-wrap items-end gap-3 rounded-xl bg-surface-2 p-4">
            <label class="flex flex-col gap-1.5 text-[13px] font-semibold">First read marker <input class="input w-32" bind:value={r1} /></label>
            <label class="flex flex-col gap-1.5 text-[13px] font-semibold">Second read marker <input class="input w-32" bind:value={r2} /></label>
            <Button onclick={pair}>Pair again</Button>
            <span class="basis-full text-xs text-muted">The sample name is everything before the marker, e.g. <code>S12_R1_001.fastq.gz</code> → S12.</span>
          </div>
        {/if}
        {#each problems as p}
          <p class="m-0 flex items-start gap-2 rounded-xl bg-danger-soft px-4 py-3 text-sm text-danger"><Icon name="alert" /> {p}</p>
        {/each}
        {#if found.samples.length}
          <div class="overflow-x-auto rounded-xl border border-line">
            <table class="data min-w-[600px]">
              <thead>
                {#if fasta}<tr><th>Sample</th><th>FASTA file</th></tr>{:else}<tr><th>Sample</th><th>First read (R1)</th><th>Second read (R2)</th></tr>{/if}
              </thead>
              <tbody>
                {#each found.samples as s}
                  <tr>
                    <td class="font-semibold">{s.sample}</td>
                    <td class="text-ink-2">{base(s.r1)}</td>
                    {#if !fasta}<td class={s.r2 ? 'text-ink-2' : 'font-semibold text-danger'}>{s.r2 ? base(s.r2) : 'missing'}</td>{/if}
                  </tr>
                {/each}
              </tbody>
            </table>
          </div>
        {/if}
        {#if found.unpaired.length}
          <p class="m-0 text-[13px] text-muted">Not used (no pair): {found.unpaired.join(', ')}</p>
        {/if}
      {/if}
    {:else if step === 2}
      <div>
        <h2 class="m-0 mb-1 text-xl font-semibold">Which workflow?</h2>
        <p class="m-0 text-sm text-muted">
          You can switch single steps off in the next step. Workflows that start from {fasta ? 'paired reads' : 'FASTA files'} can't run on these files.
        </p>
      </div>
      <div class="grid grid-cols-[repeat(auto-fit,minmax(260px,1fr))] gap-3.5" role="radiogroup" aria-label="Workflow">
        {#each workflows as w (w.id)}
          {@const sel = w.id === wfId}
          {@const ok = fits(w)}
          <button
            role="radio"
            aria-checked={sel}
            disabled={!ok}
            onclick={() => { wfId = w.id; wfVersion = null; }}
            class="flex flex-col items-start gap-2 rounded-2xl border-2 p-4.5 text-left text-ink transition-colors
              {!ok ? 'cursor-not-allowed border-line bg-surface-2 opacity-60' : sel ? 'cursor-pointer border-brand bg-brand-softer' : 'cursor-pointer border-line bg-surface hover:border-brand/50'}"
          >
            <span class="flex flex-wrap items-center gap-2">
              <span class="text-[15px] font-semibold">{w.name}</span>
              <Pill status={w.builtin ? 'builtin' : 'yours'} />
            </span>
            <span class="line-clamp-3 text-[13px] leading-snug text-muted">{w.description || 'No description.'}</span>
            {#if !w.builtin}<span class="text-xs font-semibold text-ink-2">Version {w.latest}</span>{/if}
            {#if !ok}<span class="text-xs font-semibold text-warn-ink">Starts from {w.input_type === 'assemblies' ? 'FASTA files (assemblies)' : 'paired reads'}</span>{/if}
          </button>
        {/each}
      </div>
      {#if chosen && !chosen.builtin && chosen.versions.length > 1}
        <label class="flex items-center gap-3 text-sm font-semibold">
          Version
          <select class="input w-40" bind:value={wfVersion}>
            <option value={null}>Newest ({chosen.latest})</option>
            {#each [...chosen.versions].reverse() as v}<option value={v}>Version {v}</option>{/each}
          </select>
        </label>
      {/if}
    {:else if step === 3 && view}
      <div>
        <h2 class="m-0 mb-1 text-xl font-semibold">Check before starting</h2>
        <p class="m-0 text-sm text-muted">Switch off what you don't need. Steps that depend on it are skipped too. This only changes this run.</p>
      </div>
      {#each warnings as w}
        <div
          class="flex items-start gap-3 rounded-xl border px-4 py-3 text-sm
            {w.level === 'error' ? 'border-danger-line bg-danger-soft text-danger' : w.level === 'warning' ? 'border-warn-line bg-warn-soft text-warn-ink' : 'border-line bg-surface-2 text-ink-2'}"
        >
          <span class="mt-px"><Icon name={w.level === 'info' ? 'info' : 'triangle'} /></span>
          <span class="flex-1">{w.message}</span>
          {#if w.message.includes('Databases page')}<a href={href('databases')} class="font-semibold text-current">Databases</a>{/if}
        </div>
      {/each}
      <div class="grid grid-cols-[repeat(auto-fit,minmax(280px,1fr))] gap-2">
        {#each view.order as id (id)}
          {@const st = view.workflow.steps[id]}
          {@const on = !off[id]}
          {@const skipped = on && !runnable[id]}
          <div class="flex items-center gap-3 rounded-[10px] border border-line px-3.5 py-2.5">
            <span class="flex min-w-0 flex-1 flex-col">
              <span class="truncate text-sm font-semibold {on && !skipped ? 'text-ink' : 'text-muted'}">{st.label ?? id}</span>
              <span class="truncate text-xs text-muted">{skipped ? 'Skipped: a step it needs is off' : st.tool}</span>
            </span>
            <Toggle checked={on} label={st.label ?? id} onchange={(v) => { off[id] = !v; check(); }} />
          </div>
        {/each}
      </div>
    {:else if step === 4 && view}
      <div>
        <h2 class="m-0 mb-1 text-xl font-semibold">Ready to start</h2>
        <p class="m-0 text-sm text-muted">You can close the app while it runs: the run keeps going.</p>
      </div>
      <label class="flex flex-col gap-1.5 text-sm font-semibold">
        Run name
        <input class="input h-11 text-[15px]" bind:value={name} placeholder="For example Week 45 controls" />
      </label>
      <div class="grid grid-cols-[repeat(auto-fit,minmax(200px,1fr))] gap-3 text-sm">
        <div class="rounded-xl border border-line px-4 py-3.5"><span class="block text-xs text-muted">{fasta ? 'FASTA files' : 'Reads'}</span><strong>{found?.samples.length} samples</strong></div>
        <div class="rounded-xl border border-line px-4 py-3.5"><span class="block text-xs text-muted">Workflow</span><strong>{view.workflow.name}</strong></div>
        <div class="rounded-xl border border-line px-4 py-3.5"><span class="block text-xs text-muted">Steps</span><strong>{stepsOn} of {view.order.length} on</strong></div>
      </div>
      <div class="flex items-start gap-3 rounded-xl bg-surface-2 px-4 py-3.5 text-sm">
        <span class="mt-px text-brand"><Icon name="folder" /></span>
        <span class="flex min-w-0 flex-col gap-0.5">
          <span class="font-semibold">Everything for this run goes in one folder</span>
          <span class="break-all text-[13px] text-muted">{live.hardware?.runs_folder ?? 'Documents/Genomics Workflow Microbials'}/{folderPreview}</span>
        </span>
      </div>
      <label class="flex items-center gap-3 text-sm">
        <Toggle bind:checked={keepWork} label="Keep working files" small />
        Keep the big working files after a successful run (only needed for troubleshooting)
      </label>
    {/if}

    <div class="flex flex-wrap items-center gap-2.5 border-t border-line pt-4.5">
      {#if step > 1}<Button onclick={() => (step -= 1)} icon="back">Back</Button>{/if}
      <a href={href('home')} class="px-2 text-sm font-medium text-muted no-underline hover:text-ink">Cancel</a>
      {#if step < 4}
        <Button variant="primary" class="ml-auto" disabled={!canNext} busy={checking && step === 3} onclick={next}>Continue</Button>
      {:else}
        <Button variant="primary" size="lg" icon="play" class="ml-auto" busy={starting} onclick={start}>Start run</Button>
      {/if}
    </div>
  </section>
</div>

<FolderPicker bind:open={picking} start={input} title={fasta ? 'Choose the folder of FASTA files' : 'Choose the reads folder'} onpick={(p) => { input = p; pair(); }} />
