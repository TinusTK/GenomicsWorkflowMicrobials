<script lang="ts">
  // Shown in the desktop app while the engine isn't answering: says why in
  // plain words and offers the one-click fix (install WSL, install Docker).
  import Button from './ui/Button.svelte';
  import Icon from './ui/Icon.svelte';
  import { backendStatus, invoke, type BackendStatus } from './desktop';
  import { live } from './live.svelte';
  import { errorText, toasts } from './toast.svelte';

  let status = $state<BackendStatus | null>(null);
  let busy = $state(false);

  async function refresh() {
    status = await backendStatus();
  }
  refresh();
  const timer = setInterval(refresh, 3000);
  $effect(() => () => clearInterval(timer));

  async function act(cmd: string, msg: string) {
    busy = true;
    try {
      await invoke(cmd);
      toasts.show(msg, 'info', undefined, 10000);
    } catch (e) {
      toasts.show(errorText(e), 'error');
    } finally {
      busy = false;
    }
  }
  async function retry() {
    busy = true;
    status = await invoke<BackendStatus>('restart_backend').catch(() => status);
    await live.refreshUsage();
    await live.refreshHardware();
    busy = false;
  }
</script>

<div class="fixed inset-0 z-40 flex items-center justify-center bg-ground/95 p-6">
  <section class="card flex w-[min(620px,100%)] flex-col gap-5 rounded-2xl p-8 shadow-xl">
    <div class="flex items-center gap-3">
      <img src="./icon.svg" alt="" width="40" height="40" />
      <h1 class="m-0 font-display text-2xl font-semibold">Getting ready</h1>
    </div>
    {#if !status || (!status.problem && !status.running)}
      <p class="m-0 flex items-center gap-3 text-[15px] text-ink-2">
        <span class="inline-block h-5 w-5 animate-spin rounded-full border-2 border-brand border-t-transparent"></span>
        Starting the engine{status?.windows ? ' on the Linux side of Windows (WSL)' : ''}. This takes a few seconds the first time.
      </p>
    {:else if status.problem === 'wsl'}
      <p class="m-0 text-[15px] leading-relaxed text-ink-2">
        The analysis tools are Linux programs. On Windows they run on <strong>WSL</strong>, the Linux part of Windows, which isn't
        installed yet. Installing it asks for administrator rights; restart the computer afterwards and open this app again.
      </p>
      <div class="flex flex-wrap gap-2.5">
        <Button variant="primary" size="lg" icon="download" {busy} onclick={() => act('setup_wsl', 'Follow the Windows window. Restart the computer when it is done.')}>Install WSL</Button>
        <Button size="lg" icon="refresh" {busy} onclick={retry}>Check again</Button>
      </div>
    {:else if status.problem === 'distro'}
      <p class="m-0 text-[15px] leading-relaxed text-ink-2">
        WSL is there, but not the <strong>{status.distro}</strong> Linux the tools use. Installing it asks for administrator rights
        and then for a Linux user name and password of your choice (they stay on this computer).
      </p>
      <div class="flex flex-wrap gap-2.5">
        <Button variant="primary" size="lg" icon="download" {busy} onclick={() => act('setup_wsl', `Follow the window that installs ${status?.distro}, then press Check again.`)}>Install {status.distro}</Button>
        <Button size="lg" icon="refresh" {busy} onclick={retry}>Check again</Button>
      </div>
    {:else}
      <p class="m-0 flex items-start gap-2 rounded-xl bg-danger-soft px-4 py-3 text-sm text-danger"><Icon name="alert" /> {status.problem}</p>
      <div class="flex flex-wrap gap-2.5">
        <Button variant="primary" size="lg" icon="refresh" {busy} onclick={retry}>Try again</Button>
      </div>
    {/if}
  </section>
</div>
