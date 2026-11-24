<script lang="ts">
  import type { AutoSyncItem } from './api'
  let { item }: { item: AutoSyncItem } = $props()
  let checked = $derived(new Date(item.checkedAt * 1000))
</script>

<small class="sync-result" class:attention={item.status === 'error' || item.status === 'skipped'}>
  Auto-sync · {item.message}
  {#if item.status !== 'checking' && item.checkedAt > 0}
    <time datetime={checked.toISOString()} title={checked.toLocaleString()}>{checked.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}</time>
  {/if}
</small>

<style>
  .sync-result { display: block; color: var(--muted); margin-top: 3px; overflow-wrap: anywhere; }
  .attention { color: var(--accent-ink); }
  time { margin-inline-start: .35em; white-space: nowrap; }
</style>
