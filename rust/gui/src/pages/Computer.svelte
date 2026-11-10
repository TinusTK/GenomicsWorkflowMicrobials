<script lang="ts">
  import PageHeader from '../lib/PageHeader.svelte';
  import Button from '../lib/ui/Button.svelte';
  import Icon from '../lib/ui/Icon.svelte';
  import { openFolder } from '../lib/desktop';
  import { live } from '../lib/live.svelte';

  live.refreshHardware();
  const hw = $derived(live.hardware);
  const engine = $derived(hw?.docker ? `Docker ${hw.docker}${hw.wsl ? ' (inside WSL)' : ''}` : hw?.apptainer ? hw.apptainer : null);
</script>

<PageHeader title="This computer">
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
        <div class="rounded-xl bg-surface-2 px-4 py-3 text-[13px] leading-relaxed text-ink-2">
          {#if hw.wsl}
            Docker runs inside the Linux side of Windows (WSL). Open the <strong>Ubuntu</strong> app and type
            <code>sudo service docker start</code>, then press Check again.
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
      <div class="flex flex-wrap items-center gap-3 text-sm">
        <span class="flex min-w-0 flex-1 flex-col"><span class="font-semibold">Runs (one folder per run)</span><span class="text-[13px] break-all text-muted">{hw.runs_folder}</span></span>
        <Button size="sm" icon="folder" onclick={() => openFolder(hw!.runs_folder)}>Open</Button>
      </div>
      <p class="m-0 text-[13px] text-muted">To delete a run, delete its folder here or in the app. Working files of deleted runs are cleared automatically.</p>
    </section>

    <p class="m-0 text-xs text-muted">Genomics Workflow Microbials {hw.app_version}</p>
  {:else}
    <p class="m-0 text-sm text-muted">Checking…</p>
  {/if}
</div>
