<script lang="ts">
  import { api, errorText, suggestsGitHubAccess, type Repo } from './api'
  import Modal from './Modal.svelte'
  import Icon from './Icon.svelte'

  let { repo, login, connected, close }: {
    repo: Repo
    login: string
    connected: (destination: Repo) => Promise<void>
    close: () => void
  } = $props()
  let mode = $state<'existing' | 'create'>('existing')
  let existing = $state<string>(), newName = $state<string>()
  let target = $derived(mode === 'existing' ? existing ?? `${login}/${repo.name}` : newName ?? repo.name)
  let busy = $state(false), error = $state('')

  async function connect(event: SubmitEvent) {
    event.preventDefault()
    if (busy || !target.trim()) return
    busy = true
    error = ''
    try {
      const destination = await api<Repo>('repoConnect', { id: repo.id, mode, target: target.trim() })
      await connected(destination)
    } catch (e) {
      error = errorText(e)
    } finally {
      busy = false
    }
  }
</script>

<Modal title="Connect to GitHub" close={() => { if (!busy) close() }}>
  <p>Choose where to push the commits from <b>{repo.name}</b>.</p>
  <form onsubmit={connect}>
    <label>Destination
      <select bind:value={mode} disabled={busy} onchange={() => (error = '')}>
        <option value="existing">Existing private repository</option>
        <option value="create">Create a private repository</option>
      </select>
    </label>
    {#if mode === 'existing'}
      <label>GitHub owner/repository
        <input value={target} oninput={(event) => (existing = event.currentTarget.value)}
          placeholder="owner/repository" required spellcheck="false" disabled={busy} />
      </label>
    {:else}
      <label>New repository name under @{login}
        <input value={target} oninput={(event) => (newName = event.currentTarget.value)}
          maxlength="100" required spellcheck="false" disabled={busy} />
      </label>
    {/if}
    {#if error}
      <p class="inline-error" role="alert">{error}</p>
      {#if suggestsGitHubAccess(error)}
        <button type="button" class="text-button"
          onclick={() => api('openInstall').catch((e) => (error = errorText(e)))}
        >Manage GitHub App access <Icon name="external" size={14} /></button>
      {/if}
    {/if}
    <div class="dialog-actions">
      <button type="button" disabled={busy} onclick={close}>Cancel</button>
      <button class="primary" disabled={busy || !target.trim()}>
        {busy ? 'Connecting…' : mode === 'existing' ? 'Connect repository' : 'Create and connect'}
      </button>
    </div>
  </form>
</Modal>
