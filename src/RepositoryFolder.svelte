<script lang="ts">
  import { onMount } from 'svelte'
  import { api, errorText, type Repo } from './api'
  import Modal from './Modal.svelte'
  import Icon from './Icon.svelte'

  let { repo, close, saved }: {
    repo: Repo
    close: () => void
    saved: (initialized: boolean) => Promise<void>
  } = $props()
  type Choice = { path: string; identity: string; initialize: boolean; destination: string | null }
  let path = $state(''), choice = $state<Choice | null>(null), error = $state('')
  let busy = $state(false)
  let read = 0

  async function check(candidate: string) {
    const run = ++read
    path = candidate
    choice = null
    error = ''
    try {
      const result = await api<Choice>('repoFolderInspect', { id: repo.id, path: candidate })
      if (run === read) choice = result
    } catch (e) {
      if (run === read) error = errorText(e)
    }
  }

  onMount(() => { void check(repo.path) })

  async function browse() {
    busy = true
    try {
      const folder = await api<string | null>('pickFolder')
      if (folder) await check(folder)
    } catch (e) {
      error = errorText(e)
    } finally {
      busy = false
    }
  }

  async function apply() {
    if (!choice) {
      await check(path)
      return
    }
    const selected = choice
    busy = true
    error = ''
    try {
      await api('repoRelocate', { id: repo.id, choice: selected })
      await saved(selected.initialize)
    } catch (e) {
      choice = null
      error = errorText(e)
    } finally {
      busy = false
    }
  }

  async function reveal() {
    busy = true
    try {
      await api('openFolder', { id: repo.id, choice })
      close()
    } catch (e) {
      error = errorText(e)
    } finally {
      busy = false
    }
  }
</script>

<Modal title="Repository folder" close={() => { if (!busy) close() }}>
  <form onsubmit={(e) => { e.preventDefault(); void apply() }}>
    <label>
      Local folder
      <span class="path-field">
        <input value={path} required disabled={busy} oninput={(e) => {
          path = e.currentTarget.value
          read++
          choice = null
          error = ''
        }} />
        <button type="button" disabled={busy} onclick={browse}><Icon name="folder" size={16} />Browse</button>
      </span>
    </label>
    {#if choice}
      {#if choice.initialize}
        <p>This folder has no Git metadata. Initialize a new local history here and keep the existing files.</p>
      {:else}
        <p>Use the existing Git history and working files in this folder.</p>
      {/if}
      {#if choice.destination}
        <p>GitHub repository <b>{choice.destination}</b></p>
      {:else if repo.fullName}
        <p>This entry will become local only. You can connect it to GitHub again from Push.</p>
      {/if}
    {/if}
    {#if error}<p class="inline-error" role="alert">{error}</p>{/if}
    <div class="dialog-actions">
      <button type="button" disabled={busy} onclick={close}>Cancel</button>
      {#if choice}
        <button type="button" disabled={busy} onclick={reveal}>Show in file manager</button>
      {/if}
      <button class="primary" disabled={busy || !path.trim()}>
        {choice ? (choice.initialize ? 'Initialize Git here' : 'Reconnect folder') : 'Check folder'}
      </button>
    </div>
  </form>
</Modal>
