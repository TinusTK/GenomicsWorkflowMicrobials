<script lang="ts">
  let {
    checked = $bindable(false),
    label,
    disabled = false,
    small = false,
    onchange,
  }: { checked?: boolean; label: string; disabled?: boolean; small?: boolean; onchange?: (v: boolean) => void } = $props();
</script>

<button
  type="button"
  role="switch"
  aria-checked={checked}
  aria-label={label}
  {disabled}
  onclick={() => {
    checked = !checked;
    onchange?.(checked);
  }}
  class="relative inline-flex shrink-0 cursor-pointer items-center rounded-full p-[3px] transition-colors disabled:cursor-not-allowed disabled:opacity-50
    {small ? 'h-5 w-[34px]' : 'h-[26px] w-11'} {checked ? 'bg-brand' : 'bg-line-strong'}"
>
  <span
    class="block rounded-full bg-white shadow transition-transform {small ? 'h-[14px] w-[14px]' : 'h-5 w-5'}"
    style="transform: translateX({checked ? (small ? 14 : 18) : 0}px)"
  ></span>
</button>
