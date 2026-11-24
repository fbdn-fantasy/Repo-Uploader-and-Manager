<script lang="ts">
  import { onMount } from 'svelte'
  import { listen } from '@tauri-apps/api/event'
  import { api, errorText, type Notice } from './api'
  import Icon from './Icon.svelte'

  let { notify, closeMenu }: { notify: Notice; closeMenu: () => void } = $props()
  let progress = $state('')
  onMount(() => {
    const subscription = listen<string>('update-progress', ({ payload }) => (progress = payload))
    return () => { void subscription.then((stop) => stop()) }
  })
  async function check() {
    if (progress) return
    progress = 'Checking for updates…'
    try {
      notify(await api<string>('checkForUpdates'))
    } catch (error) {
      notify(errorText(error), true)
    } finally {
      closeMenu()
      progress = ''
    }
  }
</script>

<button class="update-button" disabled={!!progress} onclick={check}>
  <Icon name="refresh" size={16} /><span>Check For Updates</span>
</button>
{#if progress}<small role="status" class="update-progress">{progress}</small>{/if}

<style>
  .update-button { width: 100%; flex: none; margin: 0 0 12px; gap: 8px; }
  .update-progress { display: block; color: var(--muted); padding: 0 6px 12px; }
</style>
