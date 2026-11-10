<script lang="ts">
  import PageHeader from '../lib/PageHeader.svelte';
  import Button from '../lib/ui/Button.svelte';
  import Dialog from '../lib/ui/Dialog.svelte';
  import Pill from '../lib/ui/Pill.svelte';
  import { del, type RunSummary } from '../lib/api';
  import { openFolder } from '../lib/desktop';
  import { duration, live, when } from '../lib/live.svelte';
  import { href } from '../lib/router.svelte';
  import { errorText, toasts } from '../lib/toast.svelte';

  let filter = $state('');
  let confirm = $state<RunSummary | null>(null);
  let confirmOpen = $state(false);
  let deleting = $state(false);

  const shown = $derived(
    live.runs.filter((r) => !filter || `${r.name} ${r.workflow?.name} ${r.id}`.toLowerCase().includes(filter.toLowerCase())),
  );

  async function remove() {
    if (!confirm) return;
    deleting = true;
    try {
      await del(`/api/runs/${encodeURIComponent(confirm.id)}`);
      toasts.show(`Deleted "${confirm.name}"`);
      await live.refreshRuns();
      confirmOpen = false;
    } catch (e) {
      toasts.show(errorText(e), 'error');
    } finally {
      deleting = false;
    }
  }
</script>

<PageHeader title="Runs">
  {#snippet actions()}
    {#if live.hardware?.runs_folder}
      <Button icon="folder" onclick={() => openFolder(live.hardware!.runs_folder)}>Open runs folder</Button>
    {/if}
    <Button variant="primary" icon="plus" href={href('new')}>New run</Button>
  {/snippet}
</PageHeader>

<div class="flex w-full max-w-[1180px] flex-col gap-5 px-8 pt-7 pb-10 max-[720px]:px-4">
  <label class="flex h-11 max-w-[420px] items-center gap-2 rounded-[10px] border border-line-strong bg-surface px-3">
    <span class="sr-only">Find a run</span>
    <input class="min-w-0 flex-1 border-0 bg-transparent text-sm text-ink outline-none" placeholder="Find a run by name or workflow" bind:value={filter} />
  </label>

  <section class="card overflow-x-auto">
    {#if shown.length}
      <table class="data min-w-[760px]">
        <thead><tr><th>Run</th><th>Workflow</th><th>Samples</th><th>Status</th><th>Started</th><th>Took</th><th></th></tr></thead>
        <tbody>
          {#each shown as r (r.id)}
            <tr class="hover:bg-surface-2">
              <td><a href={href(`runs/${encodeURIComponent(r.id)}`)} class="font-semibold text-ink no-underline hover:text-brand">{r.name}</a></td>
              <td class="text-ink-2">{r.workflow?.name}</td>
              <td class="text-ink-2">{r.n_samples}</td>
              <td><Pill status={r.status} /></td>
              <td class="whitespace-nowrap text-muted">{when(r.started)}</td>
              <td class="whitespace-nowrap text-muted">{r.finished ? duration(r.started, r.finished) : ''}</td>
              <td class="whitespace-nowrap text-right">
                <Button size="sm" variant="quiet" href={href(`runs/${encodeURIComponent(r.id)}`)}>Open</Button>
                <Button
                  size="sm"
                  variant="quiet"
                  icon="trash"
                  label="Delete {r.name}"
                  class="!text-danger"
                  disabled={r.status === 'running' || r.status === 'queued'}
                  onclick={() => { confirm = r; confirmOpen = true; }}
                />
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    {:else}
      <p class="m-0 p-6 text-sm text-muted">
        {filter ? 'No run matches.' : 'No runs yet.'}
        <a href={href('new')} class="font-semibold text-brand">Start a run</a>
      </p>
    {/if}
  </section>
</div>

<Dialog bind:open={confirmOpen} title={`Delete the run "${confirm?.name ?? ''}"?`}>
  Its folder goes: results, run details and working files. Your read files are not touched. This can't be undone.
  {#snippet actions()}
    <Button onclick={() => (confirmOpen = false)}>Keep it</Button>
    <Button variant="danger-solid" busy={deleting} onclick={remove}>Delete run</Button>
  {/snippet}
</Dialog>
