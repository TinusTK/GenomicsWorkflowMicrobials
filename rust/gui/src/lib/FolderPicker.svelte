<script lang="ts">
  // Pick a folder of reads: walk folders, see how many read files each has.
  import Button from './ui/Button.svelte';
  import Dialog from './ui/Dialog.svelte';
  import Icon from './ui/Icon.svelte';
  import { get } from './api';
  import { errorText } from './toast.svelte';

  let { open = $bindable(false), start = '', onpick }: { open?: boolean; start?: string; onpick: (path: string) => void } = $props();

  interface Listing {
    path: string;
    parent: string | null;
    dirs: string[];
    reads: string[];
  }
  let listing = $state<Listing | null>(null);
  let error = $state('');
  let typed = $state('');

  async function load(path: string) {
    try {
      listing = await get<Listing>(`/api/browse${path ? `?path=${encodeURIComponent(path)}` : ''}`);
      typed = listing.path;
      error = '';
    } catch (e) {
      error = errorText(e);
    }
  }
  $effect(() => {
    if (open) load(start);
  });
  const join = (a: string, b: string) => (a.endsWith('/') ? a + b : `${a}/${b}`);
</script>

<Dialog bind:open title="Choose the reads folder" wide>
  <div class="flex flex-col gap-3">
    <form class="flex gap-2" onsubmit={(e) => { e.preventDefault(); load(typed); }}>
      <label class="sr-only" for="fp-path">Folder</label>
      <input id="fp-path" class="input flex-1" bind:value={typed} placeholder="For example C:\Lab\Reads or /home/lab/reads" />
      <Button type="submit">Go</Button>
    </form>
    {#if error}<p class="m-0 text-sm text-danger">{error}</p>{/if}
    {#if listing}
      <div class="max-h-[380px] overflow-y-auto rounded-xl border border-line">
        {#if listing.parent}
          <button class="flex w-full cursor-pointer items-center gap-3 border-0 border-b border-line bg-transparent px-4 py-2.5 text-left text-sm text-ink hover:bg-hover" onclick={() => load(listing!.parent!)}>
            <Icon name="back" /> Up one folder
          </button>
        {/if}
        {#each listing.dirs as d}
          <button class="flex w-full cursor-pointer items-center gap-3 border-0 border-b border-line bg-transparent px-4 py-2.5 text-left text-sm text-ink hover:bg-hover" onclick={() => load(join(listing!.path, d))}>
            <span class="text-brand"><Icon name="folder" /></span>
            <span class="flex-1">{d}</span>
            <Icon name="chevron" size={16} />
          </button>
        {/each}
        {#if listing.reads.length}
          <div class="flex flex-col gap-1 px-4 py-3 text-[13px] text-muted">
            <span class="font-semibold text-ink-2">{listing.reads.length} read file{listing.reads.length === 1 ? '' : 's'} here</span>
            {#each listing.reads.slice(0, 6) as f}<span>{f}</span>{/each}
            {#if listing.reads.length > 6}<span>…and {listing.reads.length - 6} more</span>{/if}
          </div>
        {:else if !listing.dirs.length}
          <p class="m-0 px-4 py-3 text-sm text-muted">This folder is empty.</p>
        {/if}
      </div>
    {/if}
  </div>
  {#snippet actions()}
    <Button onclick={() => (open = false)}>Cancel</Button>
    <Button variant="primary" disabled={!listing?.reads.length} onclick={() => { onpick(listing!.path); open = false; }}>
      Use this folder
    </Button>
  {/snippet}
</Dialog>
