<script lang="ts">
  import Setup from './lib/Setup.svelte';
  import Sidebar from './lib/Sidebar.svelte';
  import { isDesktop } from './lib/desktop';
  import Icon from './lib/ui/Icon.svelte';
  import { live } from './lib/live.svelte';
  import { route } from './lib/router.svelte';
  import { toasts } from './lib/toast.svelte';
  import Home from './pages/Home.svelte';
  import NewRun from './pages/NewRun.svelte';
  import Runs from './pages/Runs.svelte';
  import Run from './pages/Run.svelte';
  import Workflows from './pages/Workflows.svelte';
  import Builder from './pages/Builder.svelte';
  import Tools from './pages/Tools.svelte';
  import ToolEditor from './pages/ToolEditor.svelte';
  import Databases from './pages/Databases.svelte';
  import Settings from './pages/Settings.svelte';

  live.start();
  const r = $derived(route.current);
  // the desktop app shows Setup until its engine answers (first start, WSL missing, ...)
  const showSetup = $derived(isDesktop() && !live.online);
</script>

{#if showSetup}<Setup />{/if}

<div class="flex min-h-screen flex-wrap bg-ground text-ink">
  <Sidebar />
  <main class="flex min-w-0 flex-[999_1_560px] flex-col">
    {#if r.page === 'new'}
      <NewRun workflow={r.query.get('workflow') ?? ''} />
    {:else if r.page === 'runs' && r.id}
      {#key r.id}<Run id={r.id} />{/key}
    {:else if r.page === 'runs'}
      <Runs />
    {:else if r.page === 'workflows' && r.id}
      {#key r.id + r.query.toString()}<Builder id={r.id} version={r.query.get('version')} startFrom={r.query.get('from')} />{/key}
    {:else if r.page === 'workflows'}
      <Workflows />
    {:else if r.page === 'tools' && r.id}
      {#key r.id}<ToolEditor id={r.id} />{/key}
    {:else if r.page === 'tools'}
      <Tools />
    {:else if r.page === 'databases'}
      <Databases />
    {:else if r.page === 'settings' || r.page === 'computer'}
      <Settings />
    {:else}
      <Home />
    {/if}
  </main>
</div>

<div class="fixed right-5 bottom-5 z-50 flex w-[min(380px,90vw)] flex-col gap-2" aria-live="polite">
  {#each toasts.list as t (t.id)}
    <div
      class="flex items-start gap-3 rounded-xl border px-4 py-3 text-sm shadow-lg
        {t.kind === 'error' ? 'border-danger-line bg-danger-soft text-danger' : t.kind === 'info' ? 'border-line bg-surface text-ink' : 'border-ok bg-ok-soft text-ok-ink'}"
    >
      <span class="mt-0.5"><Icon name={t.kind === 'error' ? 'alert' : t.kind === 'info' ? 'info' : 'check'} /></span>
      <span class="flex-1">{t.text}</span>
      {#if t.action}
        <button class="cursor-pointer border-0 bg-transparent font-semibold text-brand underline" onclick={() => { t.action?.run(); toasts.close(t.id); }}>{t.action.label}</button>
      {/if}
      <button class="cursor-pointer border-0 bg-transparent text-current opacity-70 hover:opacity-100" aria-label="Close" onclick={() => toasts.close(t.id)}><Icon name="x" size={16} /></button>
    </div>
  {/each}
</div>
