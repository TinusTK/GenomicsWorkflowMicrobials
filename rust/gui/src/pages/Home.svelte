<script lang="ts">
  import PageHeader from '../lib/PageHeader.svelte';
  import Button from '../lib/ui/Button.svelte';
  import Icon from '../lib/ui/Icon.svelte';
  import Pill from '../lib/ui/Pill.svelte';
  import { get, type WorkflowSummary } from '../lib/api';
  import { live, when } from '../lib/live.svelte';
  import { go, href } from '../lib/router.svelte';

  let workflows = $state<WorkflowSummary[]>([]);
  get<WorkflowSummary[]>('/api/workflows').then((w) => (workflows = w)).catch(() => {});

  const attention = $derived.by(() => {
    const out: { icon: string; tone: string; title: string; text: string; action: string; to: string }[] = [];
    const hw = live.hardware;
    if (hw && !hw.docker && !hw.apptainer)
      out.push({ icon: 'alert', tone: 'bg-danger-soft text-danger', title: 'The tools can\'t start yet', text: hw.docker_error ?? 'No container engine found.', action: 'Set up', to: 'computer' });
    for (const d of live.missingDb)
      out.push({ icon: 'alert', tone: 'bg-warn-soft text-warn', title: `${d.name} is not installed`, text: `Steps that need it are switched off until it is there (${d.size_gb} GB).`, action: 'Install', to: 'databases' });
    for (const r of live.runs.filter((r) => r.status === 'failed' || r.status === 'interrupted').slice(0, 2))
      out.push({ icon: 'triangle', tone: 'bg-danger-soft text-danger', title: `Run "${r.name}" ${r.status === 'failed' ? 'failed' : 'was interrupted'}`, text: 'Open it to see why, then resume it: finished steps are kept.', action: 'Open', to: `runs/${r.id}` });
    return out;
  });
</script>

<PageHeader title="Home" />

<div class="flex w-full max-w-[1160px] flex-col gap-6 px-8 pt-7 pb-10 max-[720px]:px-4">
  <section class="flex flex-wrap items-center gap-6 rounded-2xl bg-brand px-8 py-7 text-on-brand">
    <div class="flex flex-[1_1_420px] flex-col gap-2">
      <h2 class="m-0 font-display text-[28px] font-semibold">Type your isolates</h2>
      <p class="m-0 text-[15px] leading-relaxed opacity-90">
        Pick a folder of paired reads and a workflow. The app does the assembly, MLST, serovar, emm type, AMR and quality
        checks, and every result records the exact tool versions.
      </p>
    </div>
    <div class="flex flex-wrap gap-2.5">
      <Button variant="on-brand" size="lg" icon="plus" href={href('new')}>New run</Button>
      <a href={href('workflows/new')} class="inline-flex h-12 items-center rounded-xl border border-white/50 px-5 text-[15px] font-semibold text-on-brand no-underline hover:bg-white/10">Build a workflow</a>
    </div>
  </section>

  <div class="grid grid-cols-[repeat(auto-fit,minmax(320px,1fr))] gap-6">
    <section class="card flex flex-col gap-3.5 px-5.5 py-5" aria-labelledby="att">
      <h2 id="att" class="m-0 text-base font-semibold">Needs your attention</h2>
      {#each attention as a}
        <div class="flex items-center gap-3.5">
          <span class="flex h-9 w-9 shrink-0 items-center justify-center rounded-[10px] {a.tone}"><Icon name={a.icon} /></span>
          <span class="flex flex-1 flex-col gap-0.5">
            <span class="text-sm font-semibold">{a.title}</span>
            <span class="text-[13px] text-muted">{a.text}</span>
          </span>
          <Button size="sm" onclick={() => go(a.to)}>{a.action}</Button>
        </div>
      {:else}
        <div class="flex items-center gap-3 text-sm text-muted">
          <span class="flex h-9 w-9 items-center justify-center rounded-[10px] bg-ok-soft text-ok"><Icon name="check" /></span>
          All set: nothing needs you right now.
        </div>
      {/each}
    </section>

    <section class="card flex flex-col gap-3 px-5.5 py-5" aria-labelledby="wfs">
      <h2 id="wfs" class="m-0 text-base font-semibold">Your workflows</h2>
      {#each workflows.slice(0, 4) as w (w.id)}
        <a href={href(`new?workflow=${w.id}`)} class="flex items-center gap-3 rounded-[10px] border border-line px-3 py-2.5 text-ink no-underline hover:bg-surface-2">
          <span class="flex flex-1 flex-col gap-0.5">
            <span class="text-sm font-semibold">{w.name}</span>
            <span class="text-[13px] text-muted">{w.builtin ? 'Built-in' : `Yours · version ${w.latest}`}</span>
          </span>
          <span class="text-[13px] font-semibold text-brand">Run</span>
        </a>
      {/each}
      <div><Button variant="quiet" size="sm" icon="plus" href={href('workflows/new')}>New workflow</Button></div>
    </section>
  </div>

  <section class="card flex flex-col gap-3 px-5.5 py-5" aria-labelledby="recent">
    <div class="flex items-center gap-3">
      <h2 id="recent" class="m-0 text-base font-semibold">Recent runs</h2>
      <a href={href('runs')} class="ml-auto text-[13px] font-semibold text-brand">All runs</a>
    </div>
    {#if live.runs.length}
      <div class="overflow-x-auto">
        <table class="data min-w-[640px]">
          <thead><tr><th>Run</th><th>Workflow</th><th>Samples</th><th>Status</th><th>Started</th><th></th></tr></thead>
          <tbody>
            {#each live.runs.slice(0, 6) as r (r.id)}
              <tr class="hover:bg-surface-2">
                <td class="font-semibold">{r.name}</td>
                <td class="text-ink-2">{r.workflow?.name}</td>
                <td class="text-ink-2">{r.n_samples}</td>
                <td><Pill status={r.status} /></td>
                <td class="text-muted">{when(r.started)}</td>
                <td class="text-right"><a href={href(`runs/${r.id}`)} class="text-[13px] font-semibold text-brand">Open</a></td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {:else}
      <p class="m-0 text-sm text-muted">No runs yet. Start one with <a href={href('new')} class="font-semibold text-brand">New run</a>.</p>
    {/if}
  </section>
</div>
