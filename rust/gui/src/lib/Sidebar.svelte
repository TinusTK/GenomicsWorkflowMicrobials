<script lang="ts">
  import Icon from './ui/Icon.svelte';
  import { live } from './live.svelte';
  import { href, route } from './router.svelte';

  const items = [
    { page: 'home', label: 'Home', icon: 'home' },
    { page: 'runs', label: 'Runs', icon: 'list' },
    { page: 'workflows', label: 'Workflows', icon: 'flow' },
    { page: 'tools', label: 'Tools and packs', icon: 'tool' },
    { page: 'databases', label: 'Databases', icon: 'db' },
    { page: 'settings', label: 'Settings', icon: 'settings' },
  ];
  const active = $derived(route.current.page === 'new' ? 'runs' : route.current.page);
  const engineOk = $derived(!!(live.hardware?.docker || live.hardware?.apptainer));
</script>

<nav aria-label="Main" class="flex w-[248px] shrink-0 flex-col gap-1 border-r border-line bg-surface px-3.5 py-5 max-[720px]:w-full max-[720px]:border-r-0 max-[720px]:border-b">
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
        {#if it.page === 'databases' && live.missingDb.length > 0}
          <span class="ml-auto h-2 w-2 rounded-full bg-warn" title="A database is not installed"></span>
        {/if}
      </a>
    {/each}
  </div>

  <div class="mt-auto flex flex-col gap-1.5 border-t border-line px-2 pt-3.5 text-xs text-muted max-[720px]:hidden">
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
    <span>Version {live.hardware?.app_version ?? ''}</span>
  </div>
</nav>
