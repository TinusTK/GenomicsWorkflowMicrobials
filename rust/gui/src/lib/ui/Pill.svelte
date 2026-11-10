<script lang="ts" module>
  // Run and task status in plain words, with one colour each.
  const LOOK: Record<string, [string, string]> = {
    running: ['Running', 'bg-info-soft text-info-ink'],
    queued: ['Starting', 'bg-info-soft text-info-ink'],
    starting: ['Starting', 'bg-info-soft text-info-ink'],
    succeeded: ['Finished', 'bg-ok-soft text-ok-ink'],
    failed: ['Failed', 'bg-danger-soft text-danger'],
    stopped: ['Stopped', 'bg-surface-2 text-ink-2 border border-line'],
    interrupted: ['Interrupted', 'bg-warn-soft text-warn-ink'],
    attention: ['Needs attention', 'bg-warn-soft text-warn-ink'],
    builtin: ['Built-in', 'bg-surface-2 text-ink-2 border border-line'],
    yours: ['Yours', 'bg-[#ede9fe] text-[#5b21b6]'],
    installed: ['Installed', 'bg-ok-soft text-ok-ink'],
    missing: ['Not installed', 'bg-warn-soft text-warn-ink'],
    draft: ['Draft, not saved', 'bg-warn-soft text-warn-ink'],
    locked: ['Read-only', 'bg-surface-2 text-ink-2 border border-line'],
  };
</script>

<script lang="ts">
  let { status, text = '' }: { status: string; text?: string } = $props();
  const look = $derived(LOOK[status] ?? [status, 'bg-surface-2 text-ink-2 border border-line']);
</script>

<span class="inline-flex h-6 items-center gap-1.5 whitespace-nowrap rounded-full px-2.5 text-xs font-bold {look[1]}">
  {#if status === 'running'}<span class="h-1.5 w-1.5 animate-pulse rounded-full bg-info"></span>{/if}
  {text || look[0]}
</span>
