<script lang="ts">
  import PageHeader from '../lib/PageHeader.svelte';
  import Button from '../lib/ui/Button.svelte';
  import Icon from '../lib/ui/Icon.svelte';
  import { get } from '../lib/api';
  import { backendStatus, invoke, isDesktop, openFolder, openLink } from '../lib/desktop';
  import { errorText, toasts } from '../lib/toast.svelte';
  import { live } from '../lib/live.svelte';

  live.refreshHardware();
  const hw = $derived(live.hardware);
  const engine = $derived(hw?.docker ? `Docker ${hw.docker}${hw.wsl ? ' (inside WSL)' : ''}` : hw?.apptainer ? hw.apptainer : null);

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
  interface Update {
    current: string;
    latest: string | null;
    newer: boolean;
    url: string;
  }
  let update = $state<Update | null>(null);
  let checking = $state(false);
  async function checkUpdate() {
    checking = true;
    try {
      update = await get<Update>('/api/update');
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
            Install Docker Engine (Linux), or Colima (Mac), then press Check again.
          {/if}
        </div>
      {/if}
    </section>

    <section class="card grid grid-cols-[repeat(auto-fit,minmax(200px,1fr))] gap-3 p-5 text-sm">
      <div><span class="label-caps block">CPU threads</span><span class="text-xl font-bold">{hw.cpus}</span></div>
      <div><span class="label-caps block">Memory</span><span class="text-xl font-bold">{Math.round(hw.memory_gb)} GB</span></div>
      <div><span class="label-caps block">Free disk (working files)</span><span class="text-xl font-bold">{hw.disk_free_gb.work != null ? `${Math.round(hw.disk_free_gb.work)} GB` : '?'}</span></div>
      <div><span class="label-caps block">System</span><span class="font-semibold">{hw.os ?? ''}</span></div>
    </section>

    {#if hw.wsl && hw.memory_gb < 32}
      <section class="card flex gap-3 p-5 text-[13px] leading-relaxed text-ink-2">
        <span class="text-info"><Icon name="info" /></span>
        <span>
          Windows gives its Linux side (WSL) half of the computer's memory by default: here {Math.round(hw.memory_gb)} GB.
          To give it more, make a file called <code>.wslconfig</code> in your Windows user folder with
          <code>[wsl2]</code> and <code>memory=24GB</code> on the next line (leave a few GB for Windows), then restart the computer.
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
            You have version {hw.app_version}.
          {:else if update.newer}
            <strong>Version {update.latest} is out</strong> (you have {update.current}). Download the installer and run it: your
            workflows, tools, databases and runs are kept.
          {:else}
            <span class="flex items-center gap-2 text-ok-ink"><Icon name="check" /> You have the newest version ({update.current}).</span>
          {/if}
        </span>
        {#if update?.newer}
          <Button variant="primary" icon="download" onclick={() => openLink(update!.url)}>Open the download page</Button>
        {/if}
        <Button icon="refresh" busy={checking} onclick={checkUpdate}>Check for updates</Button>
      </div>
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
