<script lang="ts">
  // One box on the workflow canvas: inputs on the left, outputs on the right.
  import { Handle, Position } from '@xyflow/svelte';

  interface Port {
    name: string;
    type: string;
    optional?: boolean;
  }
  let {
    data,
    selected = false,
  }: {
    data: { title: string; category: string; ins: Port[]; outs: Port[]; broken: boolean; off: boolean; isInput: boolean };
    selected?: boolean;
  } = $props();

  const CAT: Record<string, string> = {
    Input: 'bg-surface-2 text-ink-2',
    'Read preprocessing': 'bg-[#e0f2fe] text-[#075985]',
    Assembly: 'bg-brand-soft text-brand',
    'Assembly QC': 'bg-[#ecfccb] text-[#3f6212]',
    'Species identification': 'bg-[#fae8ff] text-[#86198f]',
    Typing: 'bg-[#ede9fe] text-[#5b21b6]',
    'AMR and virulence': 'bg-[#ffe4e6] text-[#9f1239]',
    'AMR and plasmids': 'bg-[#ffe4e6] text-[#9f1239]',
    Reports: 'bg-warn-soft text-warn-ink',
    Utilities: 'bg-surface-2 text-ink-2',
  };
  const rows = $derived(Math.max(data.ins.length, data.outs.length, 1));
</script>

<div
  class="w-[190px] rounded-xl bg-surface text-left text-ink shadow-sm
    {selected ? 'border-2 border-brand shadow-[0_6px_18px_rgba(15,118,110,0.18)]' : data.broken ? 'border-2 border-dashed border-warn' : 'border border-line-strong'}
    {data.off ? 'opacity-60' : ''}"
>
  <div class="flex flex-col gap-1 px-3 pt-2.5 pb-1.5">
    <span class="self-start rounded-md px-1.5 py-0.5 text-[11px] font-bold {CAT[data.category] ?? 'bg-surface-2 text-ink-2'}">{data.category}</span>
    <span class="truncate text-[15px] font-semibold">{data.title}</span>
  </div>
  <div class="relative flex flex-col gap-0 pb-2" style="min-height: {rows * 22}px">
    {#each data.ins as p, i}
      <span class="absolute left-3 text-[11px] text-muted" style="top: {i * 22}px">{p.name}{p.optional ? '' : ' *'}</span>
      <Handle type="target" position={Position.Left} id={p.name} style="top: {i * 22 + 8}px; width: 11px; height: 11px; background: var(--surface); border: 2px solid var(--brand)" />
    {/each}
    {#each data.outs as p, i}
      <span class="absolute right-3 text-[11px] text-muted" style="top: {i * 22}px">{p.name}</span>
      <Handle type="source" position={Position.Right} id={p.name} style="top: {i * 22 + 8}px; width: 11px; height: 11px; background: var(--brand); border: 2px solid var(--surface)" />
    {/each}
  </div>
</div>
