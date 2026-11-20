<script lang="ts">
  import PageHeader from '../lib/PageHeader.svelte';
  import MlstDatabase from '../lib/MlstDatabase.svelte';
  import Button from '../lib/ui/Button.svelte';
  import Icon from '../lib/ui/Icon.svelte';
  import Dialog from '../lib/ui/Dialog.svelte';
  import Pill from '../lib/ui/Pill.svelte';
  import { post, type Database, type Job } from '../lib/api';
  import { KEY_DBS, KEY_DB_TOOLS, KUN_PENG, live, size } from '../lib/live.svelte';
  import { errorText, toasts } from '../lib/toast.svelte';

  // the installs live in `live`: they carry on, and still show here, after a look at another page
  const jobs = $derived(live.dbJobs);
  live.refreshDatabases();
  live.refreshKeyDbs();

  async function install(id: string) {
    try {
      await live.installDatabase(id);
    } catch (e) {
      toasts.show(errorText(e), 'error');
    }
  }
  async function prepare(id: string) {
    try {
      await live.prepareForKunPeng(id);
    } catch (e) {
      toasts.show(errorText(e), 'error', undefined, 10000);
    }
  }

  // removing: the Kraken2 database itself, or the copy of it that Kun-peng reads
  let removing = $state<{ db: Database; what: 'database' | 'copy' } | null>(null);
  let removeOpen = $state(false);
  let removeBusy = $state(false);
  function askRemove(db: Database, what: 'database' | 'copy') {
    removing = { db, what };
    removeOpen = true;
  }
  async function remove() {
    if (!removing) return;
    const { db, what } = removing;
    removeBusy = true;
    try {
      await post(`/api/databases/${db.id}/${what === 'copy' ? 'kun-peng/remove' : 'remove'}`);
      // an ended install or copy job no longer says anything about it
      delete live.dbJobs[what === 'copy' ? KUN_PENG + db.id : db.id];
      await live.refreshDatabases();
      toasts.show(what === 'copy' ? `The Kun-peng copy of ${db.name} is removed.` : `${db.name} is removed.`);
      removeOpen = false;
    } catch (e) {
      toasts.show(errorText(e), 'error', undefined, 10000);
    } finally {
      removeBusy = false;
    }
  }

  // databases that are alternatives of each other (the Kraken2 sizes) share one box, closed until clicked
  const GROUPS: Record<string, { title: string; about: string }> = {
    kraken2: {
      title: 'Kraken2',
      about: 'Databases for species identification with Kraken2, Bracken and Kun-peng. One of them is enough.',
    },
  };
  type Item = { db: Database; group?: undefined } | { group: string; dbs: Database[]; db?: undefined };
  const items = $derived.by(() => {
    const out: Item[] = [];
    for (const db of live.databases) {
      const g = db.alternatives && GROUPS[db.alternatives] ? db.alternatives : null;
      if (!g) out.push({ db });
      else {
        const box = out.find((i) => i.group === g);
        if (box?.group) box.dbs.push(db);
        else out.push({ group: g, dbs: [db] });
      }
    }
    return out;
  });
  let opened = $state<Record<string, boolean>>({});
  const working = (dbs: Database[]) =>
    dbs.some((d) => jobs[d.id]?.status === 'running' || jobs[KUN_PENG + d.id]?.status === 'running' || d.kun_peng?.preparing);
  /** "Standard-8" from "Kraken2 Standard-8 (with Bracken files)" */
  const short = (db: Database, group: string) =>
    db.name.replace(new RegExp(`^${GROUPS[group].title} `), '').replace(/ \(.*\)$/, '');
  /** one database in a few words, for the closed box */
  function inShort(db: Database): { text: string; tone: string } {
    if (jobs[db.id]?.status === 'running') return { text: 'installing', tone: 'bg-info' };
    if (jobs[KUN_PENG + db.id]?.status === 'running' || db.kun_peng?.preparing) return { text: 'preparing for Kun-peng', tone: 'bg-info' };
    const copy = !!db.kun_peng?.prepared;
    if (db.installed) return { text: copy ? 'installed, ready for Kun-peng' : 'installed', tone: 'bg-ok' };
    if (copy) return { text: 'Kun-peng copy only', tone: 'bg-ok' };
    return { text: 'not installed', tone: 'bg-line-strong' };
  }

  const pct = (j: Job) => (j.progress && j.progress[1] ? Math.round((j.progress[0] / j.progress[1]) * 100) : null);
</script>

{#snippet progress(j: Job, label: string)}
  <div class="flex flex-col gap-1.5">
    <div class="h-2 overflow-hidden rounded-full bg-line" role="progressbar" aria-label={label} aria-valuenow={pct(j) ?? 0} aria-valuemin={0} aria-valuemax={100}>
      <div class="h-full rounded-full {j.status === 'failed' ? 'bg-danger' : 'bg-brand'}" style="width: {pct(j) ?? 5}%"></div>
    </div>
    <span class="text-xs text-muted">
      {#if j.progress && j.progress[1] > 1}{(j.progress[0] / 1e9).toFixed(2)} of {(j.progress[1] / 1e9).toFixed(2)} GB ·{/if}
      {j.log.at(-1) ?? ''}
    </span>
  </div>
{/snippet}

{#snippet card(db: Database, inner: boolean)}
  {@const j = jobs[db.id]}
  {@const kp = db.kun_peng}
  {@const kj = jobs[KUN_PENG + db.id]}
  {@const copying = kj?.status === 'running' || !!kp?.preparing}
  <article class="flex flex-col gap-3 {inner ? 'rounded-xl border border-line p-4' : 'card p-5'}">
    <div class="flex flex-wrap items-center gap-3">
      <h2 class="m-0 text-base font-semibold">{db.name}</h2>
      {#if !db.installed && kp?.prepared}
        <Pill status="locked" text="Removed · Kun-peng copy kept" />
      {:else}
        <Pill status={db.installed ? 'installed' : 'missing'} />
      {/if}
      <span class="text-sm text-muted">Version {db.version} · {db.size_gb} GB</span>
      <div class="ml-auto">
        {#if !db.installed}
          <Button variant={kp?.prepared ? 'secondary' : 'primary'} icon="download" busy={j?.status === 'running'} onclick={() => install(db.id)}>
            {j?.status === 'failed' ? 'Try again' : j?.status === 'running' ? 'Installing' : 'Install'}
          </Button>
        {:else if kp}
          <Button variant="danger" size="sm" disabled={copying} onclick={() => askRemove(db, 'database')}>Remove</Button>
        {/if}
      </div>
    </div>
    {#if db.description}<p class="m-0 text-[13px] leading-relaxed text-ink-2">{db.description}</p>{/if}
    {#if db.galaxy_note}<p class="m-0 rounded-lg bg-surface-2 px-3 py-2 text-[13px] text-ink-2"><strong>Compared with Galaxy:</strong> {db.galaxy_note}</p>{/if}
    {#if j && j.status !== 'done'}{@render progress(j, 'Download progress')}{/if}
    {#if db.installed}<span class="text-xs break-all text-muted">{db.path}</span>{/if}

    {#if kp}
      <!-- the copy of this database that the Kun-peng tool reads -->
      <div class="flex flex-col gap-2.5 rounded-xl border border-line bg-surface-2 p-4">
        <div class="flex flex-wrap items-center gap-3">
          <h3 class="m-0 text-sm font-semibold">For Kun-peng</h3>
          {#if kp.prepared}
            <Pill status="installed" text="Ready" />
            <span class="text-[13px] text-muted">{size(kp.bytes)}{kp.source ? ` · made from ${kp.source}` : ''}</span>
          {:else if copying}
            <Pill status="running" text="Preparing" />
          {:else}
            <Pill status="locked" text="Not prepared" />
          {/if}
          <div class="ml-auto">
            {#if kp.prepared}
              <Button variant="danger" size="sm" onclick={() => askRemove(db, 'copy')}>Remove the Kun-peng copy</Button>
            {:else if db.installed}
              <Button size="sm" busy={copying} onclick={() => prepare(db.id)}>
                {kj?.status === 'failed' ? 'Try again' : copying ? 'Preparing' : 'Prepare for Kun-peng'}
              </Button>
            {/if}
          </div>
        </div>
        <p class="m-0 text-[13px] leading-relaxed text-ink-2">
          {#if kp.prepared && db.installed}
            Kun-peng uses its own copy and Kraken2 uses the database above. Keep both, or remove the one you don't need: each works without the other.
          {:else if kp.prepared}
            Kun-peng works with this copy. Kraken2 and Bracken need the database itself: install it again to use them with it.
          {:else if db.installed}
            Kun-peng reads its own copy of this database, cut into pieces so that it needs little memory however big the database is.
            The copy takes about {db.size_gb} GB more disk. A Kun-peng step makes it by itself the first time; here you can make it ahead of time.
            Once it is made, the database above can be removed if you only use Kun-peng with it.
          {:else}
            Install this database first: the Kun-peng copy is made from it.
          {/if}
        </p>
        {#if kj && kj.status !== 'done'}{@render progress(kj, 'Progress of the Kun-peng copy')}{/if}
        {#if kp.prepared}<span class="text-xs break-all text-muted">{kp.path}</span>{/if}
      </div>
    {/if}
  </article>
{/snippet}

<PageHeader title="Databases" />

<div class="flex w-full max-w-[1000px] flex-col gap-5 px-8 pt-7 pb-10 max-[720px]:px-4">
  <p class="m-0 max-w-[720px] text-sm text-muted">
    Some tools compare your genomes with a reference database. They are downloaded once and kept on this computer. If a
    download is interrupted, press Install again: it carries on where it stopped.
  </p>
  {#each items as item (item.group ?? item.db.id)}
    {#if item.group}
      {@const g = GROUPS[item.group]}
      {@const open = opened[item.group] ?? working(item.dbs)}
      <section class="card flex flex-col">
        <button
          type="button"
          class="flex w-full cursor-pointer flex-col gap-2 rounded-[inherit] border-0 bg-transparent p-5 text-left text-ink hover:bg-hover"
          aria-expanded={open}
          onclick={() => (opened[item.group] = !open)}
        >
          <span class="flex w-full flex-wrap items-center gap-3">
            <span class="text-base font-semibold">{g.title}</span>
            <span class="text-sm text-muted">{item.dbs.length} databases</span>
            <span class="ml-auto flex items-center gap-1.5 text-[13px] font-semibold text-brand">
              {open ? 'Hide' : 'Show'}
              <span class="inline-flex transition-transform {open ? 'rotate-90' : ''}"><Icon name="chevron" size={16} /></span>
            </span>
          </span>
          <span class="flex flex-wrap gap-x-5 gap-y-1 text-[13px] text-ink-2">
            {#each item.dbs as d (d.id)}
              {@const st = inShort(d)}
              <span class="inline-flex items-center gap-1.5">
                <span class="h-2 w-2 rounded-full {st.tone}"></span>
                <strong class="font-semibold">{short(d, item.group)}</strong> {st.text}
              </span>
            {/each}
          </span>
        </button>
        {#if open}
          <div class="flex flex-col gap-4 border-t border-line p-5">
            <p class="m-0 text-[13px] text-muted">{g.about}</p>
            {#each item.dbs as d (d.id)}{@render card(d, true)}{/each}
          </div>
        {/if}
      </section>
    {:else}
      {@render card(item.db!, false)}
    {/if}
  {/each}
  {#each KEY_DB_TOOLS as tool (tool)}
    {#if live.keyDb[tool]?.available}
      <article class="card flex flex-col gap-3 p-5" id="{tool}-database">
        <div class="flex flex-wrap items-center gap-3">
          <h2 class="m-0 text-base font-semibold">{KEY_DBS[tool].tool} database</h2>
          <Pill status="installed" text={live.keyDb[tool]?.in_use === 'own' ? 'Using your up-to-date database' : 'Using the pre-2025 database'} />
        </div>
        <MlstDatabase {tool} />
      </article>
    {/if}
  {/each}
</div>

<Dialog
  bind:open={removeOpen}
  title={removing?.what === 'copy' ? `Remove the Kun-peng copy of ${removing.db.name}?` : `Remove ${removing?.db.name ?? ''}?`}
>
  {#if removing}
    {@const db = removing.db}
    {#if removing.what === 'copy'}
      {#if db.installed}
        Kun-peng makes the copy again by itself the next time it needs it. {db.name} itself stays, and Kraken2 keeps working.
      {:else}
        <strong>{db.name} itself is not installed any more.</strong> After this, neither Kun-peng nor Kraken2 can use it until it
        is downloaded again ({db.size_gb} GB).
      {/if}
    {:else if db.kun_peng?.prepared}
      Its Kun-peng copy stays, so Kun-peng keeps working with it. Kraken2 and Bracken steps that use this database won't run
      until it is installed again ({db.size_gb} GB download).
    {:else}
      <strong>It has no Kun-peng copy.</strong> After this, neither Kraken2 nor Kun-peng can use it until it is downloaded
      again ({db.size_gb} GB). To keep using it with Kun-peng, press Prepare for Kun-peng first.
    {/if}
  {/if}
  {#snippet actions()}
    <Button onclick={() => (removeOpen = false)}>Keep it</Button>
    <Button variant="danger-solid" busy={removeBusy} onclick={remove}>Yes, remove it</Button>
  {/snippet}
</Dialog>
