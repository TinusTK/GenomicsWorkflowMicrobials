<script lang="ts">
  import PageHeader from '../lib/PageHeader.svelte';
  import Icon from '../lib/ui/Icon.svelte';
  import Pill from '../lib/ui/Pill.svelte';
  import { get, type Tool } from '../lib/api';
  import { errorText, toasts } from '../lib/toast.svelte';

  let tools = $state<Tool[]>([]);
  let search = $state('');
  let open = $state<string | null>(null);
  get<Tool[]>('/api/tools').then((t) => (tools = t)).catch((e) => toasts.show(errorText(e), 'error'));

  const groups = $derived.by(() => {
    const q = search.trim().toLowerCase();
    const g = new Map<string, Tool[]>();
    for (const t of tools) {
      if (q && !`${t.name} ${t.id} ${t.category} ${t.description} ${t.source}`.toLowerCase().includes(q)) continue;
      const c = t.category ?? 'Other';
      g.set(c, [...(g.get(c) ?? []), t]);
    }
    return [...g.entries()].sort((a, b) => a[0].localeCompare(b[0]));
  });
  const packs = $derived([...new Set(tools.filter((t) => t.source !== 'built-in').map((t) => t.source))]);
</script>

<PageHeader title="Tools and packs" />

<div class="flex w-full max-w-[1180px] flex-col gap-5 px-8 pt-7 pb-10 max-[720px]:px-4">
  <div class="flex flex-wrap items-center gap-3">
    <label class="flex h-11 min-w-[280px] flex-1 items-center gap-2 rounded-[10px] border border-line-strong bg-surface px-3 sm:max-w-[420px]">
      <span class="text-muted"><Icon name="search" size={16} /></span>
      <span class="sr-only">Search tools</span>
      <input class="min-w-0 flex-1 border-0 bg-transparent text-sm text-ink outline-none" placeholder="Search tools" bind:value={search} />
    </label>
    <span class="text-sm text-muted">{tools.length} tools · {packs.length} tool pack{packs.length === 1 ? '' : 's'} installed{packs.length ? ` (${packs.join(', ')})` : ''}</span>
  </div>
  <p class="m-0 rounded-xl border border-line bg-surface-2 px-4 py-3 text-[13px] text-ink-2">
    Every tool runs in its own container at a fixed version, so results are repeatable. Installing new tool packs and
    extra tool versions from this page arrives in the next build; packs installed with version 1 are already here.
  </p>
  {#each groups as [cat, list] (cat)}
    <section class="flex flex-col gap-2">
      <h2 class="label-caps m-0">{cat}</h2>
      <div class="card divide-y divide-line">
        {#each list as t (t.id)}
          <div>
            <button
              class="flex w-full cursor-pointer flex-wrap items-center gap-3 border-0 bg-transparent px-4.5 py-3.5 text-left text-ink hover:bg-surface-2"
              aria-expanded={open === t.id}
              onclick={() => (open = open === t.id ? null : t.id)}
            >
              <span class="flex min-w-0 flex-[1_1_300px] flex-col gap-0.5">
                <span class="text-[15px] font-semibold">{t.name} <span class="font-normal text-muted">{t.version}</span></span>
                <span class="line-clamp-1 text-[13px] text-muted">{t.description ?? ''}</span>
              </span>
              {#if t.variants.length}<span class="text-xs text-muted">+{t.variants.length} other version{t.variants.length === 1 ? '' : 's'}</span>{/if}
              <Pill status={t.source === 'built-in' ? 'builtin' : 'yours'} text={t.source === 'built-in' ? 'Built-in' : `Pack: ${t.source}`} />
              <Icon name={open === t.id ? 'down' : 'chevron'} size={16} />
            </button>
            {#if open === t.id}
              <div class="grid grid-cols-[repeat(auto-fit,minmax(260px,1fr))] gap-3 px-4.5 pb-4 text-[13px] text-ink-2">
                <div><span class="label-caps block">Container</span><span class="break-all">{t.container}</span></div>
                <div><span class="label-caps block">Takes</span>{Object.keys(t.inputs).join(', ') || '-'}</div>
                <div><span class="label-caps block">Makes</span>{Object.keys(t.outputs).join(', ') || '-'}</div>
                {#if t.databases.length}<div><span class="label-caps block">Needs database</span>{t.databases.join(', ')}</div>{/if}
                {#if t.license}<div><span class="label-caps block">Licence</span>{t.license}</div>{/if}
                {#if t.homepage}<div><span class="label-caps block">Website</span><a href={t.homepage} target="_blank" rel="noreferrer" class="text-brand">{t.homepage}</a></div>{/if}
                {#if t.citation}<div class="col-span-full"><span class="label-caps block">Cite</span>{t.citation}</div>{/if}
              </div>
            {/if}
          </div>
        {/each}
      </div>
    </section>
  {/each}
</div>
