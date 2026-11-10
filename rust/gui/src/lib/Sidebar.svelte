<script lang="ts">
  import Button from './ui/Button.svelte';
  import Icon from './ui/Icon.svelte';
  import { colimaSize, invoke, isDesktop } from './desktop';
  import { live } from './live.svelte';
  import { href, route } from './router.svelte';
  import { errorText, toasts } from './toast.svelte';

  const items = [
    { page: 'home', label: 'Home', icon: 'home' },
    { page: 'runs', label: 'Runs', icon: 'list' },
    { page: 'workflows', label: 'Workflows', icon: 'flow' },
    { page: 'tools', label: 'Tools', icon: 'tool' },
    { page: 'databases', label: 'Databases', icon: 'db' },
    { page: 'settings', label: 'Settings', icon: 'settings' },
  ];
  const active = $derived(route.current.page === 'new' ? 'runs' : route.current.page);
  const engineOk = $derived(!!(live.hardware?.docker || live.hardware?.apptainer));
  /** Why the tools can't run yet, in a line, and what to do about it. */
  const reason = $derived.by(() => {
    if (!live.online) return 'The app is still starting its background service. If this stays, close the app fully (tray icon, Quit) and open it again.';
    if (!live.hardware) return 'Checking this computer…';
    if (engineOk) return '';
    const why = live.hardware.docker_error ?? '';
    if (why.includes('not allowed')) return 'Docker is installed, but your user may not use it yet. Log out and in again; if it stays, open Settings.';
    if (why.includes('not running')) return 'Docker is installed but has not started yet. It can take a minute after the computer starts; if it stays, open Settings.';
    if (why.includes('not installed')) return live.hardware.wsl ? 'Docker is not installed yet. Open Settings and press Set up Docker.' : 'Docker is not installed yet. Open Settings to see how.';
    return why || 'No container engine was found. Open Settings.';
  });

  // ---------------------------------------------------------------- start and stop the services
  // What the tools run in, and what takes the memory: Docker (on a Mac with
  // Colima's Linux machine, on Windows with the Linux side, WSL). It does not
  // start by itself with the app on Mac and Linux, so Start is here; Stop
  // gives the memory back. Both are greyed out when they can't do anything.
  const dockerOn = $derived(!!live.hardware?.docker);
  const dockerWhy = $derived(live.hardware?.docker_error ?? '');
  const canStart = $derived(live.online && !dockerOn && dockerWhy.includes('not running'));
  const canStop = $derived(live.online && dockerOn && live.running === 0);
  const startHint = $derived(
    dockerOn ? 'Already started' : dockerWhy.includes('not installed') ? 'Docker is not installed yet: see Settings' : canStart ? 'Start Docker, where the tools run' : 'Nothing to start',
  );
  const stopHint = $derived(
    !dockerOn
      ? dockerWhy.includes('not installed')
        ? 'Docker is not installed yet: see Settings'
        : 'Not started'
      : live.running > 0
        ? 'A run is going: stop it first, or wait until it has finished'
        : live.hardware?.wsl
          ? 'Switch off Docker and the Linux side of Windows, so they take no memory'
          : 'Stop Docker, so it takes no memory',
  );
  let busy = $state<'' | 'start' | 'stop'>('');
  const pause = (ms: number) => new Promise((r) => setTimeout(r, ms));
  /** Keep looking until Docker is in the state asked for (it can take a minute to start). */
  async function settle(on: boolean) {
    for (let i = 0; i < 30; i++) {
      await live.refreshUsage();
      if (!live.online) return;
      await live.refreshHardware();
      if (!!live.hardware?.docker === on) return;
      await pause(2000);
    }
  }
  let checking = $state(false);
  async function checkAgain() {
    checking = true;
    await live.refreshUsage();
    await live.refreshHardware();
    checking = false;
  }
  async function services(what: 'start' | 'stop') {
    busy = what;
    try {
      await invoke(what === 'start' ? 'services_start' : 'services_stop', what === 'start' ? colimaSize(live.hardware) : undefined);
      await settle(what === 'start');
    } catch (e) {
      toasts.show(errorText(e), 'error', undefined, 12000);
    } finally {
      busy = '';
    }
  }
</script>

<nav
  aria-label="Main"
  class="flex w-[248px] shrink-0 flex-col gap-1 border-r border-line bg-surface px-3.5 py-5 max-[720px]:w-full max-[720px]:border-r-0 max-[720px]:border-b
    min-[721px]:sticky min-[721px]:top-0 min-[721px]:h-screen min-[721px]:self-start min-[721px]:overflow-y-auto"
>
  <a href={href('home')} class="mb-4 flex items-center gap-2.5 px-2 no-underline">
    <img src="./icon.svg" alt="" width="34" height="34" />
    <span class="flex flex-col leading-tight">
      <span class="font-display text-[15px] font-bold text-ink">Genomics Workflow</span>
      <span class="text-xs font-medium text-muted">Microbials</span>
    </span>
  </a>

  <a
    href={href('new')}
    class="mb-3.5 flex h-11 items-center justify-center gap-2 rounded-[10px] bg-brand text-sm font-semibold text-on-brand no-underline hover:bg-brand-hover"
  >
    <Icon name="plus" stroke={2.4} /> New run
  </a>

  <div class="flex flex-col gap-1 max-[720px]:flex-row max-[720px]:flex-wrap">
    {#each items as it (it.page)}
      {@const on = active === it.page}
      <a
        href={href(it.page)}
        aria-current={on ? 'page' : undefined}
        class="flex h-[42px] items-center gap-3 rounded-[10px] px-3 text-sm no-underline transition-colors
          {on ? 'bg-brand-soft font-semibold text-brand' : 'font-medium text-ink-2 hover:bg-hover'}"
      >
        <Icon name={it.icon} />
        {it.label}
        {#if it.page === 'runs' && live.running > 0}
          <span class="ml-auto rounded-full bg-info-soft px-2 py-0.5 text-xs font-semibold text-info-ink">{live.running} running</span>
        {/if}
        {#if it.page === 'settings' && live.update?.newer}
          <span class="ml-auto rounded-full bg-info-soft px-2 py-0.5 text-xs font-semibold text-info-ink">Update</span>
        {/if}
        {#if it.page === 'databases' && live.missingDb.length > 0}
          <span class="ml-auto h-2 w-2 rounded-full bg-warn" title="A database is not installed"></span>
        {/if}
      </a>
    {/each}
  </div>

  <!-- right under the menu, and the menu stays put while a page scrolls, so this is always in view -->
  <div class="mt-3 flex flex-col gap-1.5 border-t border-line px-2 pt-3.5 text-xs text-muted max-[720px]:hidden">
    {#if isDesktop()}
      <div class="flex gap-2">
        <span class="flex flex-1" title={startHint}>
          <Button size="sm" icon="play" class="flex-1" disabled={!canStart || busy !== ''} busy={busy === 'start'} onclick={() => services('start')}>Start</Button>
        </span>
        <span class="flex flex-1" title={stopHint}>
          <Button size="sm" icon="stop" class="flex-1" disabled={!canStop || busy !== ''} busy={busy === 'stop'} onclick={() => services('stop')}>Stop</Button>
        </span>
      </div>
    {/if}
    <span class="mb-1.5 flex" title="Look again at whether Docker is ready">
      <Button size="sm" icon="refresh" class="flex-1" busy={checking} disabled={busy !== ''} onclick={checkAgain}>Check again</Button>
    </span>
    <span class="flex items-center gap-2">
      <span class="h-2 w-2 rounded-full {live.online ? (engineOk ? 'bg-ok' : 'bg-warn') : 'bg-danger'}"></span>
      {#if !live.online}
        Background service not answering
      {:else if engineOk}
        Containers ready ({live.hardware?.docker ? `Docker${live.hardware?.wsl ? ' in WSL' : ''}` : 'Apptainer'})
      {:else}
        Containers not ready
      {/if}
    </span>
    {#if reason}<a href={href('settings')} class="leading-snug text-warn-ink no-underline hover:underline">{reason}</a>{/if}
    <span>Version {live.hardware?.app_version ?? ''}</span>
  </div>
</nav>
