<script lang="ts">
  // A modal dialog (confirm a delete, pick a folder, ...). Escape closes it.
  import type { Snippet } from 'svelte';

  let {
    open = $bindable(false),
    title,
    wide = false,
    children,
    actions,
  }: { open?: boolean; title: string; wide?: boolean; children?: Snippet; actions?: Snippet } = $props();

  let dlg: HTMLDialogElement | undefined = $state();
  $effect(() => {
    if (!dlg) return;
    if (open && !dlg.open) dlg.showModal();
    if (!open && dlg.open) dlg.close();
  });
</script>

<dialog
  bind:this={dlg}
  onclose={() => (open = false)}
  class="m-auto rounded-2xl border border-line bg-surface p-0 text-ink shadow-2xl backdrop:bg-black/40 {wide ? 'w-[min(760px,92vw)]' : 'w-[min(520px,92vw)]'}"
  aria-labelledby="dlg-title"
>
  {#if open}
    <div class="flex flex-col gap-4 p-6">
      <h2 id="dlg-title" class="m-0 text-lg font-semibold">{title}</h2>
      <div class="text-sm leading-relaxed text-ink-2">{@render children?.()}</div>
      {#if actions}<div class="flex flex-wrap justify-end gap-2.5">{@render actions()}</div>{/if}
    </div>
  {/if}
</dialog>
