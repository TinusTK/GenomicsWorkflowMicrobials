<script lang="ts">
  import PageHeader from '../lib/PageHeader.svelte';
  import Icon from '../lib/ui/Icon.svelte';
  import Pill from '../lib/ui/Pill.svelte';
  import Toggle from '../lib/ui/Toggle.svelte';
  import { get, post, type Tool } from '../lib/api';
  import { errorText, toasts } from '../lib/toast.svelte';

  /** the folder under the data folder's tools/ that Galaxy imports go in */
  const IMPORTED = 'galaxy_import';

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
  const packs = $derived([...new Set(tools.filter((t) => t.source !== 'built-in' && t.source !== IMPORTED).map((t) => t.source))]);
  const imported = $derived(tools.filter((t) => t.source === IMPORTED).length);

  /** Show or hide one of a tool's outputs on the Run page. */
  async function showOutput(t: Tool, output: string, show: boolean) {
    const hidden = show ? t.hidden_outputs.filter((o) => o !== output) : [...t.hidden_outputs, output];
    try {
      const r = await post<{ hidden_outputs: string[] }>(`/api/tools/${encodeURIComponent(t.id)}/outputs`, { hidden });
      t.hidden_outputs = r.hidden_outputs;
    } catch (e) {
      toasts.show(errorText(e), 'error');
    }
  }
</script>

<PageHeader title="Tools and packs" />

<div class="flex w-full max-w-[1180px] flex-col gap-5 px-8 pt-7 pb-10 max-[720px]:px-4">
  <div class="flex flex-wrap items-center gap-3">
    <label class="flex h-11 min-w-[280px] flex-1 items-center gap-2 rounded-[10px] border border-line-strong bg-surface px-3 sm:max-w-[420px]">
      <span class="text-muted"><Icon name="search" size={16} /></span>
      <span class="sr-only">Search tools</span>
      <input class="min-w-0 flex-1 border-0 bg-transparent text-sm text-ink outline-none" placeholder="Search tools" bind:value={search} />
    </label>
    <span class="text-sm text-muted"
      >{tools.length} tools · {packs.length} tool pack{packs.length === 1 ? '' : 's'} installed{packs.length ? ` (${packs.join(', ')})` : ''}{imported
        ? ` · ${imported} imported from Galaxy`
        : ''}</span
    >
  </div>
  <p class="m-0 rounded-xl border border-line bg-surface-2 px-4 py-3 text-[13px] text-ink-2">
    Every tool runs in its own container at a fixed version, so results are repeatable. Open a tool to choose which of its
    results show on a run's page (all of them to begin with). Importing a Galaxy workflow (Workflows page) adds the tool
    versions and tools it needs here.
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
              {#if t.variants.length}<span class="text-xs text-muted">+{t.variants.length} other version{t.variants.length === 1 ? '' : 's'} ({t.variants.join(', ')})</span>{/if}
              {#if t.draft}<Pill status="attention" text="Needs its command" />{/if}
              {#if t.source === 'built-in'}
                <Pill status="builtin" text="Built-in" />
              {:else if t.source === IMPORTED}
                <Pill status="yours" text="Imported from Galaxy" />
              {:else}
                <Pill status="yours" text={`Pack: ${t.source}`} />
              {/if}
              <Icon name={open === t.id ? 'down' : 'chevron'} size={16} />
            </button>
            {#if open === t.id}
              <div class="grid grid-cols-[repeat(auto-fit,minmax(260px,1fr))] gap-3 px-4.5 pb-4 text-[13px] text-ink-2">
                {#if t.draft}
                  <div class="col-span-full rounded-lg bg-warn-soft px-3 py-2 text-warn-ink">
                    This tool came from a Galaxy workflow. Its container, inputs, outputs and settings are set up, but a Galaxy
                    workflow file has no command lines, so its command still has to be written before it can run:
                    <span class="font-mono break-all">{t.folder}/tool.yml</span>
                  </div>
                {/if}
                <div><span class="label-caps block">Container</span><span class="break-all">{t.container || 'none found'}</span></div>
                <div><span class="label-caps block">Takes</span>{Object.keys(t.inputs).join(', ') || '-'}</div>
                {#if t.databases.length}<div><span class="label-caps block">Needs database</span>{t.databases.join(', ')}</div>{/if}
                {#if t.license}<div><span class="label-caps block">Licence</span>{t.license}</div>{/if}
                {#if t.homepage}<div><span class="label-caps block">Website</span><a href={t.homepage} target="_blank" rel="noreferrer" class="text-brand">{t.homepage}</a></div>{/if}
                {#if t.citation}<div class="col-span-full"><span class="label-caps block">Cite</span>{t.citation}</div>{/if}
                <div class="col-span-full">
                  <span class="label-caps block">Makes · shown on a run's page</span>
                  {#if Object.keys(t.outputs).length}
                    <ul class="m-0 mt-1.5 flex list-none flex-col gap-2 p-0">
                      {#each Object.entries(t.outputs) as [name, o] (name)}
                        <li class="flex items-center gap-3">
                          <Toggle
                            small
                            label={`Show ${name} of ${t.name} on a run's page`}
                            checked={!t.hidden_outputs.includes(name)}
                            onchange={(v) => showOutput(t, name, v)}
                          />
                          <span><span class="font-semibold text-ink">{name}</span>{o.description ? ` · ${o.description}` : ''}</span>
                        </li>
                      {/each}
                    </ul>
                  {:else}-{/if}
                </div>
              </div>
            {/if}
          </div>
        {/each}
      </div>
    </section>
  {/each}
</div>
