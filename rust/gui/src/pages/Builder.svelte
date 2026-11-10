<script lang="ts">
  // The workflow builder: tools on the left, the workflow as boxes in the middle
  // (drag from an output to an input to connect), the chosen step's settings on
  // the right. Plain-language problems at the top, each with a fix button.
  import { untrack } from 'svelte';
  import { Background, Controls, MiniMap, SvelteFlow, type Connection, type Edge, type Node } from '@xyflow/svelte';
  import '@xyflow/svelte/dist/style.css';
  import PageHeader from '../lib/PageHeader.svelte';
  import StepNode from '../lib/StepNode.svelte';
  import Button from '../lib/ui/Button.svelte';
  import Icon from '../lib/ui/Icon.svelte';
  import Pill from '../lib/ui/Pill.svelte';
  import Toggle from '../lib/ui/Toggle.svelte';
  import { get, post, type Tool, type Workflow, type WorkflowView } from '../lib/api';
  import { live } from '../lib/live.svelte';
  import { addStep, candidates, downstream, INPUT, label as srcLabel, layout, problems, removeStep, slug, sourceType } from '../lib/builder';
  import { go, href } from '../lib/router.svelte';
  import { errorText, toasts } from '../lib/toast.svelte';

  let { id, version = null, startFrom = null }: { id: string; version?: string | null; startFrom?: string | null } = $props();

  // the page is re-made when the address changes (App keys it)
  const isNew = untrack(() => id) === 'new';
  let tools = $state<Record<string, Tool>>({});
  let wf = $state<Workflow | null>(null);
  let view = $state<WorkflowView | null>(null);
  let editing = $state(isNew);
  let dirty = $state(false);
  let saving = $state(false);
  let selected = $state<string | null>(null);
  let search = $state('');
  let history: string[] = [];
  let nodes = $state.raw<Node[]>([]);
  let edges = $state.raw<Edge[]>([]);

  const nodeTypes = { step: StepNode };

  async function load() {
    try {
      const list = await get<Tool[]>('/api/tools');
      tools = Object.fromEntries(list.map((t) => [t.id, t]));
      if (isNew && !startFrom) {
        wf = { id: '', name: 'My new workflow', version: 1, description: '', steps: {} };
      } else {
        view = await get<WorkflowView>(`/api/workflows/${encodeURIComponent(startFrom ?? id)}${version ? `?version=${version}` : ''}`);
        wf = structuredClone($state.snapshot(view.workflow)) as Workflow;
        if (isNew) {
          wf.name = `${wf.name} (my copy)`;
          wf.id = '';
          delete (wf as Record<string, unknown>).extends;
        }
      }
      rebuild();
    } catch (e) {
      toasts.show(errorText(e), 'error');
    }
  }
  load();

  const builtin = $derived(!!view?.builtin && !isNew);
  const issues = $derived(wf ? problems(wf, tools) : []);
  const broken = $derived(new Set(issues.map((p) => p.step)));

  function rebuild() {
    if (!wf) return;
    const ports = (sid: string) => {
      const t = tools[wf!.steps[sid]?.tool];
      return t ? Math.max(Object.keys(t.inputs).length, Object.keys(t.outputs).length, 1) : 1;
    };
    // a box: title area plus one 22 px row per input/output
    const pos = layout(wf, (sid) => 66 + (sid === INPUT ? 1 : ports(sid)) * 22);
    const port = (m: Record<string, { type: string; optional?: boolean }>) => Object.entries(m).map(([name, p]) => ({ name, type: p.type, optional: p.optional }));
    nodes = [
      {
        id: INPUT,
        type: 'step',
        position: pos[INPUT],
        deletable: false,
        draggable: editing,
        data: { title: 'Paired reads', category: 'Input', ins: [], outs: [{ name: 'reads', type: 'reads' }], broken: false, off: false, isInput: true },
      },
      ...Object.entries(wf.steps).map(([sid, st]) => {
        const t = tools[st.tool];
        return {
          id: sid,
          type: 'step',
          position: pos[sid],
          selected: sid === selected,
          draggable: editing,
          deletable: editing,
          data: {
            title: st.label ?? t?.name ?? sid,
            category: t?.category ?? 'Tool',
            ins: port(t?.inputs ?? {}),
            outs: port(t?.outputs ?? {}),
            broken: broken.has(sid),
            off: st.enabled === false,
            isInput: false,
          },
        } satisfies Node;
      }),
    ];
    const es: Edge[] = [];
    for (const [sid, st] of Object.entries(wf.steps))
      for (const [inp, srcs] of Object.entries(st.inputs ?? {}))
        srcs.forEach((src, n) => {
          // an input that takes every source has no "else": each line counts
          const i = tools[st.tool]?.inputs[inp]?.collect ? 0 : n;
          const [from, out] = src === INPUT ? [INPUT, 'reads'] : src.split('.');
          es.push({
            id: `${src}->${sid}.${inp}`,
            source: from,
            sourceHandle: out,
            target: sid,
            targetHandle: inp,
            deletable: editing,
            animated: false,
            label: i > 0 ? 'if the other is off' : undefined,
            style: i > 0 ? 'stroke-dasharray: 6 5; stroke: var(--muted)' : 'stroke: var(--brand); stroke-width: 2',
          });
        });
    edges = es;
  }

  function change(fn: (w: Workflow) => void) {
    if (!wf || !editing) return;
    history.push(JSON.stringify($state.snapshot(wf)));
    if (history.length > 60) history.shift();
    fn(wf);
    dirty = true;
    rebuild();
  }
  function undo() {
    const prev = history.pop();
    if (!prev) return;
    wf = JSON.parse(prev);
    dirty = history.length > 0 || dirty;
    rebuild();
  }

  function startEditing() {
    editing = true;
    rebuild();
  }

  // ---------------------------------------------------------------- canvas events

  function valid(c: Connection | Edge) {
    if (!wf || !c.target || c.target === INPUT || !c.targetHandle) return false;
    const src = c.source === INPUT ? INPUT : `${c.source}.${c.sourceHandle}`;
    const t = tools[wf.steps[c.target]?.tool];
    const spec = t?.inputs[c.targetHandle];
    if (spec?.collect) {
      // any number of sources, as long as each is a kind of file it takes and no loop is made
      const kind = sourceType(wf, tools, src);
      return !!kind && (spec.accepts ?? []).includes(kind) && c.source !== c.target && !downstream(wf, c.target).has(c.source);
    }
    return sourceType(wf, tools, src) === t?.inputs[c.targetHandle]?.type && candidates(wf, tools, c.target, t.inputs[c.targetHandle].type).includes(src);
  }
  function onconnect(c: Connection) {
    if (!valid(c)) {
      const spec = wf && c.targetHandle ? tools[wf.steps[c.target]?.tool]?.inputs[c.targetHandle] : undefined;
      toasts.show(
        spec?.collect
          ? `Only tables fit here (${(spec.accepts ?? []).join(', ')} files). That output is another kind of file.`
          : 'Those don\'t fit together: the output and input are different kinds of file (or it would make a loop).',
        'error',
      );
      rebuild();
      return;
    }
    const src = c.source === INPUT ? INPUT : `${c.source}.${c.sourceHandle}`;
    change((w) => {
      const st = w.steps[c.target];
      st.inputs ??= {};
      const list = st.inputs[c.targetHandle!] ?? [];
      const collect = tools[st.tool]?.inputs[c.targetHandle!]?.collect;
      if (!list.includes(src)) st.inputs[c.targetHandle!] = collect ? [...list, src] : [src, ...list];
    });
  }
  function ondelete({ nodes: ns, edges: es }: { nodes: Node[]; edges: Edge[] }) {
    change((w) => {
      for (const e of es) {
        const [src, rest] = e.id.split('->');
        const [sid, inp] = rest.split('.');
        const st = w.steps[sid];
        if (st?.inputs?.[inp]) st.inputs[inp] = st.inputs[inp].filter((s) => s !== src);
      }
      for (const n of ns) if (n.id !== INPUT) removeStep(w, n.id);
    });
    if (ns.some((n) => n.id === selected)) selected = null;
  }
  function ondragstop({ targetNode }: { targetNode: Node | null }) {
    if (!wf || !targetNode || !editing) return;
    wf.layout = { ...(wf.layout ?? {}), [targetNode.id]: { x: Math.round(targetNode.position.x), y: Math.round(targetNode.position.y) } };
    dirty = true;
  }

  // ---------------------------------------------------------------- tools list

  const groups = $derived.by(() => {
    const q = search.trim().toLowerCase();
    const g = new Map<string, Tool[]>();
    for (const t of Object.values(tools)) {
      if (q && !`${t.name} ${t.id} ${t.category} ${t.description}`.toLowerCase().includes(q)) continue;
      const c = t.category ?? 'Other';
      g.set(c, [...(g.get(c) ?? []), t]);
    }
    return [...g.entries()].sort((a, b) => a[0].localeCompare(b[0]));
  });
  function add(t: Tool) {
    if (!editing) startEditing();
    let sid = '';
    change((w) => (sid = addStep(w, tools, t.id)));
    selected = sid;
    rebuild();
    toasts.show(`Added ${t.name}${Object.keys(wf!.steps[sid].inputs ?? {}).length ? ', connected for you' : ''}.`);
  }

  // ---------------------------------------------------------------- selected step

  const sel = $derived(wf && selected && selected !== INPUT ? wf.steps[selected] : null);
  const selTool = $derived(sel ? tools[sel.tool] : null);
  const selVersions = $derived(selTool ? [selTool.version, ...selTool.variants] : []);

  function setSetting(name: string, value: unknown) {
    change((w) => {
      const st = w.steps[selected!];
      st.settings = { ...(st.settings ?? {}), [name]: value };
    });
  }
  function valueOf(name: string) {
    return sel?.settings?.[name] ?? selTool?.settings[name]?.default ?? '';
  }

  // ---------------------------------------------------------------- save

  let newName = $state('');
  async function save(andRun = false) {
    if (!wf) return;
    saving = true;
    try {
      const target = builtin || isNew ? slug(newName || wf.name) : null;
      const body = { workflow: { ...$state.snapshot(wf), id: wf.id || target }, new_id: target };
      const res = await post<{ id: string; version: number }>('/api/workflows/save', body);
      toasts.show(`Saved "${wf.name}" as version ${res.version}.`);
      dirty = false;
      if (andRun) go(`new?workflow=${res.id}`);
      else go(`workflows/${res.id}?version=${res.version}`);
    } catch (e) {
      toasts.show(errorText(e), 'error');
    } finally {
      saving = false;
    }
  }
</script>

<PageHeader title={wf?.name || 'Workflow'} crumbs={[{ label: 'Workflows', href: href('workflows') }]}>
  {#snippet extra()}
    {#if editing}<Pill status={dirty ? 'draft' : 'draft'} text={dirty ? 'Draft, not saved' : 'Editing'} />{:else if builtin}<Pill status="builtin" />{:else if view}<Pill status="yours" text="Version {view.version}" />{/if}
  {/snippet}
  {#snippet actions()}
    {#if editing}
      <Button icon="undo" label="Undo" disabled={!history.length} onclick={undo} />
      <Button icon="play" disabled={issues.length > 0 || saving} onclick={() => save(true)}>Save and run</Button>
      <Button variant="primary" busy={saving} disabled={issues.length > 0 || !wf || Object.keys(wf.steps).length === 0} onclick={() => save()}>
        {builtin || isNew ? 'Save as new workflow' : 'Save new version'}
      </Button>
    {:else}
      <Button icon="play" href={href(`new?workflow=${id}`)}>Run</Button>
      {#if builtin}
        <Button variant="primary" icon="copy" href={href(`workflows/new?from=${id}`)}>Make my own copy</Button>
      {:else}
        <Button variant="primary" icon="edit" onclick={startEditing}>Edit</Button>
      {/if}
    {/if}
  {/snippet}
</PageHeader>

{#if wf}
  <div class="flex min-h-0 flex-1 flex-wrap">
    <aside aria-label="Tools" class="flex w-[264px] shrink-0 flex-col gap-3 overflow-y-auto border-r border-line bg-surface p-4 max-[900px]:w-full max-[900px]:border-r-0 max-[900px]:border-b" style="max-height: calc(100vh - 80px)">
      <label class="flex h-10 items-center gap-2 rounded-[10px] border border-line-strong px-3">
        <span class="text-muted"><Icon name="search" size={16} /></span>
        <span class="sr-only">Search tools</span>
        <input class="min-w-0 flex-1 border-0 bg-transparent text-sm text-ink outline-none" placeholder="Search tools" bind:value={search} />
      </label>
      <p class="m-0 text-xs text-muted">Click <strong>+</strong> to add a tool: it is connected for you. Drag between the dots to change a connection.</p>
      {#each groups as [cat, list] (cat)}
        <div class="flex flex-col gap-0.5">
          <span class="label-caps px-1 pt-1 pb-0.5">{cat}</span>
          {#each list as t (t.id)}
            <div class="flex items-center gap-2 rounded-[10px] p-2 hover:bg-brand-softer">
              <span class="flex min-w-0 flex-1 flex-col">
                <span class="truncate text-sm font-semibold">{t.name}</span>
                <span class="line-clamp-2 text-xs text-muted">{t.description ?? ''}</span>
              </span>
              <button
                class="flex h-[30px] w-[30px] shrink-0 cursor-pointer items-center justify-center rounded-lg border border-brand/40 bg-brand-softer text-brand hover:bg-brand-soft"
                aria-label="Add {t.name}"
                onclick={() => add(t)}><Icon name="plus" size={14} stroke={2.6} /></button
              >
            </div>
          {/each}
        </div>
      {/each}
    </aside>

    <section aria-label="Workflow" class="flex min-w-0 flex-[999_1_420px] flex-col gap-3 p-4">
      {#if editing && (isNew || builtin)}
        <div class="flex flex-wrap gap-3">
          <label class="flex flex-[1_1_260px] flex-col gap-1 text-[13px] font-semibold">Workflow name <input class="input" bind:value={wf.name} oninput={() => (dirty = true)} /></label>
          <label class="flex flex-[2_1_320px] flex-col gap-1 text-[13px] font-semibold">What it is for <input class="input" bind:value={wf.description} placeholder="One line other users will see" oninput={() => (dirty = true)} /></label>
        </div>
      {/if}
      {#each issues.slice(0, 3) as p}
        <div class="flex flex-wrap items-center gap-3 rounded-xl border border-warn-line bg-warn-soft px-3.5 py-3">
          <span class="text-warn"><Icon name="alert" /></span>
          <span class="flex-[1_1_260px] text-sm text-warn-ink">{p.message}</span>
          {#if p.fix && editing}
            <Button size="sm" variant="primary" onclick={() => change((w) => { const st = w.steps[p.step]; st.inputs = { ...(st.inputs ?? {}), [p.input!]: [p.fix!] }; })}>Connect for me</Button>
          {:else if p.fix}
            <Button size="sm" onclick={startEditing}>Edit to fix</Button>
          {/if}
        </div>
      {/each}
      {#if issues.length > 3}<p class="m-0 text-[13px] text-warn-ink">…and {issues.length - 3} more to fix.</p>{/if}
      {#if !issues.length && Object.keys(wf.steps).length}
        <div class="flex items-center gap-3 rounded-xl border border-ok bg-ok-soft px-3.5 py-3 text-sm text-ok-ink">
          <Icon name="check" /> <span><strong>Everything is connected.</strong>{editing ? ' Save it, then try a run on one sample.' : ''}</span>
        </div>
      {/if}
      {#if !Object.keys(wf.steps).length}
        <div class="rounded-xl border border-line bg-surface px-4 py-3.5 text-sm text-ink-2">
          Start by adding <strong>fastp</strong> (cleans the reads) and <strong>Shovill</strong> (assembly) from the list, then the typing tools you need.
        </div>
      {/if}

      <div class="relative min-h-[560px] flex-1 overflow-hidden rounded-[14px] border border-line bg-surface">
        <SvelteFlow
          bind:nodes
          bind:edges
          {nodeTypes}
          fitView
          minZoom={0.3}
          nodesConnectable={editing}
          elementsSelectable={true}
          isValidConnection={valid}
          {onconnect}
          {ondelete}
          onnodedragstop={ondragstop}
          onnodeclick={({ node }) => (selected = node.id)}
          onpaneclick={() => (selected = null)}
          deleteKey={editing ? ['Delete', 'Backspace'] : null}
          proOptions={{ hideAttribution: true }}
        >
          <Background gap={20} />
          <Controls showLock={false} />
          <MiniMap pannable zoomable width={150} height={100} />
        </SvelteFlow>
      </div>
    </section>

    <aside aria-label="Step settings" class="flex w-[340px] shrink-0 flex-col gap-4 overflow-y-auto border-l border-line bg-surface p-4.5 max-[1200px]:w-full max-[1200px]:border-l-0 max-[1200px]:border-t" style="max-height: calc(100vh - 80px)">
      {#if sel && selTool}
        <div class="flex flex-col gap-1">
          <span class="text-xs font-semibold text-muted">Selected step</span>
          {#if editing}
            <label class="sr-only" for="step-label">Step name</label>
            <input id="step-label" class="input text-base font-semibold" value={sel.label ?? selTool.name} onchange={(e) => change((w) => (w.steps[selected!].label = e.currentTarget.value))} />
          {:else}
            <h2 class="m-0 text-lg font-semibold">{sel.label ?? selTool.name}</h2>
          {/if}
          <p class="m-0 text-[13px] leading-relaxed text-muted">{selTool.description}</p>
        </div>
        <label class="flex flex-col gap-1.5 text-[13px] font-semibold">
          Tool version
          <select
            class="input"
            disabled={!editing}
            value={sel.tool_version ?? selTool.version}
            onchange={(e) => change((w) => { const v = e.currentTarget.value; w.steps[selected!].tool_version = v === selTool.version ? undefined : v; })}
          >
            {#each selVersions as v, i}<option value={v}>{v}{i === 0 ? ' (original)' : ''}</option>{/each}
          </select>
        </label>
        <div class="flex flex-col gap-3.5 border-t border-line pt-3.5">
          {#each Object.entries(selTool.settings) as [name, spec] (name)}
            {@const v = valueOf(name)}
            {@const isAuto = spec.auto !== undefined && v === spec.auto.value}
            <div class="flex flex-col gap-1.5">
              <div class="flex items-center gap-2">
                <span class="flex-1 text-[13px] font-semibold">{spec.label ?? name}</span>
                {#if spec.auto}
                  <span class="flex items-center gap-1.5 text-xs font-semibold text-brand">
                    <Toggle small label="Auto for {spec.label ?? name}" checked={isAuto} disabled={!editing} onchange={(on) => setSetting(name, on ? spec.auto!.value : (spec.type === 'integer' || spec.type === 'number' ? 1 : ''))} />Auto
                  </span>
                {/if}
              </div>
              {#if isAuto}
                <span class="rounded-[9px] bg-surface-2 px-3 py-2 text-sm text-ink-2">{spec.auto?.label}</span>
              {:else if spec.type === 'boolean'}
                <Toggle label={spec.label ?? name} checked={v === true || v === 'true'} disabled={!editing} onchange={(on) => setSetting(name, on)} />
              {:else if spec.type === 'select'}
                <select class="input" disabled={!editing} value={String(v)} onchange={(e) => setSetting(name, e.currentTarget.value)}>
                  {#each spec.options ?? [] as o}
                    {@const db = selTool.database_setting === name ? live.databases.find((d) => d.id === o) : undefined}
                    <option value={o}>{spec.option_labels?.[o] ?? o}{db && !db.installed ? ' · not installed yet' : ''}</option>
                  {/each}
                </select>
              {:else}
                <input
                  class="input"
                  disabled={!editing}
                  type={spec.type === 'integer' || spec.type === 'number' ? 'number' : 'text'}
                  step={spec.type === 'number' ? 'any' : '1'}
                  value={String(v)}
                  onchange={(e) => setSetting(name, spec.type === 'integer' ? parseInt(e.currentTarget.value) : spec.type === 'number' ? parseFloat(e.currentTarget.value) : e.currentTarget.value)}
                />
              {/if}
              {#if spec.description}<span class="text-xs leading-snug text-muted">{spec.description}</span>{/if}
            </div>
          {:else}
            <p class="m-0 text-sm text-muted">This tool has no settings.</p>
          {/each}
        </div>
        {#if Object.keys(selTool.inputs).length}
          <div class="flex flex-col gap-1.5 border-t border-line pt-3.5">
            <span class="text-[13px] font-semibold">Gets its input from</span>
            {#each Object.entries(selTool.inputs) as [inp, spec]}
              <span class="text-[13px] text-ink-2"><strong>{inp}</strong>: {(sel.inputs?.[inp] ?? []).map((s) => srcLabel(wf!, tools, s)).join(spec.collect ? ', ' : ', else ') ||
                  (spec.collect ? 'nothing yet: drag any table output onto it' : spec.optional ? 'nothing (optional)' : 'not connected')}</span>
            {/each}
          </div>
        {/if}
        {#if editing}
          <div class="mt-auto flex flex-wrap gap-2 pt-2">
            <Button size="sm" variant={sel.enabled === false ? 'primary' : 'secondary'} onclick={() => change((w) => (w.steps[selected!].enabled = w.steps[selected!].enabled === false ? undefined : false))}>
              {sel.enabled === false ? 'Switch on' : 'Switch off'}
            </Button>
            <Button size="sm" variant="danger" icon="trash" onclick={() => { const s = selected!; change((w) => removeStep(w, s)); selected = null; }}>Remove step</Button>
          </div>
        {/if}
      {:else}
        <div class="flex flex-col gap-2 text-sm text-muted">
          <h2 class="m-0 text-base font-semibold text-ink">{wf.name}</h2>
          <p class="m-0">{wf.description || 'Click a step to see and change its settings.'}</p>
          <p class="m-0">{Object.keys(wf.steps).length} steps.</p>
          {#if !editing && !builtin}<Button size="sm" icon="edit" onclick={startEditing}>Edit this workflow</Button>{/if}
        </div>
      {/if}
    </aside>
  </div>
{:else}
  <p class="m-8 text-sm text-muted">Loading…</p>
{/if}
