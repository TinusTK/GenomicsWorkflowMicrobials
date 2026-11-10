<script lang="ts">
  // The app's buttons. One "primary" per screen; "secondary" for other actions;
  // "quiet" for small links inside a card; "danger" for anything that removes.
  import type { Snippet } from 'svelte';
  import Icon from './Icon.svelte';

  let {
    variant = 'secondary',
    size = 'md',
    icon = '',
    href = '',
    label = '',
    disabled = false,
    busy = false,
    type = 'button',
    onclick,
    children,
    class: extra = '',
  }: {
    variant?: 'primary' | 'secondary' | 'quiet' | 'danger' | 'danger-solid' | 'on-brand';
    size?: 'sm' | 'md' | 'lg';
    icon?: string;
    href?: string;
    /** spoken name for an icon-only button */
    label?: string;
    disabled?: boolean;
    busy?: boolean;
    type?: 'button' | 'submit';
    onclick?: (e: MouseEvent) => void;
    children?: Snippet;
    class?: string;
  } = $props();

  const sizes = {
    sm: 'h-9 px-3 text-[13px] gap-1.5 rounded-[9px]',
    md: 'h-11 px-4 text-sm gap-2 rounded-[10px]',
    lg: 'h-12 px-6 text-[15px] gap-2 rounded-xl',
  };
  const variants = {
    primary: 'bg-brand text-on-brand hover:bg-brand-hover border border-transparent',
    secondary: 'bg-surface text-ink border border-line-strong hover:bg-hover',
    quiet: 'bg-transparent text-brand border border-transparent hover:bg-brand-softer',
    danger: 'bg-surface text-danger border border-danger-line hover:bg-danger-soft',
    'danger-solid': 'bg-danger text-white border border-transparent hover:opacity-90',
    'on-brand': 'bg-surface text-brand border border-transparent hover:bg-brand-softer',
  };
  const iconOnly = $derived(!children && !!icon);
  const cls = $derived(
    `inline-flex items-center justify-center font-semibold whitespace-nowrap transition-colors select-none
     disabled:opacity-50 disabled:cursor-not-allowed cursor-pointer no-underline
     ${sizes[size]} ${variants[variant]} ${iconOnly ? '!px-0 aspect-square' : ''} ${extra}`,
  );
</script>

{#if href && !disabled}
  <a {href} class={cls} aria-label={label || undefined}>
    {#if icon}<Icon name={icon} size={size === 'sm' ? 15 : 18} stroke={2.2} />{/if}
    {@render children?.()}
  </a>
{:else}
  <button {type} class={cls} disabled={disabled || busy} aria-label={label || undefined} aria-busy={busy || undefined} {onclick}>
    {#if busy}
      <span class="inline-block h-4 w-4 animate-spin rounded-full border-2 border-current border-t-transparent" aria-hidden="true"></span>
    {:else if icon}
      <Icon name={icon} size={size === 'sm' ? 15 : 18} stroke={2.2} />
    {/if}
    {@render children?.()}
  </button>
{/if}
