<script lang="ts">
  import PageHeader from '../lib/PageHeader.svelte';
  import Button from '../lib/ui/Button.svelte';
  import Icon from '../lib/ui/Icon.svelte';
  import { backendStatus, invoke, isDesktop, openFolder } from '../lib/desktop';
  import { toasts } from '../lib/toast.svelte';
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
          { title: 'Your data', what: 'Your own workflows, installed tool packs and extra tool versions.', path: hw.data_folder, linux: true },
          { title: 'Reference databases', what: 'Kraken2, CheckM data and other databases you installed.', path: hw.databases_folder, linux: true },
          { title: 'Working files', what: 'Big temporary files while a run works; cleared after a successful run.', path: hw.work_folder, linux: true },
        ].map((p) => ({ ...p, linux: !p.path.startsWith('/mnt/') }))
      : [],
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
        automatically. Don't delete the data folder by hand: it holds your own workflows and tool packs.
      </p>
    </section>

    <p class="m-0 text-xs text-muted">Genomics Workflow Microbials {hw.app_version}</p>
  {:else}
    <p class="m-0 text-sm text-muted">Checking…</p>
  {/if}
</div>
