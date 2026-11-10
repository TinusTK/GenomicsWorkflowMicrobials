<script lang="ts">
  // Edit a tool kept on this computer (added from GitHub or by a Galaxy import):
  // its container, inputs, outputs, settings and command, with a guide beside
  // the form, the tool's own help text and a try-out on one sample.
  import FolderPicker from '../lib/FolderPicker.svelte';
  import PageHeader from '../lib/PageHeader.svelte';
  import Button from '../lib/ui/Button.svelte';
  import Dialog from '../lib/ui/Dialog.svelte';
  import Icon from '../lib/ui/Icon.svelte';
  import Toggle from '../lib/ui/Toggle.svelte';
  import { del, get, post, type Job, type Tool } from '../lib/api';
  import { asTable, fileKind } from '../lib/results';
  import { go, href } from '../lib/router.svelte';
  import { errorText, toasts } from '../lib/toast.svelte';

  let { id }: { id: string } = $props();

  const FILE_TYPES = ['reads', 'fasta', 'tsv', 'csv', 'txt', 'json', 'html', 'log', 'any'];
  const SETTING_TYPES: [string, string][] = [
    ['string', 'Text'],
    ['integer', 'Whole number'],
    ['number', 'Number'],
    ['boolean', 'On / off'],
    ['select', 'One of a list'],
  ];

  interface InputRow { name: string; type: string; optional: boolean; description: string }
  interface OutputRow { name: string; type: string; path: string; optional: boolean; description: string }
  interface SettingRow { name: string; label: string; type: string; value: string; description: string; options: string }

  let tool = $state<Tool | null>(null);
  let missing = $state('');
  let f = $state({
    name: '', version: '', category: '', description: '', homepage: '', container: '', scope: 'per_sample',
    program: '', version_command: '', command: '', cpus: 4, memory_gb: 8, ready: false,
  });
  let inputs = $state<InputRow[]>([]);
  let outputs = $state<OutputRow[]>([]);
  let settings = $state<SettingRow[]>([]);
  let saving = $state(false);

  async function load() {
    try {
      const t = (await get<Tool[]>('/api/tools')).find((x) => x.id === id);
      if (!t) {
        missing = 'This tool is not on this computer any more.';
        return;
      }
      if (t.source === 'built-in') {
        missing = `${t.name} is built in and can't be changed. To change how it runs, use its settings in a workflow.`;
        return;
      }
      tool = t;
      // no container found: that has to be filled in first
      moreOpen = !t.container.trim();
      f = {
        name: t.name, version: t.version, category: t.category ?? '', description: t.description ?? '',
        homepage: t.homepage ?? '', container: t.container, scope: t.scope, program: t.program ?? t.id,
        version_command: t.version_command ?? '', command: t.command ?? '',
        cpus: t.resources?.cpus ?? 1, memory_gb: t.resources?.memory_gb ?? 2, ready: !t.draft,
      };
      inputs = Object.entries(t.inputs).map(([name, i]) => ({ name, type: i.type || 'any', optional: !!i.optional, description: i.description ?? '' }));
      outputs = Object.entries(t.outputs)
        // the app's own "tables" output (its reports as tables) is not the tool's to edit
        .filter(([name, o]) => !(t.report_tables && name === 'tables' && o.path === 'out/tables/*.tsv'))
        .map(([name, o]) => ({ name, type: o.type || 'any', path: o.path, optional: !!o.optional, description: o.description ?? '' }));
      settings = Object.entries(t.settings).map(([name, s]) => ({
        name, label: s.label ?? name, type: s.type ?? 'string', value: s.default == null ? '' : String(s.default),
        description: s.description ?? '', options: (s.options ?? []).join(', '),
      }));
    } catch (e) {
      toasts.show(errorText(e), 'error');
    }
  }
  load();

  const perSample = $derived(f.scope !== 'all_samples');

  function settingDefault(s: SettingRow): unknown {
    if (s.type === 'boolean') return s.value === 'true';
    if (s.type === 'integer' || s.type === 'number') return s.value.trim() === '' || isNaN(Number(s.value)) ? '' : Number(s.value);
    return s.value;
  }

  async function save(quiet = false): Promise<boolean> {
    saving = true;
    try {
      await post(`/api/tools/${encodeURIComponent(id)}/definition`, {
        ...f,
        inputs: Object.fromEntries(inputs.map((i) => [i.name.trim(), { type: i.type, optional: i.optional, description: i.description }])),
        outputs: Object.fromEntries(outputs.map((o) => [o.name.trim(), { type: o.type, path: o.path, optional: o.optional, description: o.description }])),
        settings: Object.fromEntries(
          settings.map((s) => [
            s.name.trim(),
            { label: s.label, type: s.type, default: settingDefault(s), description: s.description, options: s.options.split(',').map((o) => o.trim()).filter(Boolean) },
          ]),
        ),
      });
      if (!quiet) toasts.show(f.ready ? `Saved. ${f.name} is ready to use in workflows.` : `Saved. ${f.name} is still marked not ready.`);
      return true;
    } catch (e) {
      toasts.show(errorText(e), 'error');
      return false;
    } finally {
      saving = false;
    }
  }

  let deleteOpen = $state(false);
  async function remove() {
    try {
      await del(`/api/tools/${encodeURIComponent(id)}`);
      toasts.show(`Removed ${f.name} from this computer.`);
      go('tools');
    } catch (e) {
      toasts.show(errorText(e), 'error');
      deleteOpen = false;
    }
  }

  // ---------------------------------------------------------------- the guide

  /** The names the command can use, from what the form holds now. */
  const names = $derived.by(() => {
    const out: { name: string; what: string }[] = [
      { name: '$SAMPLE', what: perSample ? "the sample's name" : 'the word "all" (this tool runs once for every sample together)' },
      { name: '$CPUS', what: 'CPU threads it may use' },
      { name: '$MEM_GB', what: 'memory it may use, in GB' },
    ];
    for (const i of inputs.filter((x) => x.name.trim())) {
      const n = i.name.trim();
      if (!perSample) out.push({ name: `$IN_${n}_DIR`, what: `a folder holding every sample's ${n} file` });
      else if (i.type === 'reads') {
        out.push({ name: `$IN_${n}_1`, what: 'the forward reads file' });
        out.push({ name: `$IN_${n}_2`, what: 'the reverse reads file' });
      } else out.push({ name: `$IN_${n}`, what: `the ${n} file` });
    }
    for (const s of settings.filter((x) => x.name.trim())) out.push({ name: `$S_${s.name.trim()}`, what: `the setting "${s.label || s.name}"` });
    return out;
  });

  const uses = (name: string) => new RegExp(`\\$\\{?${name}\\b`).test(f.command);

  /** Plain checks of the form, shown as a list: done, or what is still to do. */
  const checks = $derived.by(() => {
    const out: { ok: boolean; text: string }[] = [];
    out.push({ ok: !!f.container.trim(), text: f.container.trim() ? 'It has a container.' : 'Fill in the container (where the tool is installed).' });
    out.push({ ok: !!f.command.trim(), text: f.command.trim() ? 'The command is written.' : 'Write the command.' });
    for (const i of inputs.filter((x) => x.name.trim())) {
      const n = i.name.trim();
      const used = !perSample ? uses(`IN_${n}_DIR`) : i.type === 'reads' ? uses(`IN_${n}_1`) && uses(`IN_${n}_2`) : uses(`IN_${n}`);
      const want = !perSample ? `$IN_${n}_DIR` : i.type === 'reads' ? `$IN_${n}_1 and $IN_${n}_2` : `$IN_${n}`;
      out.push({ ok: used, text: used ? `The command reads the input "${n}".` : `The command never uses the input "${n}": put ${want} in it.` });
    }
    for (const s of settings.filter((x) => x.name.trim())) {
      const n = s.name.trim();
      out.push({ ok: uses(`S_${n}`), text: uses(`S_${n}`) ? `The command uses the setting "${n}".` : `The command never uses the setting "${n}": put $S_${n} in it.` });
    }
    out.push({
      ok: outputs.length > 0,
      text: outputs.length ? `It lists ${outputs.length} output${outputs.length === 1 ? '' : 's'}.` : 'Add at least one output, so its results are kept and shown.',
    });
    for (const o of outputs.filter((x) => x.name.trim())) {
      if (!o.path.trim()) out.push({ ok: false, text: `The output "${o.name}" needs a path, like out/{sample}.${o.name}.tsv.` });
      else if (!o.path.trim().startsWith('out/')) out.push({ ok: false, text: `The output "${o.name}" is not under out/: later steps can use it, but it is not saved with the results.` });
    }
    if (f.command.trim() && outputs.length && !f.command.includes('out/')) out.push({ ok: false, text: 'The command never writes into out/.' });
    return out;
  });
  const todo = $derived(checks.filter((c) => !c.ok).length);

  const example = $derived.by(() => {
    const p = f.program.trim() || 'mytool';
    const reads = inputs.find((i) => i.type === 'reads' && i.name.trim());
    const other = inputs.find((i) => i.type !== 'reads' && i.name.trim());
    const input = reads ? `-1 "$IN_${reads.name.trim()}_1" -2 "$IN_${reads.name.trim()}_2"` : other ? `"$IN_${other.name.trim()}"` : '"$IN_contigs"';
    return `${p} --threads "$CPUS" ${input} > "out/$SAMPLE.${p}.tsv"`;
  });

  let commandBox: HTMLTextAreaElement | undefined = $state();
  /** Put a name into the command where the cursor is. */
  function insert(text: string) {
    const quoted = `"${text}"`;
    const at = commandBox?.selectionStart ?? f.command.length;
    const end = commandBox?.selectionEnd ?? at;
    f.command = f.command.slice(0, at) + quoted + f.command.slice(end);
    queueMicrotask(() => {
      commandBox?.focus();
      commandBox?.setSelectionRange(at + quoted.length, at + quoted.length);
    });
  }

  // ---------------------------------------------------------------- help text and try-out

  async function follow(job: Job): Promise<Job> {
    let j = job;
    while (j.status === 'running') {
      await new Promise((r) => setTimeout(r, 1200));
      j = await get<Job>(`/api/jobs/${j.id}`);
    }
    return j;
  }

  let helpText = $state('');
  let helpBusy = $state(false);
  async function showHelp() {
    helpBusy = true;
    helpText = '';
    try {
      if (!(await save(true))) return;
      const j = await follow(await post<Job>(`/api/tools/${encodeURIComponent(id)}/help`, { program: f.program }));
      if (j.status === 'failed') toasts.show(j.log.at(-1) ?? 'Could not get the help text.', 'error');
      else helpText = (j.result as { text: string }).text;
    } catch (e) {
      toasts.show(errorText(e), 'error');
    } finally {
      helpBusy = false;
    }
  }

  interface Trial {
    exit: number;
    log: string;
    outputs: { name: string; pattern: string; optional: boolean; files: string[] }[];
    files: { path: string; size: number; preview: string | null }[];
    dir: string;
    sample: string;
  }
  let readsFolder = $state('');
  let pickerOpen = $state(false);
  let tryFiles = $state<Record<string, string>>({});
  let trying = $state(false);
  let trial = $state<Trial | null>(null);
  let tryError = $state('');
  const needsReads = $derived(inputs.some((i) => i.type === 'reads'));
  const fileInputs = $derived(inputs.filter((i) => i.type !== 'reads' && i.name.trim()));
  // a tool that takes an assembly can be tried on the first FASTA file of a folder
  const needsFasta = $derived(inputs.some((i) => i.type === 'fasta'));

  async function tryIt() {
    trying = true;
    trial = null;
    tryError = '';
    try {
      if (!(await save(true))) return;
      const j = await follow(await post<Job>(`/api/tools/${encodeURIComponent(id)}/try`, { reads_folder: readsFolder, files: tryFiles }));
      trial = (j.result as Trial | undefined) ?? null;
      if (j.status === 'failed') tryError = (j.log.at(-1) ?? 'It did not work.').replace(/^Error: /, '');
    } catch (e) {
      tryError = errorText(e);
    } finally {
      trying = false;
    }
  }

  /** files the command made that no output lists */
  const unlisted = $derived(trial ? trial.files.filter((x) => !trial!.outputs.some((o) => o.files.includes(x.path))) : []);
  const size = (n: number) => (n > 1e6 ? `${(n / 1e6).toFixed(1)} MB` : n > 1e3 ? `${Math.round(n / 1e3)} kB` : `${n} bytes`);

  /** Add an output for a file the try-out made. */
  function keep(path: string) {
    const file = path.split('/').pop() ?? path;
    // the sample's name in the file name becomes {sample}, so the path fits every sample
    const sample = trial && trial.sample !== 'all' && file.includes(trial.sample) ? trial.sample : '';
    const pattern = sample ? path.replace(sample, '{sample}') : path;
    const ext = (file.split('.').pop() ?? '').toLowerCase();
    let name = file.replace(sample, '').replace(/\.[^.]+$/, '').replace(/[^A-Za-z0-9]+/g, '_').replace(/^_+|_+$/g, '').toLowerCase() || 'report';
    if (!/^[a-z]/i.test(name)) name = `out_${name}`;
    while (outputs.some((o) => o.name === name)) name += '_2';
    outputs.push({ name, type: FILE_TYPES.includes(ext) ? ext : ext === 'tab' ? 'tsv' : 'any', path: pattern, optional: false, description: '' });
    toasts.show(`Added the output "${name}". Save, then try again to check it.`, 'info');
  }

  // ---------------------------------------------------------------- set it up for me

  let moreOpen = $state(false);
  let settingUp = $state(false);
  let setupLog = $state<string[]>([]);

  /** The app works the command out from the tool's help, tries it and lists what it made. */
  async function setUp() {
    settingUp = true;
    setupLog = [];
    trial = null;
    tryError = '';
    try {
      if (!(await save(true))) return;
      // no ready-made container: build one from the tool's repository first
      if (!f.container.trim() && tool?.build) {
        let b = await post<Job>(`/api/tools/${encodeURIComponent(id)}/build`);
        while (b.status === 'running') {
          setupLog = b.log;
          await new Promise((r) => setTimeout(r, 1500));
          b = await get<Job>(`/api/jobs/${b.id}`);
        }
        setupLog = b.log.map((l) => l.replace(/^Error: /, ''));
        if (b.status === 'failed') {
          toasts.show('The container could not be built. The last lines of the build are shown.', 'error');
          return;
        }
        await load();
      }
      let j = await post<Job>(`/api/tools/${encodeURIComponent(id)}/setup`, { reads_folder: readsFolder });
      while (j.status === 'running') {
        setupLog = j.log;
        await new Promise((r) => setTimeout(r, 1200));
        j = await get<Job>(`/api/jobs/${j.id}`);
      }
      setupLog = j.log.map((l) => l.replace(/^Error: /, ''));
      const r = j.result as { ready: boolean; tried: boolean; trial?: Trial } | undefined;
      await load();
      trial = r?.trial ?? null;
      if (j.status === 'failed') toasts.show(setupLog.at(-1) ?? 'The app could not set this tool up.', 'error');
      else if (r?.ready) toasts.show(`${f.name} is set up and ready to use in workflows.`);
      else toasts.show('The command is written. Check it, try it, then switch on Ready to use.', 'info');
    } catch (e) {
      toasts.show(errorText(e), 'error');
    } finally {
      settingUp = false;
    }
  }
</script>

<PageHeader title={tool ? `Edit ${f.name || tool.name}` : 'Edit a tool'} crumbs={[{ label: 'Tools', href: href('tools') }]}>
  {#snippet actions()}
    {#if tool}
      <Button variant="danger" icon="trash" onclick={() => (deleteOpen = true)}>Remove</Button>
      <Button variant={f.ready ? 'primary' : 'secondary'} icon="check" busy={saving} onclick={() => save()}>Save</Button>
    {/if}
  {/snippet}
</PageHeader>

{#if missing}
  <div class="px-8 pt-7 max-[720px]:px-4">
    <p class="card m-0 flex items-center gap-3 p-5 text-sm"><Icon name="info" /> {missing}</p>
  </div>
{:else if tool}
  <div class="grid w-full max-w-[1320px] grid-cols-[minmax(0,1fr)_370px] items-start gap-5 px-8 pt-7 pb-10 max-[1150px]:grid-cols-1 max-[720px]:px-4">
    <div class="flex min-w-0 flex-col gap-5">
      <section class="card flex flex-wrap items-center gap-4 p-5 {f.ready ? '' : 'border-warn-line bg-warn-soft'}">
        <span class="flex min-w-0 flex-[1_1_320px] flex-col gap-1 text-sm">
          <span class="font-semibold">{f.ready ? 'Ready to use in workflows' : 'Not ready yet'}</span>
          <span class="text-[13px] text-ink-2">
            {#if f.ready}
              Workflows can run this tool. Switch this off while you are still changing it.
            {:else}
              Let the app set it up, or write the command yourself and try it on one sample. Then switch this on and save. Until then workflows refuse to run it.
              Everything here is kept on this computer only{tool.github ? `: nothing changes in ${tool.github.repo} on GitHub` : ''}.
            {/if}
          </span>
        </span>
        <span class="flex items-center gap-2 text-sm font-semibold"><Toggle label="Ready to use in workflows" bind:checked={f.ready} /> Ready to use</span>
      </section>

      {#if !f.ready}
        <section class="card flex flex-col gap-3 border-brand p-5">
          <h2 class="m-0 text-base font-semibold">Let the app set it up</h2>
          <p class="m-0 text-[13px] text-ink-2">
            The app reads the tool's own help, writes the command, tries it on one sample and lists the result files it finds.
            This works for tools with the usual options. If it doesn't fit this tool, the command below is yours to change.
          </p>
          <div class="flex flex-wrap items-end gap-2">
            <label class="flex min-w-[260px] flex-1 flex-col gap-1.5 text-[13px] font-semibold">
              A folder to try it on: reads, or FASTA files for a tool that takes an assembly (the first sample is used)
              <input class="input" bind:value={readsFolder} placeholder="Choose a folder" />
            </label>
            <Button icon="folder" onclick={() => (pickerOpen = true)}>Choose</Button>
            <Button variant="primary" icon="play" busy={settingUp} disabled={!f.container.trim() && !tool.build} onclick={setUp}>Set it up for me</Button>
          </div>
          {#if !f.container.trim()}
            <span class="text-xs text-warn-ink">
              {tool.build
                ? 'No ready-made container was found for this tool, so the app builds one first. That takes several minutes, once.'
                : 'No container was found for this tool: open More options and fill in its container address first.'}
            </span>
          {/if}
          {#if setupLog.length}
            <ul class="m-0 flex list-none flex-col gap-1 rounded-lg bg-surface-2 p-3 text-[13px] text-ink-2">
              {#each setupLog.slice(-12) as line}<li class="break-words">{line}</li>{/each}
              {#if settingUp}<li class="text-muted">Working…</li>{/if}
            </ul>
          {/if}
        </section>
      {/if}

      <details class="order-last rounded-2xl border border-line bg-surface" bind:open={moreOpen}>
        <summary class="cursor-pointer px-5 py-4 text-base font-semibold">
          More options <span class="text-[13px] font-normal text-muted">· name, container, inputs, outputs, settings (filled in for you; change them only if you need to)</span>
        </summary>
        <div class="flex flex-col gap-5 border-t border-line p-5">
      <section class="card flex flex-col gap-4 p-5">
        <h2 class="m-0 text-base font-semibold">About the tool</h2>
        <div class="grid grid-cols-[repeat(auto-fit,minmax(220px,1fr))] gap-3.5">
          <label class="flex flex-col gap-1.5 text-[13px] font-semibold">Name<input class="input" bind:value={f.name} /></label>
          <label class="flex flex-col gap-1.5 text-[13px] font-semibold">Version<input class="input" bind:value={f.version} placeholder="1.2.3" /></label>
          <label class="flex flex-col gap-1.5 text-[13px] font-semibold">Group on the Tools page<input class="input" bind:value={f.category} placeholder="Typing" /></label>
          <label class="flex flex-col gap-1.5 text-[13px] font-semibold">Website<input class="input" bind:value={f.homepage} placeholder="https://" /></label>
        </div>
        <label class="flex flex-col gap-1.5 text-[13px] font-semibold">What it does<textarea class="input min-h-[64px]" bind:value={f.description}></textarea></label>
      </section>

      <section class="card flex flex-col gap-4 p-5">
        <h2 class="m-0 text-base font-semibold">Where it runs</h2>
        <label class="flex flex-col gap-1.5 text-[13px] font-semibold">
          Container
          <input class="input font-mono text-[13px]" bind:value={f.container} placeholder="quay.io/biocontainers/mytool:1.2.3--h1234_0" />
          <span class="font-normal text-muted">The address of a container image with the tool installed, at one exact version. Most tools have one on BioContainers (quay.io/biocontainers).</span>
        </label>
        <div class="grid grid-cols-[repeat(auto-fit,minmax(180px,1fr))] gap-3.5">
          <label class="flex flex-col gap-1.5 text-[13px] font-semibold">
            Runs
            <select class="input" bind:value={f.scope}>
              <option value="per_sample">Once for each sample</option>
              <option value="all_samples">Once, on every sample together</option>
            </select>
          </label>
          <label class="flex flex-col gap-1.5 text-[13px] font-semibold">CPU threads<input class="input" type="number" min="1" bind:value={f.cpus} /></label>
          <label class="flex flex-col gap-1.5 text-[13px] font-semibold">Memory (GB)<input class="input" type="number" min="1" bind:value={f.memory_gb} /></label>
        </div>
        <div class="flex flex-col gap-2 rounded-xl bg-surface-2 p-4">
          <span class="text-[13px] font-semibold">Not sure what to type in the command? Read the tool's own help.</span>
          <div class="flex flex-wrap items-end gap-2">
            <label class="flex min-w-[200px] flex-1 flex-col gap-1.5 text-[13px] font-semibold">
              Program name, as typed in a terminal (add a sub-command to see its options: mytool profile)
              <input class="input font-mono text-[13px]" bind:value={f.program} placeholder="mytool" />
            </label>
            <Button icon="file" busy={helpBusy} disabled={!f.container.trim() || !f.program.trim()} onclick={showHelp}>Show the tool's help</Button>
          </div>
          {#if helpBusy}<span class="text-xs text-muted">Getting the container if it is not here yet: the first time can take a few minutes.</span>{/if}
          {#if helpText}<pre class="m-0 max-h-[340px] overflow-auto rounded-lg border border-line bg-surface p-3 text-[12px] leading-snug whitespace-pre-wrap">{helpText}</pre>{/if}
        </div>
      </section>

      <section class="card flex flex-col gap-3 p-5">
        <h2 class="m-0 text-base font-semibold">What it takes in</h2>
        <p class="m-0 text-[13px] text-muted">Each input is a file from the reads or from an earlier step. In the workflow builder you connect it.</p>
        {#each inputs as i, n (n)}
          <div class="flex flex-wrap items-end gap-2 border-t border-line pt-3">
            <label class="flex w-[150px] flex-col gap-1 text-xs font-semibold">Name<input class="input font-mono text-[13px]" bind:value={i.name} placeholder="contigs" /></label>
            <label class="flex w-[130px] flex-col gap-1 text-xs font-semibold">
              Kind of file
              <select class="input" bind:value={i.type}>
                {#each FILE_TYPES.includes(i.type) ? FILE_TYPES : [i.type, ...FILE_TYPES] as k}<option value={k}>{k === 'reads' ? 'reads (a pair)' : k}</option>{/each}
              </select>
            </label>
            <label class="flex min-w-[180px] flex-1 flex-col gap-1 text-xs font-semibold">What it is<input class="input" bind:value={i.description} /></label>
            <span class="flex h-11 items-center gap-1.5 text-xs font-semibold"><Toggle small label="Input {i.name} is optional" bind:checked={i.optional} /> Optional</span>
            <Button variant="quiet" icon="trash" label="Remove the input {i.name}" class="!text-danger" onclick={() => inputs.splice(n, 1)} />
          </div>
        {/each}
        <div><Button size="sm" icon="plus" onclick={() => inputs.push({ name: '', type: 'fasta', optional: false, description: '' })}>Add an input</Button></div>
      </section>

      <section class="card flex flex-col gap-3 p-5">
        <h2 class="m-0 text-base font-semibold">What it makes</h2>
        <p class="m-0 text-[13px] text-muted">
          Each output is a file the command writes. The path says where: start it with <code>out/</code> and write the sample's name as
          <code>{'{sample}'}</code>. Later steps can be connected to an output, and it shows on the run's page.
        </p>
        {#each outputs as o, n (n)}
          <div class="flex flex-wrap items-end gap-2 border-t border-line pt-3">
            <label class="flex w-[150px] flex-col gap-1 text-xs font-semibold">Name<input class="input font-mono text-[13px]" bind:value={o.name} placeholder="report" /></label>
            <label class="flex w-[110px] flex-col gap-1 text-xs font-semibold">
              Kind of file
              <select class="input" bind:value={o.type}>
                {#each (FILE_TYPES.includes(o.type) ? FILE_TYPES : [o.type, ...FILE_TYPES]).filter((k) => k !== 'reads' || o.type === 'reads') as k}<option value={k}>{k}</option>{/each}
              </select>
            </label>
            <label class="flex min-w-[220px] flex-1 flex-col gap-1 text-xs font-semibold">Path<input class="input font-mono text-[13px]" bind:value={o.path} placeholder={'out/{sample}.report.tsv'} /></label>
            <label class="flex min-w-[160px] flex-1 flex-col gap-1 text-xs font-semibold">What it is<input class="input" bind:value={o.description} /></label>
            <span class="flex h-11 items-center gap-1.5 text-xs font-semibold"><Toggle small label="Output {o.name} is not always made" bind:checked={o.optional} /> Not always made</span>
            <Button variant="quiet" icon="trash" label="Remove the output {o.name}" class="!text-danger" onclick={() => outputs.splice(n, 1)} />
          </div>
        {/each}
        <div><Button size="sm" icon="plus" onclick={() => outputs.push({ name: '', type: 'tsv', path: perSample ? 'out/{sample}.report.tsv' : 'out/report.tsv', optional: false, description: '' })}>Add an output</Button></div>
      </section>

      <section class="card flex flex-col gap-3 p-5">
        <h2 class="m-0 text-base font-semibold">Settings people can change <span class="font-normal text-muted">(optional)</span></h2>
        <p class="m-0 text-[13px] text-muted">A setting shows in the workflow builder when this tool's step is selected. The command gets its value as <code>$S_name</code>.</p>
        {#each settings as s, n (n)}
          <div class="flex flex-wrap items-end gap-2 border-t border-line pt-3">
            <label class="flex w-[140px] flex-col gap-1 text-xs font-semibold">Name<input class="input font-mono text-[13px]" bind:value={s.name} placeholder="min_depth" /></label>
            <label class="flex min-w-[160px] flex-1 flex-col gap-1 text-xs font-semibold">Shown as<input class="input" bind:value={s.label} placeholder="Minimum depth" /></label>
            <label class="flex w-[150px] flex-col gap-1 text-xs font-semibold">
              Kind
              <select class="input" bind:value={s.type}>{#each SETTING_TYPES as [k, label]}<option value={k}>{label}</option>{/each}</select>
            </label>
            <label class="flex w-[130px] flex-col gap-1 text-xs font-semibold">
              Starts as
              {#if s.type === 'boolean'}
                <select class="input" bind:value={s.value}><option value="false">Off</option><option value="true">On</option></select>
              {:else}
                <input class="input" bind:value={s.value} />
              {/if}
            </label>
            {#if s.type === 'select'}
              <label class="flex min-w-[180px] flex-1 flex-col gap-1 text-xs font-semibold">Choices, with commas between<input class="input" bind:value={s.options} placeholder="fast, careful" /></label>
            {/if}
            <Button variant="quiet" icon="trash" label="Remove the setting {s.name}" class="!text-danger" onclick={() => settings.splice(n, 1)} />
          </div>
        {/each}
        <div><Button size="sm" icon="plus" onclick={() => settings.push({ name: '', label: '', type: 'string', value: '', description: '', options: '' })}>Add a setting</Button></div>
      </section>
        </div>
      </details>

      <section class="card flex flex-col gap-3 p-5">
        <h2 class="m-0 text-base font-semibold">The command</h2>
        <p class="m-0 text-[13px] text-muted">
          What you would type in a terminal to run the tool on one sample, with the names from the guide in place of real file names.
          More than one line is fine: they run one after another.
        </p>
        <label class="sr-only" for="tool-command">Command</label>
        <textarea
          id="tool-command"
          bind:this={commandBox}
          class="input min-h-[190px] font-mono text-[13px] leading-relaxed"
          spellcheck="false"
          bind:value={f.command}
          placeholder={example}
        ></textarea>
        <label class="flex flex-col gap-1.5 text-[13px] font-semibold">
          Command that prints the tool's version <span class="font-normal text-muted">(written into every run's record)</span>
          <input class="input font-mono text-[13px]" bind:value={f.version_command} placeholder="mytool --version" />
        </label>
      </section>

      <section class="card flex flex-col gap-3 p-5">
        <h2 class="m-0 text-base font-semibold">Try it on one sample</h2>
        <p class="m-0 text-[13px] text-muted">Saves your changes, runs the command once and shows what it made. Nothing is added to your runs.</p>
        {#if needsReads || needsFasta}
          <div class="flex flex-wrap items-end gap-2">
            <label class="flex min-w-[260px] flex-1 flex-col gap-1.5 text-[13px] font-semibold">
              {needsReads ? 'A folder of reads (its first sample is used)' : 'A folder of FASTA files (the first one is used)'}
              <input class="input" bind:value={readsFolder} placeholder="Choose a folder" />
            </label>
            <Button icon="folder" onclick={() => (pickerOpen = true)}>Choose</Button>
          </div>
        {/if}
        {#each fileInputs as i (i.name)}
          <label class="flex flex-col gap-1.5 text-[13px] font-semibold">
            A {i.type} file for the input "{i.name}"{i.optional ? ' (optional)' : i.type === 'fasta' ? ' (or leave empty to use the folder above)' : ''}
            <input class="input font-mono text-[13px]" bind:value={tryFiles[i.name]} placeholder="The full path of a file, for example from an earlier run's Results folder" />
          </label>
        {/each}
        <div><Button icon="play" busy={trying} disabled={!f.command.trim()} onclick={tryIt}>Try on one sample</Button></div>
        {#if trying}<span class="text-xs text-muted">Running. Getting the container the first time can take a few minutes.</span>{/if}
        {#if tryError}
          <p class="m-0 flex items-start gap-2 rounded-lg bg-danger-soft px-3 py-2 text-[13px] font-semibold text-danger"><Icon name="alert" /> {tryError}</p>
        {/if}
        {#if trial}
          {#if trial.exit === 0}
            <p class="m-0 flex items-center gap-2 rounded-lg bg-ok-soft px-3 py-2 text-[13px] font-semibold text-ok-ink"><Icon name="check" /> The command ran to the end.</p>
          {/if}
          {#if trial.outputs.length}
            <div class="overflow-auto rounded-xl border border-line">
              <table class="data">
                <thead><tr><th>Output</th><th>Looked for</th><th>Found</th></tr></thead>
                <tbody>
                  {#each trial.outputs as o (o.name)}
                    <tr>
                      <td class="font-semibold text-ink">{o.name}</td>
                      <td class="font-mono text-[12px]">{o.pattern}</td>
                      <td>
                        {#if o.files.length}
                          <span class="text-ok-ink">{o.files.join(', ')}</span>
                        {:else}
                          <span class={o.optional ? 'text-muted' : 'font-semibold text-danger'}>{o.optional ? 'not made this time' : 'not made: check the path and the command'}</span>
                        {/if}
                      </td>
                    </tr>
                  {/each}
                </tbody>
              </table>
            </div>
          {/if}
          {#if unlisted.length}
            <div class="flex flex-col gap-1.5 text-[13px]">
              <span class="font-semibold">Files it made that no output lists</span>
              <span class="text-muted">Press Keep on the ones you want saved with the results and shown on a run's page.</span>
              {#each unlisted.slice(0, 30) as x (x.path)}
                <span class="flex flex-wrap items-center gap-2">
                  <span class="font-mono text-[12px] break-all">{x.path}</span>
                  <span class="text-xs text-muted">{size(x.size)}</span>
                  <Button size="sm" variant="quiet" icon="plus" onclick={() => keep(x.path)}>Keep</Button>
                </span>
              {/each}
              {#if unlisted.length > 30}<span class="text-xs text-muted">…and {unlisted.length - 30} more</span>{/if}
            </div>
          {/if}
          {#each trial.files.filter((x) => x.preview && trial!.outputs.some((o) => o.files.includes(x.path))) as x (x.path)}
            <details class="rounded-xl border border-line" open>
              <summary class="cursor-pointer px-3 py-2 text-[13px] font-semibold">{x.path} <span class="font-normal text-muted">as it will show on a run's page (the first lines)</span></summary>
              {#if fileKind(x.path) === 'table'}
                {@const t = asTable(x.path, x.preview ?? '')}
                <div class="max-h-[260px] overflow-auto border-t border-line">
                  <table class="data">
                    <thead><tr>{#each t.header as h}<th>{h}</th>{/each}</tr></thead>
                    <tbody>{#each t.rows.slice(0, 12) as row}<tr>{#each row as c}<td>{c}</td>{/each}</tr>{/each}</tbody>
                  </table>
                </div>
              {:else}
                <pre class="m-0 max-h-[260px] overflow-auto border-t border-line p-3 text-[12px] whitespace-pre-wrap">{x.preview}</pre>
              {/if}
            </details>
          {/each}
          {#if trial.log.trim()}
            <details class="rounded-xl border border-line" open={trial.exit !== 0}>
              <summary class="cursor-pointer px-3 py-2 text-[13px] font-semibold">What the command printed</summary>
              <pre class="m-0 max-h-[300px] overflow-auto border-t border-line p-3 text-[12px] whitespace-pre-wrap">{trial.log}</pre>
            </details>
          {/if}
        {/if}
      </section>
    </div>

    <aside aria-label="Guide" class="sticky top-24 flex flex-col gap-4 max-[1150px]:static">
      <section class="card flex flex-col gap-2.5 p-5">
        <h2 class="m-0 text-base font-semibold">{todo ? `Still to do (${todo})` : 'Everything checks out'}</h2>
        <ul class="m-0 flex list-none flex-col gap-2 p-0 text-[13px]">
          {#each checks as c}
            <li class="flex items-start gap-2 {c.ok ? 'text-ok-ink' : 'text-warn-ink'}">
              <span class="mt-0.5 shrink-0"><Icon name={c.ok ? 'check' : 'alert'} size={15} /></span>
              <span>{c.text}</span>
            </li>
          {/each}
        </ul>
      </section>

      <section class="card flex flex-col gap-2.5 p-5">
        <h2 class="m-0 text-base font-semibold">Names to use in the command</h2>
        <p class="m-0 text-[13px] text-muted">The app fills these in for each sample. Press one to put it in the command.</p>
        <ul class="m-0 flex list-none flex-col gap-1.5 p-0">
          {#each names as n (n.name)}
            <li class="flex flex-wrap items-baseline gap-2 text-[13px]">
              <button class="cursor-pointer rounded-md border border-line-strong bg-surface-2 px-1.5 py-0.5 font-mono text-[12px] text-ink hover:bg-brand-softer" onclick={() => insert(n.name)}>{n.name}</button>
              <span class="text-ink-2">{n.what}</span>
            </li>
          {/each}
        </ul>
      </section>

      <section class="card flex flex-col gap-2.5 p-5 text-[13px] text-ink-2">
        <h2 class="m-0 text-base font-semibold text-ink">Three rules</h2>
        <ol class="m-0 flex flex-col gap-2 pl-5">
          <li>Write every result file into the folder <code>out/</code>. Only files there are saved with the run.</li>
          <li>
            Put the sample's name in the file name, so samples don't overwrite each other: <code>out/$SAMPLE.report.tsv</code>. In an
            output's path the same file is written <code>out/{'{sample}'}.report.tsv</code>.
          </li>
          <li>Keep the double quotes around names: <code>"$IN_reads_1"</code>.</li>
        </ol>
        <span class="font-semibold text-ink">The shape of a command</span>
        <pre class="m-0 overflow-auto rounded-lg bg-surface-2 p-3 text-[12px] whitespace-pre-wrap">{example}</pre>
        <span class="text-muted">The tool's own options differ: Show the tool's help lists them.</span>
      </section>

      <section class="card flex flex-col gap-2 p-5 text-[13px] text-ink-2">
        <h2 class="m-0 text-base font-semibold text-ink">How results show on a run's page</h2>
        <ul class="m-0 flex flex-col gap-1.5 pl-5">
          <li><strong>.tsv, .csv, .tab</strong>: a table, every sample together.</li>
          <li><strong>.txt, .log, .json</strong>: as text.</li>
          <li><strong>.html</strong>: opens as a page.</li>
          <li>Anything else: a download.</li>
        </ul>
        <span class="text-muted">A table with one heading line and one line per result reads best.</span>
      </section>
    </aside>
  </div>

  <FolderPicker bind:open={pickerOpen} start={readsFolder} title="Choose a folder of reads or FASTA files" onpick={(p) => (readsFolder = p)} />

  <Dialog bind:open={deleteOpen} title={`Remove ${f.name} from this computer?`}>
    Its definition and any extra versions of it go. {tool.github ? `Nothing changes in ${tool.github.repo} on GitHub: you can add it again later.` : 'Runs that already used it keep their own copy.'}
    {#snippet actions()}
      <Button onclick={() => (deleteOpen = false)}>Keep it</Button>
      <Button variant="danger-solid" onclick={remove}>Yes, remove it</Button>
    {/snippet}
  </Dialog>
{:else}
  <p class="m-0 px-8 pt-7 text-sm text-muted">Loading…</p>
{/if}
