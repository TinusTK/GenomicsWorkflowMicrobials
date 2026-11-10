<script lang="ts">
  import PageHeader from '../lib/PageHeader.svelte';
  import Button from '../lib/ui/Button.svelte';
  import Dialog from '../lib/ui/Dialog.svelte';
  import Pill from '../lib/ui/Pill.svelte';
  import { del, get, type WorkflowSummary } from '../lib/api';
  import { href } from '../lib/router.svelte';
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
</script>

<PageHeader title="Workflows">
  {#snippet actions()}
    <Button variant="primary" icon="plus" href={href('workflows/new')}>New workflow</Button>
  {/snippet}
</PageHeader>

<div class="flex w-full max-w-[1180px] flex-col gap-5 px-8 pt-7 pb-10 max-[720px]:px-4">
  <p class="m-0 max-w-[720px] text-sm text-muted">
    A workflow is a set of steps (tools) joined together. Built-in workflows are read-only: open one and choose
    <strong>Save as new workflow</strong> to make your own copy. Every save of your own workflows keeps the older versions.
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
