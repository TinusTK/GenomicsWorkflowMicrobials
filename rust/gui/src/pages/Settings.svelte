<script lang="ts">
  import PageHeader from '../lib/PageHeader.svelte';
  import RemoveEverything from '../lib/RemoveEverything.svelte';
  import Button from '../lib/ui/Button.svelte';
  import Icon from '../lib/ui/Icon.svelte';
  import { backendStatus, colimaSize, invoke, isDesktop, openFolder, openLink } from '../lib/desktop';
  import { errorText, toasts } from '../lib/toast.svelte';
  import { live } from '../lib/live.svelte';

  live.refreshHardware();
  const hw = $derived(live.hardware);
  const engine = $derived(hw?.docker ? `Docker ${hw.docker}${hw.wsl ? ' (inside WSL)' : ''}` : hw?.apptainer ? hw.apptainer : null);

  // ---------------------------------------------------------------- setting up Docker (Mac, Linux)
  const mac = $derived(hw?.platform === 'macos');
  /** What went wrong, from the engine's one-line reason. */
  const trouble = $derived.by(() => {
    const why = hw?.docker_error ?? '';
    return why.includes('not allowed') ? 'denied' : why.includes('not running') ? 'stopped' : 'missing';
  });
  const colima = $derived(colimaSize(hw));
  const colimaStart = $derived(`colima start --cpu ${colima.cpus} --memory ${colima.memory}`);
  /** What to do, and the lines to paste into a terminal to do it by hand. */
  const setup = $derived.by(() => {
    if (mac) {
      if (trouble === 'stopped')
        return {
          text: 'Docker is there but has not been started. With Colima: open Terminal (in Applications › Utilities), paste this line and press Return. With Docker Desktop: open Docker Desktop. Then press Check again.',
          lines: [colimaStart],
          button: false,
        };
      return {
        text: isDesktop()
          ? 'The tools run in Docker, which on a Mac comes with Colima (free). Press Set up Docker: a Terminal window installs it and shows the progress (a few minutes; it may ask for your Mac password), then press Check again.'
          : 'The tools run in Docker, which on a Mac comes with Colima (free). Open Terminal (in Applications › Utilities), paste these lines and press Return, then press Check again.',
        lines: ['brew install colima docker', colimaStart],
        button: true,
        notes: [
          'The first line needs Homebrew (brew.sh). The button installs it for you when it is missing.',
          'Colima does not start by itself when the Mac starts: press Start in the side panel when you open the app.',
        ],
      };
    }
    if (trouble === 'denied')
      return {
        text: 'Docker is running, but your user may not use it yet. Log out and in again (or restart the computer). If it stays, open a terminal, paste this line, then log out and in.',
        lines: ['sudo usermod -aG docker $USER'],
        button: false,
      };
    if (trouble === 'stopped')
      return { text: 'Docker is installed but has not been started. Open a terminal, paste this line, then press Check again.', lines: ['sudo systemctl enable --now docker'], button: false };
    return {
      text: isDesktop()
        ? 'The tools run in Docker. Press Set up Docker: a terminal window installs it and shows the progress (Ubuntu and Debian; it asks for your password). Then log out and in again and press Check again.'
        : 'The tools run in Docker. Open a terminal and paste these lines (Ubuntu and Debian), then log out and in again and press Check again.',
      lines: ['sudo apt-get install -y docker.io', 'sudo systemctl enable --now docker', 'sudo usermod -aG docker $USER'],
      button: true,
      notes: ['On another Linux, install Docker Engine the way docs.docker.com/engine/install says for it. Apptainer works too.'],
    };
  });
  async function copyLines(lines: string[]) {
    try {
      await navigator.clipboard.writeText(lines.join('\n'));
      toasts.show('Copied: paste it into the terminal.', 'info');
    } catch {
      toasts.show('Could not copy: select the lines and copy them yourself.', 'error');
    }
  }

  // ---------------------------------------------------------------- memory for the tools
  // 4 GB stay for the computer, the rest is for the tools; changed here, not in a file
  interface MemoryInfo {
    platform: string;
    total_gb: number;
    default_gb: number;
    /** what WSL (Windows) or Colima (Mac) is set to; nothing on Linux or when not set */
    current_gb: number | null;
  }
  let mem = $state<MemoryInfo | null>(null);
  let memChoice = $state(0);
  let memBusy = $state(false);
  async function loadMemory() {
    if (!isDesktop()) return;
    mem = await invoke<MemoryInfo>('memory_info').catch(() => null);
    if (mem) memChoice = mem.current_gb ?? mem.default_gb;
  }
  loadMemory();
  /** A number of GB, or null for the default (all but 4 GB). Restarts the tools' machine. */
  async function applyMemory(gb: number | null) {
    memBusy = true;
    try {
      await invoke('memory_set', { gb });
      toasts.show(`The tools now have ${gb ?? mem?.default_gb} GB of memory.`, 'info');
    } catch (e) {
      toasts.show(errorText(e), 'error', undefined, 12000);
    }
    await loadMemory();
    await live.refreshUsage();
    await live.refreshHardware();
    memBusy = false;
  }
  const memMachine = $derived(mem?.platform === 'windows' ? 'the Linux side of Windows (WSL)' : 'Colima');

  // the Linux distribution the desktop app uses (for \\wsl.localhost paths)
  let distro = $state('Ubuntu-24.04');
  backendStatus().then((s) => s && (distro = s.distro));

  /** How Windows Explorer reaches a WSL path: /mnt/c/x -> C:\x, /root/x -> \\wsl.localhost\<distro>\root\x */
  function windowsPath(p: string) {
    const m = p.match(/^\/mnt\/([a-z])(\/.*)?$/);
    if (m) return `${m[1].toUpperCase()}:${(m[2] ?? '').replaceAll('/', '\\')}`;
    return `\\\\wsl.localhost\\${distro}${p.replaceAll('/', '\\')}`;
  }

  const places = $derived(
    hw
      ? [
          { title: 'Runs', what: 'One folder per run: results, run details and a README. Safe to delete a run folder.', path: hw.runs_folder, linux: false },
          { title: 'Your data', what: 'Your own workflows, the tools you added and extra tool versions.', path: hw.data_folder, linux: true },
          { title: 'Reference databases', what: 'Kraken2, CheckM data and other databases you installed.', path: hw.databases_folder, linux: true },
          { title: 'Working files', what: 'Big temporary files while a run works; cleared after a successful run.', path: hw.work_folder, linux: true },
        ].map((p) => ({ ...p, linux: !p.path.startsWith('/mnt/') }))
      : [],
  );

  // closing the window keeps the desktop app in the system tray
  let inTray = $state(false);
  backendStatus().then((s) => s && (inTray = !!s.tray));

  // ---------------------------------------------------------------- updates
  // looked up at every start; the button looks again
  const update = $derived(live.update);
  let checking = $state(false);
  async function checkUpdate() {
    checking = true;
    try {
      await live.checkUpdate();
    } catch (e) {
      toasts.show(errorText(e), 'error');
    } finally {
      checking = false;
    }
  }

  // ---------------------------------------------------------------- report an issue
  const ISSUES = 'https://github.com/TinusTK/GenomicsWorkflowMicrobials/issues/new';
  const EMAIL = 't.k.business@outlook.com';
  /** An email with the details a report needs already filled in. */
  const mail = $derived(
    `mailto:${EMAIL}?subject=${encodeURIComponent('Genomics Workflow Microbials: issue')}&body=${encodeURIComponent(
      `What I was doing:

What happened:

Tool or workflow (and its GitHub link, if a tool would not install):

---
App version: ${hw?.app_version ?? ''}
System: ${hw?.os ?? ''}${hw?.wsl ? ' (WSL)' : ''}
Containers: ${engine ?? 'none found'}
`,
    )}`,
  );
</script>

<PageHeader title="Settings">
  {#snippet actions()}
    <Button icon="refresh" onclick={() => live.refreshHardware()}>Check again</Button>
  {/snippet}
</PageHeader>

<div class="flex w-full max-w-[1000px] flex-col gap-5 px-8 pt-7 pb-10 max-[720px]:px-4">
  {#if hw}
    <section class="card flex flex-col gap-3 p-5">
      <h2 class="m-0 text-base font-semibold">Containers (where the tools run)</h2>
      {#if engine}
        <p class="m-0 flex items-center gap-2 text-sm text-ok-ink"><Icon name="check" /> Ready: {engine}</p>
      {:else}
        <p class="m-0 flex items-center gap-2 text-sm font-semibold text-danger"><Icon name="alert" /> {hw.docker_error ?? 'No container engine found'}</p>
        <div class="flex flex-col gap-3 rounded-xl bg-surface-2 px-4 py-3 text-[13px] leading-relaxed text-ink-2">
          {#if hw.wsl}
            <span>
              Docker runs inside the Linux side of Windows (WSL). {isDesktop()
                ? 'Press Set up Docker: a window shows the progress (a few minutes), then press Check again.'
                : 'Open the Ubuntu app and type sudo apt-get install -y docker.io && sudo service docker start, then press Check again.'}
            </span>
            {#if isDesktop()}
              <div><Button variant="primary" icon="download" onclick={() => invoke('setup_docker').catch((e) => toasts.show(String(e), 'error'))}>Set up Docker</Button></div>
            {/if}
          {:else}
            <span>{setup.text}</span>
            {#if setup.button && isDesktop()}
              <div>
                <Button variant="primary" icon="download" onclick={() => invoke('setup_docker', { cpus: colima.cpus, memory: colima.memory }).catch((e) => toasts.show(String(e), 'error'))}>Set up Docker</Button>
              </div>
              <span>Or do it by hand: open {mac ? 'Terminal (in Applications › Utilities)' : 'a terminal'}, paste these lines and press {mac ? 'Return' : 'Enter'}.</span>
            {/if}
            <div class="flex flex-wrap items-start gap-3">
              <pre class="m-0 min-w-0 flex-1 overflow-x-auto rounded-lg bg-surface px-3 py-2 font-mono text-[12.5px] leading-relaxed text-ink select-text">{setup.lines.join('\n')}</pre>
              <Button onclick={() => copyLines(setup.lines)}>Copy</Button>
            </div>
            {#each setup.notes ?? [] as note (note)}
              <span>{note}</span>
            {/each}
          {/if}
        </div>
        {#if hw.docker_detail}
          <details class="text-[13px] text-ink-2">
            <summary class="cursor-pointer select-none">Details</summary>
            <pre class="m-0 mt-2 overflow-x-auto rounded-lg bg-surface-2 px-3 py-2 font-mono text-[12px] leading-relaxed whitespace-pre-wrap select-text">{hw.docker_detail}</pre>
          </details>
        {/if}
      {/if}
    </section>

    {#if Math.round(hw.memory_gb) < 16}
      <section class="card flex gap-3 p-5 text-[13px] leading-relaxed text-ink-2">
        <span class="text-warn"><Icon name="alert" /></span>
        <span>
          {hw.wsl ? "The Linux side of Windows (WSL), where the tools run, has" : "This computer has"} {Math.round(hw.memory_gb)} GB of memory. The app needs at least 16 GB: with less, some tools may not work
          (steps such as the assembly can run out of memory and fail). More than 16 GB is best.
          {#if mac}On a Mac the tools run in a small Linux machine (Colima) that gets only part of the Mac's memory.{/if}
        </span>
      </section>
    {:else if Math.round(hw.memory_gb) === 16}
      <section class="card flex gap-3 p-5 text-[13px] leading-relaxed text-ink-2">
        <span class="text-info"><Icon name="info" /></span>
        <span>{hw.wsl ? "The Linux side of Windows (WSL), where the tools run, has" : "This computer has"} 16 GB of memory, the least the app needs. Some tools may still not work with it: more than 16 GB is best.</span>
      </section>
    {/if}

    <section class="card grid grid-cols-[repeat(auto-fit,minmax(200px,1fr))] gap-3 p-5 text-sm">
      <div><span class="label-caps block">CPU threads</span><span class="text-xl font-bold">{hw.cpus}</span></div>
      <div><span class="label-caps block">Memory</span><span class="text-xl font-bold">{Math.round(hw.memory_gb)} GB</span></div>
      <div><span class="label-caps block">Free disk (working files)</span><span class="text-xl font-bold">{hw.disk_free_gb.work != null ? `${Math.round(hw.disk_free_gb.work)} GB` : '?'}</span></div>
      <div><span class="label-caps block">System</span><span class="font-semibold">{hw.os ?? ''}</span></div>
    </section>

    {#if mem}
      <section class="card flex flex-col gap-3 p-5">
        <h2 class="m-0 text-base font-semibold">Memory for the tools</h2>
        <p class="m-0 text-[13px] leading-relaxed text-ink-2">
          {#if mem.platform === 'windows'}
            On Windows the tools run on the Linux side of Windows (WSL).
            {#if mem.current_gb}It is set to <strong>{mem.current_gb} GB</strong> of this computer's {mem.total_gb} GB.{:else}No amount is set yet, so Windows gives it half of this computer's {mem.total_gb} GB.{/if}
            By default 4 GB stay for Windows and the rest ({mem.default_gb} GB) is for the tools.
            {#if mem.current_gb && Math.abs(hw.memory_gb - mem.current_gb) > 1.5}
              Right now it still has {Math.round(hw.memory_gb)} GB: the new amount is used after Stop and Start.
            {/if}
          {:else if mem.platform === 'macos'}
            On a Mac the tools run in a small Linux machine (Colima).
            {#if mem.current_gb}It is set to <strong>{mem.current_gb} GB</strong> of this Mac's {mem.total_gb} GB.{:else}It has not been made yet.{/if}
            By default 4 GB stay for the Mac and the rest ({mem.default_gb} GB) is for the tools.
          {:else}
            On Linux the tools use this computer's memory directly: runs may use up to <strong>{mem.default_gb} GB</strong> of its
            {mem.total_gb} GB, and 4 GB stay for the computer. There is nothing to set.
          {/if}
        </p>
        {#if mem.platform !== 'linux'}
          <div class="flex flex-wrap items-end gap-3">
            <label class="flex flex-col gap-1.5 text-[13px] font-semibold">
              Memory (GB)
              <input class="input w-[120px]" type="number" min="2" max={mem.total_gb} bind:value={memChoice} />
            </label>
            <span title={live.running > 0 ? 'A run is going: change the memory when it has finished' : ''}>
              <Button busy={memBusy} disabled={live.running > 0 || !memChoice || memChoice === mem.current_gb} onclick={() => applyMemory(memChoice)}>Apply</Button>
            </span>
            {#if mem.current_gb !== mem.default_gb}
              <Button variant="quiet" disabled={memBusy || live.running > 0} onclick={() => applyMemory(null)}>Back to {mem.default_gb} GB</Button>
            {/if}
          </div>
          <p class="m-0 text-xs leading-relaxed text-muted">
            Apply restarts {memMachine}, which takes a few seconds{mem.platform === 'windows' ? ' and also restarts anything else you run in WSL' : ''}.
            It can't be changed while a run is going.
          </p>
        {/if}
      </section>
    {:else if hw.wsl}
      <section class="card flex gap-3 p-5 text-[13px] leading-relaxed text-ink-2">
        <span class="text-info"><Icon name="info" /></span>
        <span>
          On Windows the tools run on the Linux side of Windows (WSL), which has {Math.round(hw.memory_gb)} GB of memory here.
          The desktop app sets and changes that amount in its Settings (4 GB stay for Windows, the rest is for the tools).
        </span>
      </section>
    {/if}

    <section class="card flex flex-col gap-3 p-5">
      <h2 class="m-0 text-base font-semibold">Where things are kept</h2>
      {#each places as p (p.path)}
        <div class="flex flex-wrap items-center gap-3 border-t border-line pt-3 text-sm first-of-type:border-0 first-of-type:pt-0">
          <span class="flex min-w-0 flex-[1_1_320px] flex-col gap-0.5">
            <span class="flex flex-wrap items-center gap-2 font-semibold">
              {p.title}
              {#if hw.wsl}
                <span class="rounded-full px-2 py-0.5 text-[11px] font-bold {p.linux ? 'bg-[#ede9fe] text-[#5b21b6]' : 'bg-info-soft text-info-ink'}">
                  {p.linux ? 'Linux side (WSL)' : 'Windows'}
                </span>
              {/if}
            </span>
            <span class="text-[13px] text-ink-2">{p.what}</span>
            <span class="font-mono text-[12px] break-all text-muted">{hw.wsl && !p.linux ? windowsPath(p.path) : p.path}</span>
            {#if p.linux && hw.wsl}
              <span class="font-mono text-[12px] break-all text-muted">In Windows: {windowsPath(p.path)}</span>
            {/if}
          </span>
          <Button size="sm" icon="folder" onclick={() => openFolder(p.path)}>Open</Button>
        </div>
      {/each}
      <p class="m-0 rounded-xl bg-surface-2 px-4 py-3 text-[13px] leading-relaxed text-ink-2">
        To delete a run, delete its folder in Runs (or in the app). Its working files on the Linux side are then cleared
        automatically. Don't delete the data folder by hand: it holds your own workflows and tools.
      </p>
    </section>

    <section class="card flex flex-col gap-3 p-5">
      <h2 class="m-0 text-base font-semibold">Updates</h2>
      <div class="flex flex-wrap items-center gap-3 text-sm">
        <span class="min-w-0 flex-[1_1_320px]">
          {#if !update}
            You have version {hw.app_version}. The app looks for a newer version each time it opens.
          {:else if update.newer}
            <strong>Version {update.latest} is out</strong> (you have {update.current}).
            {#if live.updating}
              {live.updatePercent == null ? 'Getting the update…' : live.updatePercent < 100 ? `Downloading the update: ${live.updatePercent}%` : 'Installing: the app closes and opens again by itself.'}
            {:else if isDesktop()}
              Update now downloads it, installs it over this version and opens the app again. Your workflows, tools, databases
              and runs are kept, and a run in progress carries on.
            {:else}
              Download the installer and run it: your workflows, tools, databases and runs are kept.
            {/if}
          {:else}
            <span class="flex items-center gap-2 text-ok-ink"><Icon name="check" /> You have the newest version ({update.current}).</span>
          {/if}
        </span>
        {#if update?.newer && isDesktop()}
          <Button variant="primary" icon="download" busy={live.updating} onclick={() => live.installUpdate()}>Update now</Button>
          <Button onclick={() => openLink(update!.url)}>Download page</Button>
        {:else if update?.newer}
          <Button variant="primary" icon="download" onclick={() => openLink(update!.url)}>Open the download page</Button>
        {/if}
        <Button icon="refresh" busy={checking} onclick={checkUpdate}>Check for updates</Button>
      </div>
      {#if update?.newer && update.whats_new?.length}
        <div class="flex flex-col gap-2 rounded-xl bg-surface-2 px-4 py-3 text-[13px] leading-relaxed text-ink-2">
          <span class="font-semibold text-ink">What is new in version {update.latest}</span>
          <ul class="m-0 flex list-disc flex-col gap-1.5 pl-5">
            {#each update.whats_new as n (n.title + n.text)}
              <li>{#if n.title}<strong class="text-ink">{n.title}.</strong>{' '}{/if}{n.text}</li>
            {/each}
          </ul>
        </div>
      {/if}
    </section>

    <section class="card flex flex-col gap-3 p-5">
      <h2 class="m-0 text-base font-semibold">Report an issue</h2>
      <p class="m-0 text-[13px] leading-relaxed text-ink-2">
        Something not working, or a tool that will not install or run? Tell us what you were doing and what happened, and for
        a tool add its GitHub link. Posting on GitHub needs a free GitHub account and lets others see the answer; an email
        needs no account.
      </p>
      <div class="flex flex-wrap gap-2.5">
        <Button icon="alert" onclick={() => openLink(ISSUES)}>Report on GitHub</Button>
        <Button icon="file" onclick={() => openLink(mail)}>Email it instead</Button>
      </div>
      <p class="m-0 text-xs text-muted">Email: {EMAIL}</p>
    </section>

    <RemoveEverything />

    {#if inTray}
      <section class="card flex gap-3 p-5 text-[13px] leading-relaxed text-ink-2">
        <span class="text-info"><Icon name="info" /></span>
        <span>
          Closing the window keeps the app in the system tray (the small icons next to the clock), so it opens again at once.
          To close it fully, right-click its icon there and choose <strong>Quit</strong>. Runs that are going carry on either way.
        </span>
      </section>
    {/if}
    <p class="m-0 text-xs text-muted">Genomics Workflow Microbials {hw.app_version}</p>
  {:else}
    <p class="m-0 text-sm text-muted">Checking…</p>
  {/if}
</div>
