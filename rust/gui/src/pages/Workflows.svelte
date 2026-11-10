<script lang="ts">
  import PageHeader from '../lib/PageHeader.svelte';
  import Button from '../lib/ui/Button.svelte';
  import Dialog from '../lib/ui/Dialog.svelte';
  import Pill from '../lib/ui/Pill.svelte';
  import { del, get, post, type ImportResult, type ImportStep, type WorkflowSummary } from '../lib/api';
  import { go, href } from '../lib/router.svelte';
  import { errorText, toasts } from '../lib/toast.svelte';

  let list = $state<WorkflowSummary[]>([]);
  let confirm = $state<WorkflowSummary | null>(null);
  let confirmOpen = $state(false);
  let confirm2 = $state(false);

  async function load() {
    try {
      list = await get<WorkflowSummary[]>('/api/workflows');
    } catch (e) {
      toasts.show(errorText(e), 'error');
    }
  }
  load();

  async function remove() {
    if (!confirm) return;
    try {
      await del(`/api/workflows/${encodeURIComponent(confirm.id)}`);
      toasts.show(`Deleted the workflow "${confirm.name}"`);
      confirmOpen = false;
      confirm2 = false;
      load();
    } catch (e) {
      toasts.show(errorText(e), 'error');
    }
  }

  // ---------------------------------------------------------------- import from Galaxy

  let picker: HTMLInputElement | undefined = $state();
  let ga = $state('');
  let preview = $state<ImportResult | null>(null);
  let importOpen = $state(false);
  let importing = $state(false);

  /** What happens to a Galaxy step, in plain words. */
  const ACTIONS: Record<ImportStep['action'], [string, string]> = {
    builtin: ['In the app', 'bg-ok-soft text-ok-ink'],
    new_version: ['New version', 'bg-info-soft text-info-ink'],
    new_tool: ['New tool', 'bg-[#ede9fe] text-[#5b21b6]'],
    summary: ['Summary tables', 'bg-surface-2 text-ink-2 border border-line'],
    input: ['Your reads', 'bg-surface-2 text-ink-2 border border-line'],
    left_out: ['Left out', 'bg-warn-soft text-warn-ink'],
  };
  const count = (a: ImportStep['action']) => preview?.steps.filter((s) => s.action === a).length ?? 0;
  // tools first, then what is added, then the text steps
  const ORDER: ImportStep['action'][] = ['new_tool', 'new_version', 'left_out', 'input', 'builtin', 'summary'];
  const ordered = $derived(
    [...(preview?.steps ?? [])].sort((a, b) => ORDER.indexOf(a.action) - ORDER.indexOf(b.action) || a.galaxy_step - b.galaxy_step),
  );

  async function picked() {
    const file = picker?.files?.[0];
    if (!file) return;
    try {
      ga = await file.text();
      preview = await post<ImportResult>('/api/workflows/import', { ga });
      importOpen = true;
    } catch (e) {
      toasts.show(errorText(e), 'error');
    } finally {
      if (picker) picker.value = ''; // picking the same file again works
    }
  }

  async function runImport() {
    importing = true;
    try {
      const done = await post<ImportResult>('/api/workflows/import', { ga, apply: true });
      const changed = done.steps.filter((s) => s.note.includes('could not be added')).length;
      toasts.show(
        changed
          ? `Imported "${done.name}". ${changed} tool version(s) could not be added: the app's own version is used.`
          : `Imported "${done.name}". Check it over, then run it.`,
        changed ? 'info' : undefined,
      );
      importOpen = false;
      go(`workflows/${done.id}`);
    } catch (e) {
      toasts.show(errorText(e), 'error');
    } finally {
      importing = false;
    }
  }
</script>

<PageHeader title="Workflows">
  {#snippet actions()}
    <Button icon="up" onclick={() => picker?.click()}>Import from Galaxy</Button>
    <Button variant="primary" icon="plus" href={href('workflows/new')}>New workflow</Button>
  {/snippet}
</PageHeader>
<input bind:this={picker} type="file" accept=".ga,.json,application/json" class="hidden" onchange={picked} aria-label="Galaxy workflow file" />

<div class="flex w-full max-w-[1180px] flex-col gap-5 px-8 pt-7 pb-10 max-[720px]:px-4">
  <p class="m-0 max-w-[720px] text-sm text-muted">
    A workflow is a set of steps (tools) joined together. Built-in workflows are read-only: open one and choose
    <strong>Save as new workflow</strong> to make your own copy. Every save of your own workflows keeps the older versions.
    <strong>Import from Galaxy</strong> turns a workflow file saved from Galaxy (.ga) into a workflow here, with the same tools,
    versions and settings.
  </p>
  <div class="grid grid-cols-[repeat(auto-fill,minmax(320px,1fr))] gap-4">
    {#each list as w (w.id)}
      <article class="card flex flex-col gap-3 p-5">
        <div class="flex flex-wrap items-center gap-2">
          <h2 class="m-0 text-base font-semibold">{w.name}</h2>
          <Pill status={w.builtin ? 'builtin' : 'yours'} />
        </div>
        <p class="m-0 line-clamp-3 text-[13px] leading-snug text-muted">{w.description || 'No description.'}</p>
        <span class="text-xs text-muted">{w.builtin ? 'Built-in' : `Version ${w.latest}${w.versions.length > 1 ? ` (${w.versions.length} saved)` : ''}`}</span>
        <div class="mt-auto flex flex-wrap gap-2">
          <Button size="sm" variant="primary" icon="play" href={href(`new?workflow=${w.id}`)}>Run</Button>
          <Button size="sm" icon="flow" href={href(`workflows/${w.id}`)}>Open</Button>
          <Button size="sm" variant="quiet" icon="copy" href={href(`workflows/new?from=${w.id}`)}>Copy</Button>
          {#if !w.builtin}
            <Button size="sm" variant="quiet" icon="trash" label="Delete {w.name}" class="ml-auto !text-danger" onclick={() => { confirm = w; confirmOpen = true; }} />
          {/if}
        </div>
      </article>
    {/each}
    <a href={href('workflows/new')} class="card flex min-h-[180px] flex-col items-center justify-center gap-2 border-dashed p-5 text-brand no-underline hover:bg-brand-softer">
      <span class="text-3xl leading-none">+</span>
      <span class="text-sm font-semibold">Build a new workflow</span>
    </a>
  </div>
</div>

<Dialog bind:open={confirmOpen} title={`Delete the workflow "${confirm?.name ?? ''}"?`}>
  {#if !confirm2}
    All {confirm?.versions.length} saved version(s) go. Runs that already used it keep their own copy.
  {:else}
    <strong>Are you sure?</strong> This can't be undone.
  {/if}
  {#snippet actions()}
    <Button onclick={() => { confirmOpen = false; confirm2 = false; }}>Keep it</Button>
    {#if !confirm2}
      <Button variant="danger" onclick={() => (confirm2 = true)}>Delete…</Button>
    {:else}
      <Button variant="danger-solid" onclick={remove}>Yes, delete it</Button>
    {/if}
  {/snippet}
</Dialog>

<Dialog bind:open={importOpen} wide title={`Import "${preview?.name ?? ''}"?`}>
  {#if preview}
    <p class="m-0 mb-3">
      This Galaxy workflow has {preview.steps.length} steps. Nothing is added until you press <strong>Import</strong>; it then
      opens in the workflow builder so you can check it.
    </p>
    <ul class="m-0 mb-3 flex list-none flex-wrap gap-x-5 gap-y-1 p-0 text-[13px]">
      <li><strong>{count('builtin')}</strong> tools already in the app</li>
      {#if count('new_version')}<li><strong>{count('new_version')}</strong> at a version to add</li>{/if}
      {#if count('new_tool')}<li><strong>{count('new_tool')}</strong> new tools (need their command written)</li>{/if}
      {#if count('summary')}<li><strong>{count('summary')}</strong> text steps covered by the summary tables</li>{/if}
      {#if count('left_out')}<li><strong>{count('left_out')}</strong> left out</li>{/if}
    </ul>
    <div class="max-h-[46vh] overflow-auto rounded-xl border border-line">
      <table class="data">
        <thead class="sticky top-0"><tr><th>Galaxy step</th><th>Becomes</th><th>Note</th></tr></thead>
        <tbody>
          {#each ordered as s (s.galaxy_step)}
            <tr>
              <td>
                <span class="font-semibold text-ink">{s.name}</span>
                <span class="block text-xs text-muted">step {s.galaxy_step}{s.galaxy_version ? ` · ${s.galaxy_version}` : ''}</span>
              </td>
              <td class="whitespace-nowrap">
                <span class="inline-flex h-6 items-center rounded-full px-2.5 text-xs font-bold {ACTIONS[s.action][1]}">{ACTIONS[s.action][0]}</span>
              </td>
              <td class="text-[13px]">{s.note}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {/if}
  {#snippet actions()}
    <Button onclick={() => (importOpen = false)}>Cancel</Button>
    <Button variant="primary" busy={importing} onclick={runImport}>Import</Button>
  {/snippet}
</Dialog>
