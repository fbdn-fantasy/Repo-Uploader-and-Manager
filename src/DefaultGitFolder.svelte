<script lang="ts">
  import { api, errorText } from './api'
  import Icon from './Icon.svelte'

  let { folder, changed }: { folder: string | null; changed: () => Promise<void> } = $props()
  let draft = $state<string | null>(null)
  let value = $derived(draft ?? folder ?? '')
  let dirty = $derived(value !== (folder ?? ''))
  let error = $state('')
  let busy = $state(false)

  async function browse() {
    busy = true
    error = ''
    try {
      const selected = await api<string | null>('pickFolder')
      if (selected) draft = selected
    } catch (e) {
      error = errorText(e)
    } finally {
      busy = false
    }
  }

  async function save(path: string | null) {
    busy = true
    error = ''
    try {
      await api('setDefaultGitFolder', { path })
      await changed()
      draft = null
    } catch (e) {
      error = errorText(e)
    } finally {
      busy = false
    }
  }

  function submit(event: SubmitEvent) {
    event.preventDefault()
    if (!busy && dirty) void save(value || null)
  }
</script>

<form class="panel form-panel" onsubmit={submit}>
  <h2 id="default-folder-label">Default Git folder</h2>
  <p id="default-folder-help" class="muted">Create and clone repositories in this folder. Leave it empty to choose each time.</p>
  <div class="folder-controls">
    <input aria-labelledby="default-folder-label" aria-describedby="default-folder-help"
      placeholder="Enter an absolute folder path" spellcheck="false" {value} disabled={busy}
      oninput={(e) => { draft = e.currentTarget.value; error = '' }} />
    <button type="button" disabled={busy} onclick={browse}><Icon name="folder" size={16} />Browse</button>
    <button class="primary" disabled={busy || !dirty}>Save folder</button>
    {#if folder}<button type="button" disabled={busy} onclick={() => save(null)}>Clear default</button>{/if}
  </div>
  {#if error}<p class="inline-error" role="alert">{error}</p>{/if}
</form>

<style>
  .folder-controls {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    margin-top: 12px;
  }
  .folder-controls input {
    flex: 1 1 230px;
    min-width: 0;
  }
</style>
