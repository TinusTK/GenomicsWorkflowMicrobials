<script lang="ts">
  // Settings > Remove everything: before uninstalling, take away what the app
  // keeps beside the program itself. Nothing is ticked to start with, each
  // item says what is lost, and one confirm step comes before anything goes.
  // Run folders (Documents) are never touched.
  import Button from './ui/Button.svelte';
  import Dialog from './ui/Dialog.svelte';
  import Icon from './ui/Icon.svelte';
  import { get, post } from './api';
  import { invoke, isDesktop } from './desktop';
  import { live, size } from './live.svelte';
  import { errorText, toasts } from './toast.svelte';

  interface Overview {
    active_runs: number;
    containers: { available: boolean; count: number; bytes: number };
    data: { folders: string[]; bytes: number };
    runs_folder: string;
  }
  /** Windows only: Docker inside WSL and the WSL Ubuntu (the desktop app removes those). */
  interface Extras {
    windows: boolean;
    distro: string;
    docker_bytes: number | null;
    ubuntu_bytes: number | null;
  }

  let overview = $state<Overview | null>(null);
  let extras = $state<Extras | null>(null);
  let looking = $state(false);
  async function look() {
    looking = true;
    try {
      [overview, extras] = await Promise.all([
        get<Overview>('/api/cleanup'),
        isDesktop() ? invoke<Extras>('extras_overview').catch(() => null) : Promise.resolve(null),
      ]);
    } catch (e) {
      toasts.show(errorText(e), 'error');
    } finally {
      looking = false;
    }
  }

  let pick = $state({ containers: false, data: false, docker: false, ubuntu: false });
  // Ubuntu holds all the others; Docker holds the containers
  const all = $derived(pick.ubuntu);
  const chosen = $derived({
    containers: pick.containers || pick.docker || all,
    data: pick.data || all,
    docker: pick.docker || all,
    ubuntu: pick.ubuntu,
  });
  const any = $derived(chosen.containers || chosen.data || chosen.docker || chosen.ubuntu);

  const items = $derived(
    overview
      ? [
          {
            key: 'containers' as const,
            show: overview.containers.available,
            locked: pick.docker || all,
            title: 'Tool containers',
            size: overview.containers.count ? `about ${size(overview.containers.bytes)} (${overview.containers.count})` : 'none here',
            what: 'The downloaded containers the tools run in. They download again when a run needs them.',
          },
          {
            key: 'data' as const,
            show: true,
            locked: all,
            title: 'App data',
            size: size(overview.data.bytes),
            what: 'Your own workflows, the tools you added, reference databases and working files. This cannot be brought back.',
          },
          {
            key: 'docker' as const,
            show: !!extras?.windows && extras.docker_bytes != null,
            locked: all,
            title: 'Docker',
            size: extras?.docker_bytes != null ? size(extras.docker_bytes) : '',
            what: `Docker inside ${extras?.distro ?? 'Ubuntu'}, with every container in it. Anything else using that Docker stops working.`,
          },
          {
            key: 'ubuntu' as const,
            show: !!extras?.windows,
            locked: false,
            title: `${extras?.distro ?? 'Ubuntu'} (the Linux side of Windows)`,
            size: extras?.ubuntu_bytes != null ? size(extras.ubuntu_bytes) : '',
            what: 'The whole Ubuntu the app runs in: everything above, and anything else you keep in it. This cannot be brought back. Windows’ own WSL feature stays.',
          },
        ].filter((i) => i.show)
      : [],
  );

  let confirming = $state(false);
  let working = $state(false);
  let done = $state(false);
  async function remove() {
    working = true;
    const problems: string[] = [];
    try {
      if (chosen.ubuntu) {
        await invoke('remove_ubuntu');
      } else {
        if (chosen.containers || chosen.data) {
          // with Docker going too, its containers go with it
          const r = await post<{ problems: string[] }>('/api/cleanup', { containers: chosen.containers && !chosen.docker, data: chosen.data });
          problems.push(...r.problems);
        }
        if (chosen.docker) await invoke('remove_docker');
      }
      done = true;
      confirming = false;
      if (problems.length) toasts.show(`Some things could not be removed: ${problems.join('; ')}`, 'error', undefined, 12000);
    } catch (e) {
      confirming = false;
      toasts.show(typeof e === 'string' ? e : errorText(e), 'error');
      look();
    } finally {
      working = false;
    }
  }
</script>

<section class="card flex flex-col gap-3 p-5">
  <h2 class="m-0 text-base font-semibold">Remove everything</h2>
  {#if done}
    <p class="m-0 flex items-center gap-2 text-sm text-ok-ink"><Icon name="check" /> Removed. Your runs are still in {overview?.runs_folder ?? 'Documents'}.</p>
    <p class="m-0 text-[13px] leading-relaxed text-ink-2">
      Now close the app and uninstall it the usual way for this computer (Windows: Settings › Apps › Installed apps). If you keep
      using the app instead, it sets these things up again as they are needed.
    </p>
    {#if isDesktop()}
      <div><Button icon="x" onclick={() => invoke('quit_app')}>Close the app</Button></div>
    {/if}
  {:else}
    <p class="m-0 text-[13px] leading-relaxed text-ink-2">
      Uninstalling the app removes only the program. For a clean removal, tick what else should go and remove it here first, then
      uninstall the app. Your runs in Documents are never touched.
    </p>
    {#if !overview}
      <div><Button icon="search" busy={looking} onclick={look}>Show what can be removed</Button></div>
    {:else}
      {#each items as it (it.key)}
        <label class="flex cursor-pointer items-start gap-3 border-t border-line pt-3 text-sm first-of-type:border-0 first-of-type:pt-0">
          <input
            type="checkbox"
            class="mt-1 h-4 w-4 shrink-0 accent-[var(--color-danger)]"
            checked={chosen[it.key]}
            disabled={it.locked || working}
            onchange={(e) => (pick[it.key] = e.currentTarget.checked)}
          />
          <span class="flex min-w-0 flex-1 flex-col gap-0.5">
            <span class="flex flex-wrap items-baseline gap-2 font-semibold">
              {it.title}
              {#if it.size}<span class="text-[13px] font-normal text-muted">{it.size}</span>{/if}
            </span>
            <span class="text-[13px] text-ink-2">{it.what}</span>
          </span>
        </label>
      {/each}
      {#if overview.active_runs > 0}
        <p class="m-0 flex items-center gap-2 text-[13px] font-semibold text-danger">
          <Icon name="alert" />
          {overview.active_runs === 1 ? 'A run is' : `${overview.active_runs} runs are`} still going. Wait for it to finish, or stop it, before removing anything.
        </p>
      {/if}
      <div class="flex flex-wrap gap-2.5">
        <Button variant="danger" icon="trash" disabled={!any || overview.active_runs > 0 || live.updating} onclick={() => (confirming = true)}>
          Remove the ticked items…
        </Button>
        <Button icon="refresh" busy={looking} onclick={look}>Look again</Button>
      </div>
    {/if}
  {/if}
</section>

<Dialog bind:open={confirming} title="Remove these for good?">
  <ul class="m-0 flex list-disc flex-col gap-1 pl-5">
    {#each items.filter((i) => chosen[i.key]) as it (it.key)}
      <li><strong>{it.title}</strong>{it.size ? ` (${it.size})` : ''}</li>
    {/each}
  </ul>
  <p class="mt-3 mb-0">
    This cannot be undone. Your runs in Documents are kept.
    {#if chosen.ubuntu}Everything inside {extras?.distro ?? 'Ubuntu'} is deleted, also things that are not this app's.{/if}
  </p>
  {#snippet actions()}
    <Button disabled={working} onclick={() => (confirming = false)}>Keep everything</Button>
    <Button variant="danger-solid" icon="trash" busy={working} onclick={remove}>Remove</Button>
  {/snippet}
</Dialog>
