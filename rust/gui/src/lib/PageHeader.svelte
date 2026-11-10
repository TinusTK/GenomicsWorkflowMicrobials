<script lang="ts">
  // The bar at the top of every page: where you are, the page's main actions,
  // and the computer's live use (green good, amber/red getting tight).
  import type { Snippet } from 'svelte';
  import { live } from './live.svelte';

  let {
    title,
    crumbs = [],
    actions,
    extra,
  }: { title: string; crumbs?: { label: string; href: string }[]; actions?: Snippet; extra?: Snippet } = $props();

  function tone(frac: number) {
    return frac > 0.9 ? 'bg-danger-soft text-danger' : frac > 0.75 ? 'bg-warn-soft text-warn-ink' : 'bg-ok-soft text-ok-ink';
  }
  const u = $derived(live.usage);
</script>

<header class="flex flex-col gap-3 border-b border-line bg-surface px-8 pt-[18px] pb-[18px] max-[720px]:px-4">
  <div class="flex flex-wrap items-center gap-3">
    {#each crumbs as c}
      <a href={c.href} class="text-sm font-medium text-brand no-underline hover:underline">{c.label}</a>
      <span class="text-muted" aria-hidden="true">/</span>
    {/each}
    <h1 class="m-0 font-display text-[22px] font-semibold">{title}</h1>
    {@render extra?.()}
    <div class="ml-auto flex flex-wrap items-center gap-2">
      {#if actions}
        {@render actions()}
      {:else if u}
        <span class="rounded-lg px-2.5 py-1.5 text-[13px] font-medium {tone(u.cpu_percent / 100)}">
          CPU {Math.round(u.cpu_percent)}%{u.cpu_ghz ? ` · ${u.cpu_ghz} GHz` : ''} · {u.cpus} threads
        </span>
        <span class="rounded-lg px-2.5 py-1.5 text-[13px] font-medium {tone(u.memory_used_gb / u.memory_gb)}">
          RAM {u.memory_used_gb} / {Math.round(u.memory_gb)} GB
        </span>
        {#if u.disk_free_gb != null}
          <span class="rounded-lg px-2.5 py-1.5 text-[13px] font-medium {tone(u.disk_total_gb ? 1 - u.disk_free_gb / u.disk_total_gb : 0)}">
            Disk {Math.round(u.disk_free_gb)} GB free
          </span>
        {/if}
      {/if}
    </div>
  </div>
</header>
