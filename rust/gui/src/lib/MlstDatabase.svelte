<script lang="ts">
  // MLST typing's database, shown on the MLST typing tool (Tools page) and on
  // the Databases page: the pre-2025 database the tool comes with, or an
  // up-to-date one downloaded with the user's own PubMLST key. The state and
  // the download live in `live`, so both places (and the bar at the top) show
  // the same thing and a download carries on when the page changes.
  import Button from './ui/Button.svelte';
  import Dialog from './ui/Dialog.svelte';
  import Icon from './ui/Icon.svelte';
  import Pill from './ui/Pill.svelte';
  import { post, type MlstDb, type MlstJob } from './api';
  import { openLink } from './desktop';
  import { live, when } from './live.svelte';
  import { errorText, toasts } from './toast.svelte';

  const m = $derived(live.mlst);
  const job = $derived(m?.job ?? null);
  const working = $derived(job?.status === 'running');
  const percent = $derived(job?.progress && job.progress[1] ? Math.round((job.progress[0] / job.progress[1]) * 100) : null);

  /** the key form: closed, or which kind of key is being entered */
  let form = $state<'' | 'apikey' | 'oauth'>('');
  let key = $state('');
  let clientKey = $state('');
  let clientSecret = $state('');
  let code = $state('');
  /** the PubMLST page where the tool is approved (OAuth) */
  let approveUrl = $state('');
  /** OAuth: enter the client key and secret again, though a sign-in is waiting for its code */
  let restart = $state(false);
  let busy = $state('');
  let removeOpen = $state(false);

  /** One request to the engine; the panel then shows what changed. */
  async function act(name: string, path: string, body: unknown = {}, done?: () => void) {
    busy = name;
    try {
      const r = await post<MlstDb | MlstJob | { approve_url?: string }>(`/api/mlst-database/${path}`, body);
      if ('approve_url' in r && r.approve_url) approveUrl = r.approve_url;
      done?.();
      await live.refreshMlst();
    } catch (e) {
      toasts.show(errorText(e), 'error', undefined, 12000);
    } finally {
      busy = '';
    }
  }

  function closeForm() {
    form = '';
    key = clientKey = clientSecret = code = approveUrl = '';
    restart = false;
  }

  const use = (which: 'builtin' | 'own') =>
    act(`use-${which}`, 'use', { use: which }, () =>
      toasts.show(which === 'own' ? 'MLST typing now uses your up-to-date database.' : 'MLST typing now uses the pre-2025 database.'),
    );
  const saveKey = () => act('key', 'key', { key }, closeForm);
  const oauthStart = () =>
    act('oauth-start', 'oauth/start', { client_key: clientKey, client_secret: clientSecret }, () => {
      clientSecret = '';
      restart = false;
    });
  const oauthFinish = () => act('oauth-finish', 'oauth/finish', { code }, closeForm);
  const remove = () =>
    act('remove', 'remove', {}, () => {
      removeOpen = false;
      closeForm();
      toasts.show('Your PubMLST key and the downloaded database are removed. MLST typing uses the pre-2025 database.');
    });

  const keyName = $derived(m?.key_type === 'oauth' ? 'OAuth key' : 'Public data access key');
</script>

{#if m?.available}
  <div class="flex flex-col gap-4">
    <p class="m-0 text-[13px] leading-relaxed text-ink-2">
      MLST typing comes with PubMLST data submitted up to <strong>31 December 2024</strong>, which is free and needs no account.
      PubMLST gives newer alleles and sequence types only to people with their own account. Enter your own PubMLST key and the
      app downloads the current schemes to this computer (about 1.5 hours, once) and types against them.
    </p>

    {#if !m.answered && !working}
      <div class="flex gap-3 rounded-xl border border-warn-line bg-warn-soft p-3.5 text-[13px] text-warn-ink" role="status">
        <span class="mt-0.5 flex-none"><Icon name="info" size={18} /></span>
        <span>
          <strong>Choose the database MLST typing uses.</strong> Until you choose, the pre-2025 database is used. You can change
          this, or add a key, any time later.
        </span>
      </div>
    {/if}

    <!-- a download or a check that is going: stays visible on every page that shows this panel -->
    {#if working && job}
      <div class="flex flex-col gap-1.5 rounded-xl border border-line bg-surface-2 p-3.5">
        <span class="text-sm font-semibold">
          {job.kind === 'check' ? 'Checking PubMLST for anything newer' : 'Downloading the up-to-date database'}{percent != null ? ` · ${percent}%` : ''}
        </span>
        <div class="h-2 overflow-hidden rounded-full bg-line" role="progressbar" aria-label="MLST database progress" aria-valuenow={percent ?? 0} aria-valuemin={0} aria-valuemax={100}>
          <div class="h-full rounded-full bg-brand transition-[width] duration-500 {percent == null ? 'animate-pulse' : ''}" style="width: {percent ?? 100}%"></div>
        </div>
        <span class="text-xs text-muted">
          {job.message ?? 'Starting'} · You can leave this page: it carries on, and the bar at the top shows how far it is.
        </span>
      </div>
    {:else if job?.status === 'failed'}
      <div class="flex gap-3 rounded-xl border border-danger-line bg-danger-soft p-3.5 text-[13px] text-danger" role="alert">
        <span class="mt-0.5 flex-none"><Icon name="alert" size={18} /></span>
        <span><strong>{job.kind === 'check' ? 'The check did not work.' : 'The download did not work.'}</strong> {job.message ?? ''} The database in use was not changed.</span>
      </div>
    {/if}

    <div class="grid grid-cols-[repeat(auto-fit,minmax(300px,1fr))] gap-3">
      <!-- the database the tool comes with -->
      <section class="flex flex-col gap-2 rounded-xl border p-4 {m.in_use === 'builtin' && m.answered ? 'border-brand bg-brand-softer' : 'border-line'}">
        <div class="flex flex-wrap items-center gap-2">
          <h3 class="m-0 text-sm font-semibold">Pre-2025 database</h3>
          {#if m.in_use === 'builtin'}<Pill status="installed" text={m.answered ? 'In use' : 'In use for now'} />{/if}
        </div>
        <p class="m-0 text-[13px] text-ink-2">
          Built into the tool: PubMLST data to 31 December 2024, plus the current EnteroBase schemes. Nothing to download.
        </p>
        {#if !(m.in_use === 'builtin' && m.answered)}
          <div class="mt-auto pt-1">
            <Button size="sm" busy={busy === 'use-builtin'} disabled={working} onclick={() => use('builtin')}>Use the pre-2025 database</Button>
          </div>
        {/if}
      </section>

      <!-- the user's own, with their PubMLST key -->
      <section class="flex flex-col gap-2 rounded-xl border p-4 {m.in_use === 'own' ? 'border-brand bg-brand-softer' : 'border-line'}">
        <div class="flex flex-wrap items-center gap-2">
          <h3 class="m-0 text-sm font-semibold">Up-to-date database (your PubMLST key)</h3>
          {#if m.in_use === 'own'}<Pill status="installed" text="In use" />{:else if m.own_database}<Pill status="installed" text="Downloaded" />{/if}
        </div>
        {#if m.own_database}
          <p class="m-0 text-[13px] text-ink-2">
            Version {m.own_version || 'unknown'}. Kept on this computer only: PubMLST does not allow data from 2025 on to be passed on.
            {#if m.own_limited_schemes}
              {m.own_limited_schemes} scheme{m.own_limited_schemes === 1 ? ' still holds' : 's still hold'} only pre-2025 data: PubMLST
              gave your account nothing newer for {m.own_limited_schemes === 1 ? 'it' : 'them'}.
            {/if}
          </p>
        {:else if m.key_saved && working}
          <p class="m-0 text-[13px] text-ink-2">Your {keyName} is saved and the database is being downloaded. The pre-2025 database stays in use until it is complete.</p>
        {:else if m.key_saved}
          <p class="m-0 text-[13px] text-ink-2">Your {keyName} is saved, but the database is not downloaded yet. Press Download to try again.</p>
        {:else}
          <p class="m-0 text-[13px] text-ink-2">
            Needs a free account at pubmlst.org and a key from your account profile. Not downloaded yet.
          </p>
        {/if}
        {#if m.last_check && m.own_database}
          <p class="m-0 text-[13px] {m.last_check.newer ? 'font-semibold text-info-ink' : 'text-muted'}">
            Checked {when(m.last_check.checked)}:
            {m.last_check.newer
              ? `a newer database is available (${m.last_check.schemes_newer} scheme${m.last_check.schemes_newer === 1 ? '' : 's'} changed). Press Update to download it.`
              : 'nothing is newer than your database, so there is no need to update.'}
            {m.last_check.schemes_failed ? ` ${m.last_check.schemes_failed} scheme${m.last_check.schemes_failed === 1 ? '' : 's'} could not be checked.` : ''}
          </p>
        {/if}
        <div class="mt-auto flex flex-wrap gap-2 pt-1">
          {#if m.own_database && m.in_use !== 'own'}
            <Button size="sm" variant="primary" busy={busy === 'use-own'} disabled={working} onclick={() => use('own')}>Use the up-to-date database</Button>
          {/if}
          {#if m.key_saved}
            {#if m.own_database}
              <Button size="sm" icon="search" busy={busy === 'check'} disabled={working} onclick={() => act('check', 'check')}>Check for a newer database</Button>
            {/if}
            <Button size="sm" icon="download" busy={busy === 'update'} disabled={working} onclick={() => act('update', 'update')}>{m.own_database ? 'Update' : 'Download'}</Button>
            <Button size="sm" icon="edit" disabled={working} onclick={() => (form = m.key_type === 'oauth' ? 'oauth' : 'apikey')}>Change key</Button>
            <Button size="sm" variant="danger" disabled={working} onclick={() => (removeOpen = true)}>Remove my key</Button>
          {:else if !form}
            <Button size="sm" variant={m.answered ? 'secondary' : 'primary'} icon="plus" disabled={working} onclick={() => (form = m.oauth_waiting ? 'oauth' : 'apikey')}>Enter my PubMLST key</Button>
          {/if}
        </div>
      </section>
    </div>

    {#if form}
      <section class="flex flex-col gap-3 rounded-xl border border-line-strong p-4" aria-label="Your PubMLST key">
        <div class="flex flex-wrap items-center gap-3">
          <h3 class="m-0 text-sm font-semibold">{m.key_saved ? 'Change your PubMLST key' : 'Enter your PubMLST key'}</h3>
          <div class="ml-auto flex gap-1 rounded-[10px] bg-surface-2 p-1" role="tablist" aria-label="Kind of key">
            {#each [['apikey', 'Public data access key'], ['oauth', 'OAuth key']] as [kind, label] (kind)}
              <button
                role="tab"
                aria-selected={form === kind}
                class="cursor-pointer rounded-lg border-0 px-3 py-1.5 text-[13px] font-semibold {form === kind ? 'bg-surface text-ink shadow-sm' : 'bg-transparent text-muted'}"
                onclick={() => (form = kind as 'apikey' | 'oauth')}>{label}</button
              >
            {/each}
          </div>
        </div>
        <ol class="m-0 flex list-decimal flex-col gap-1 pl-5 text-[13px] text-ink-2">
          <li>
            Make a free account at
            <button class="cursor-pointer border-0 bg-transparent p-0 text-[13px] text-brand underline" onclick={() => openLink('https://pubmlst.org')}>pubmlst.org</button>
            and accept its terms.
          </li>
          {#if form === 'apikey'}
            <li>In your account profile, create a <strong>Public data access key</strong> (one key: the simple way).</li>
            <li>Paste it here. The app tests it with PubMLST, then downloads.</li>
          {:else}
            <li>In your account profile, create an <strong>OAuth key</strong>: a client key and a client secret.</li>
            <li>Paste both here, open the PubMLST page the app gives you, approve the tool there, and paste the verification code it shows.</li>
          {/if}
        </ol>

        {#if form === 'apikey'}
          <form class="flex flex-wrap items-end gap-2" onsubmit={(e) => { e.preventDefault(); saveKey(); }}>
            <label class="flex min-w-[260px] flex-1 flex-col gap-1.5 text-[13px] font-semibold">
              Public data access key
              <input class="input" type="password" autocomplete="off" spellcheck="false" bind:value={key} placeholder="Paste your key" />
            </label>
            <Button variant="primary" type="submit" icon="download" busy={busy === 'key'} disabled={!key.trim() || working}>Save and download</Button>
            <Button onclick={closeForm}>Cancel</Button>
          </form>
        {:else if approveUrl || (m.oauth_waiting && !restart)}
          <div class="flex flex-col gap-3">
            {#if approveUrl}
              <div class="flex flex-wrap items-center gap-2 text-[13px] text-ink-2">
                <Button size="sm" variant="primary" onclick={() => openLink(approveUrl)}>Open the PubMLST page</Button>
                <span>Log in there, approve the tool, and copy the verification code it shows.</span>
              </div>
            {:else}
              <p class="m-0 text-[13px] text-ink-2">
                A sign-in was started earlier and is waiting for its verification code. If you no longer have the PubMLST page open,
                start again.
              </p>
            {/if}
            <form class="flex flex-wrap items-end gap-2" onsubmit={(e) => { e.preventDefault(); oauthFinish(); }}>
              <label class="flex min-w-[220px] flex-1 flex-col gap-1.5 text-[13px] font-semibold">
                Verification code
                <input class="input" autocomplete="off" spellcheck="false" bind:value={code} placeholder="The code PubMLST shows" />
              </label>
              <Button variant="primary" type="submit" icon="download" busy={busy === 'oauth-finish'} disabled={!code.trim() || working}>Finish and download</Button>
              <Button onclick={() => { approveUrl = ''; code = ''; restart = true; }}>Start again</Button>
              <Button onclick={closeForm}>Cancel</Button>
            </form>
          </div>
        {:else}
          <form class="flex flex-wrap items-end gap-2" onsubmit={(e) => { e.preventDefault(); oauthStart(); }}>
            <label class="flex min-w-[220px] flex-1 flex-col gap-1.5 text-[13px] font-semibold">
              Client key
              <input class="input" autocomplete="off" spellcheck="false" bind:value={clientKey} placeholder="Paste the client key" />
            </label>
            <label class="flex min-w-[220px] flex-1 flex-col gap-1.5 text-[13px] font-semibold">
              Client secret
              <input class="input" type="password" autocomplete="off" spellcheck="false" bind:value={clientSecret} placeholder="Paste the client secret" />
            </label>
            <Button variant="primary" type="submit" busy={busy === 'oauth-start'} disabled={!clientKey.trim() || !clientSecret.trim() || working}>Continue</Button>
            <Button onclick={closeForm}>Cancel</Button>
          </form>
        {/if}
        <p class="m-0 text-xs text-muted">
          Your key is kept on this computer only and is sent to PubMLST and nowhere else. The pre-2025 database stays in use until
          the download is complete. PubMLST says the Public data access key is not for commercial software; commercial users need
          OAuth keys from PubMLST.
        </p>
      </section>
    {/if}
  </div>

  <Dialog bind:open={removeOpen} title="Remove your PubMLST key?">
    <p class="m-0">
      This forgets your key and deletes the up-to-date database downloaded with it. MLST typing goes back to the pre-2025
      database. Runs you already made are not changed. You can enter a key again later; the download then starts from the
      beginning.
    </p>
    {#snippet actions()}
      <Button onclick={() => (removeOpen = false)}>Cancel</Button>
      <Button variant="danger-solid" busy={busy === 'remove'} onclick={remove}>Remove my key</Button>
    {/snippet}
  </Dialog>
{/if}
