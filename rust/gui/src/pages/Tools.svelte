<script lang="ts">
  import PageHeader from '../lib/PageHeader.svelte';
  import Button from '../lib/ui/Button.svelte';
  import Dialog from '../lib/ui/Dialog.svelte';
  import Icon from '../lib/ui/Icon.svelte';
  import Pill from '../lib/ui/Pill.svelte';
  import Toggle from '../lib/ui/Toggle.svelte';
  import { get, post, type Tool } from '../lib/api';
  import { openLink } from '../lib/desktop';
  import { go, href, route } from '../lib/router.svelte';
  import { REPORT_TABLES } from '../lib/results';
  import { errorText, toasts } from '../lib/toast.svelte';

  /** the folders under the data folder's tools/ that imports and added tools go in */
  const IMPORTED = 'galaxy_import';
  const GITHUB = 'github';

  let tools = $state<Tool[]>([]);
  let search = $state('');
  let open = $state<string | null>(null);
  async function load() {
    try {
      tools = await get<Tool[]>('/api/tools');
    } catch (e) {
      toasts.show(errorText(e), 'error');
    }
  }
  load();

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
  const yours = $derived(tools.filter((t) => t.source !== 'built-in').length);
  const sourceText = (t: Tool) =>
    t.source === IMPORTED ? 'Imported from Galaxy' : t.source === GITHUB ? 'Added from GitHub' : `Pack: ${t.source}`;

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

  // ---------------------------------------------------------------- add a tool from GitHub

  // Home's "Add a tool" button opens this page with the dialog already up
  let addOpen = $state(route.current.query.get('add') === '1');
  let link = $state('');
  let adding = $state(false);

  /** the built-in tool the pasted link belongs to: the user is told before anything is added */
  let builtinHit = $state<{ id: string; name: string; version: string; canAdd: boolean } | null>(null);
  $effect(() => {
    // another link, or the dialog closed: the warning no longer applies
    void link;
    if (!addOpen || link !== warnedLink) builtinHit = null;
  });
  let warnedLink = '';

  function useBuiltin() {
    if (!builtinHit) return;
    search = builtinHit.name;
    open = builtinHit.id;
    addOpen = false;
    link = '';
  }

  async function add(anyway = false) {
    adding = true;
    try {
      const done = await post<{
        id: string; name: string; draft: boolean; notes: string[];
        builtin?: { id: string; name: string; version: string }; can_add?: boolean;
      }>('/api/tools/add', { url: link, anyway });
      if (done.builtin) {
        warnedLink = link;
        builtinHit = { ...done.builtin, canAdd: !!done.can_add };
        return;
      }
      toasts.show(`Added ${done.name}. ${done.notes.join(' ')}`, done.draft ? 'info' : 'ok', undefined, 9000);
      addOpen = false;
      link = '';
      if (done.draft) go(`tools/${done.id}`);
      else load();
    } catch (e) {
      toasts.show(errorText(e), 'error');
    } finally {
      adding = false;
    }
  }

  // ---------------------------------------------------------------- other versions

  interface Versions {
    package: string | null;
    current: string;
    available: { version: string; container: string }[];
  }
  let versions = $state<Record<string, Versions>>({});
  let looking = $state<string | null>(null);
  let pick = $state<Record<string, string>>({});
  let addingVersion = $state(false);

  async function lookUp(t: Tool) {
    looking = t.id;
    try {
      const v = await get<Versions>(`/api/tools/${encodeURIComponent(t.id)}/versions`);
      versions[t.id] = v;
      pick[t.id] = newer[t.id] && v.available.some((a) => a.version === newer[t.id]) ? newer[t.id] : (v.available[0]?.version ?? '');
    } catch (e) {
      toasts.show(errorText(e), 'error');
    } finally {
      looking = null;
    }
  }

  async function addVersion(t: Tool) {
    const version = pick[t.id];
    if (!version) return;
    addingVersion = true;
    try {
      await post(`/api/tools/${encodeURIComponent(t.id)}/versions`, { version });
      toasts.show(`Added ${t.name} ${version}. Pick it for a step in the workflow builder (Tool version).`);
      delete versions[t.id];
      delete newer[t.id];
      await load();
    } catch (e) {
      toasts.show(errorText(e), 'error');
    } finally {
      addingVersion = false;
    }
  }

  /** tool id -> its newest version, when that is newer than any here */
  let newer = $state<Record<string, string>>({});
  let checking = $state(false);

  async function checkNewer() {
    checking = true;
    try {
      const found = await get<{ id: string; name: string; current: string; newest: string }[]>('/api/tools/updates');
      newer = Object.fromEntries(found.map((f) => [f.id, f.newest]));
      toasts.show(
        found.length
          ? `${found.length} tool${found.length === 1 ? ' has' : 's have'} a newer version: ${found.map((f) => `${f.name} ${f.newest}`).join(', ')}. Open a tool to add it.`
          : 'Every tool is at its newest version.',
        found.length ? 'info' : 'ok',
        undefined,
        9000,
      );
    } catch (e) {
      toasts.show(errorText(e), 'error');
    } finally {
      checking = false;
    }
  }
</script>

<PageHeader title="Tools">
  {#snippet actions()}
    <Button icon="refresh" busy={checking} onclick={checkNewer}>Check for newer versions</Button>
    <Button variant="primary" icon="plus" onclick={() => (addOpen = true)}>Add a tool</Button>
  {/snippet}
</PageHeader>

<div class="flex w-full max-w-[1180px] flex-col gap-5 px-8 pt-7 pb-10 max-[720px]:px-4">
  <div class="flex flex-wrap items-center gap-3">
    <label class="flex h-11 min-w-[280px] flex-1 items-center gap-2 rounded-[10px] border border-line-strong bg-surface px-3 sm:max-w-[420px]">
      <span class="text-muted"><Icon name="search" size={16} /></span>
      <span class="sr-only">Search tools</span>
      <input class="min-w-0 flex-1 border-0 bg-transparent text-sm text-ink outline-none" placeholder="Search tools" bind:value={search} />
    </label>
    <span class="text-sm text-muted">{tools.length} tools{yours ? ` · ${yours} added by you` : ''}</span>
  </div>
  <p class="m-0 rounded-xl border border-line bg-surface-2 px-4 py-3 text-[13px] text-ink-2">
    Every tool runs in its own container at a fixed version, so results are repeatable. Open a tool to choose which of its
    results show on a run's page, or to add another version of it. <strong>Add a tool</strong> brings in a tool from its GitHub
    page; importing a Galaxy workflow (Workflows page) adds the tools it needs too. Tools you add are kept on this computer only.
  </p>
  {#each groups as [cat, list] (cat)}
    <section class="flex flex-col gap-2">
      <h2 class="label-caps m-0">{cat}</h2>
      <div class="card divide-y divide-line">
        {#each list as t (t.id)}
          {@const v = versions[t.id]}
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
              {#if newer[t.id]}<span class="inline-flex h-6 items-center rounded-full bg-info-soft px-2.5 text-xs font-bold text-info-ink">Newer: {newer[t.id]}</span>{/if}
              {#if t.draft}<Pill status="attention" text="Not ready yet" />{/if}
              {#if t.source === 'built-in'}
                <Pill status="builtin" text="Built-in" />
              {:else}
                <Pill status="yours" text={sourceText(t)} />
              {/if}
              <Icon name={open === t.id ? 'down' : 'chevron'} size={16} />
            </button>
            {#if open === t.id}
              <div class="grid grid-cols-[repeat(auto-fit,minmax(260px,1fr))] gap-3 px-4.5 pb-4 text-[13px] text-ink-2">
                {#if t.source !== 'built-in'}
                  <div class="col-span-full flex flex-wrap items-center gap-3 rounded-lg px-3 py-2 {t.draft ? 'bg-warn-soft text-warn-ink' : 'bg-surface-2'}">
                    <span class="min-w-0 flex-[1_1_320px]">
                      {#if t.draft}
                        This tool is not ready yet: its command still has to be written before a workflow can run it. The editor
                        shows what to type and lets you try it on one sample.
                      {:else}
                        This tool was added on this computer. You can change its command, inputs, outputs and settings.
                      {/if}
                    </span>
                    <Button size="sm" variant={t.draft ? 'primary' : 'secondary'} icon="edit" href={href(`tools/${t.id}`)}>{t.draft ? 'Finish this tool' : 'Edit'}</Button>
                  </div>
                {/if}
                <div><span class="label-caps block">Container</span><span class="break-all">{t.container || 'none yet'}</span></div>
                <div><span class="label-caps block">Takes</span>{Object.keys(t.inputs).join(', ') || '-'}</div>
                {#if t.databases.length}<div><span class="label-caps block">Needs database</span>{t.databases.join(', ')}</div>{/if}
                {#if t.license}<div><span class="label-caps block">Licence</span>{t.license}</div>{/if}
                {#if t.homepage}
                  <div>
                    <span class="label-caps block">Website</span>
                    <button class="cursor-pointer border-0 bg-transparent p-0 text-left text-[13px] break-all text-brand underline" onclick={() => openLink(t.homepage!)}>{t.homepage}</button>
                  </div>
                {/if}
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
                  {#if Object.values(t.outputs).some((o) => !['tsv', 'csv', 'tab'].includes(o.type))}
                    <div class="mt-3 flex items-center gap-3 border-t border-line pt-3">
                      <Toggle
                        small
                        label={`Show the reports of ${t.name} as tables`}
                        checked={!t.hidden_outputs.includes(REPORT_TABLES)}
                        onchange={(v) => showOutput(t, REPORT_TABLES, v)}
                      />
                      <span>
                        <span class="font-semibold text-ink">Show its reports as tables</span> · text and JSON reports are read into tables
                        with headings, every sample together.
                        {t.source === 'built-in' && !t.report_tables
                          ? 'Switch off to see the files as the tool wrote them.'
                          : 'The tables are also saved as files beside the tool\'s own, as its "tables" output: connect that to Summary tables in the workflow builder. Switch off for only the files the tool wrote.'}
                      </span>
                    </div>
                  {/if}
                </div>
                <div class="col-span-full flex flex-col gap-2 border-t border-line pt-3">
                  <span class="label-caps block">Versions</span>
                  <span>
                    This computer has {t.name} {t.version}{t.variants.length ? ` and ${t.variants.join(', ')}` : ''}. Another version
                    sits beside it: each step of a workflow picks the version it uses, and saved workflows keep theirs.
                  </span>
                  {#if !v}
                    <div><Button size="sm" icon="search" busy={looking === t.id} onclick={() => lookUp(t)}>See other versions</Button></div>
                  {:else if !v.package}
                    <span class="text-muted">No other versions of this tool are published to add.</span>
                  {:else if !v.available.length}
                    <span class="text-muted">Every published version is already here.</span>
                  {:else}
                    <div class="flex flex-wrap items-center gap-2">
                      <label class="sr-only" for="ver-{t.id}">Version of {t.name} to add</label>
                      <select id="ver-{t.id}" class="input !w-auto min-w-[180px]" bind:value={pick[t.id]}>
                        {#each v.available as a, i (a.version)}<option value={a.version}>{a.version}{i === 0 ? ' (newest)' : ''}</option>{/each}
                      </select>
                      <Button size="sm" icon="plus" busy={addingVersion} onclick={() => addVersion(t)}>Add this version</Button>
                    </div>
                  {/if}
                </div>
              </div>
            {/if}
          </div>
        {/each}
      </div>
    </section>
  {/each}
</div>

<Dialog bind:open={addOpen} title="Add a tool from GitHub">
  <form class="flex flex-col gap-3" onsubmit={(e) => { e.preventDefault(); if (!builtinHit) add(); }}>
    <p class="m-0">
      Paste the link to the tool's GitHub page. The app reads its name, description, licence and newest release, looks for its
      container, and opens the editor so you can finish it. Nothing on GitHub is changed: the tool is kept on this computer.
    </p>
    <label class="flex flex-col gap-1.5 text-[13px] font-semibold">
      GitHub link
      <input class="input" bind:value={link} placeholder="https://github.com/owner/tool" />
    </label>
    {#if builtinHit}
      <div class="flex gap-3 rounded-xl border border-warn-line bg-warn-soft p-3.5 text-[13px] text-warn-ink" role="alert">
        <span class="mt-0.5 flex-none"><Icon name="info" size={18} /></span>
        <span>
          <strong>{builtinHit.name} {builtinHit.version} is already built into the app.</strong>
          The built-in one is set up and tested, and is ready to use in workflows: you don't need to add it.
          {#if builtinHit.canAdd}
            Adding it from GitHub makes a second, separate tool that the app sets up by best guess, so its results may differ from
            the built-in one. Do that only if you need another version of it.
          {:else}
            Its GitHub setup is the same one the app already uses, so there is nothing more to add.
          {/if}
        </span>
      </div>
    {/if}
  </form>
  {#snippet actions()}
    <Button onclick={() => (addOpen = false)}>Cancel</Button>
    {#if builtinHit}
      {#if builtinHit.canAdd}<Button busy={adding} onclick={() => add(true)}>Add it anyway</Button>{/if}
      <Button variant="primary" onclick={useBuiltin}>Show the built-in one</Button>
    {:else}
      <Button variant="primary" busy={adding} disabled={!link.trim()} onclick={() => add()}>Add</Button>
    {/if}
  {/snippet}
</Dialog>
