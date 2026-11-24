<script lang="ts">
  import { api, errorText, type DeleteMode, type Notice, type Repo } from './api'
  import Modal from './Modal.svelte'

  let {
    repo,
    mode,
    title,
    close,
    changed,
    notify,
  }: {
    repo: Repo
    mode: DeleteMode
    title: string
    close: () => void
    changed: () => Promise<void>
    notify: Notice
  } = $props()
  let confirmation = $state(''),
    busy = $state(false),
    error = $state('')
  let exact = $derived(repo.fullName || repo.name)

  async function remove() {
    if (busy || confirmation !== exact) return
    busy = true
    error = ''
    try {
      await api('repoDelete', { id: repo.id, mode, confirmation })
    } catch (e) {
      error = errorText(e)
      // A combined deletion can finish on GitHub before local deletion fails.
      if (mode === 'both') await changed().catch((e) => notify(errorText(e), true))
      busy = false
      return
    }
    close()
    try {
      await changed()
      notify(mode === 'local' || mode === 'remove' ? 'Repository removed from this app.' : 'Selected deletion completed.')
    } catch (e) {
      notify(errorText(e), true)
    }
  }
</script>

<Modal {title} close={() => { if (!busy) close() }}>
  {#if mode === 'remove'}<p>This removes the repository from the app list and keeps its files.</p>{/if}
  {#if mode === 'local' || mode === 'both'}
    <p>This permanently deletes the entire local folder, including untracked files and unpushed commits.</p>
    <p class="target-path">{repo.path}</p>
  {/if}
  {#if mode === 'local'}
    <p>The repository will be removed from this app even if its folder is already gone.
      {#if repo.fullName}<b>{repo.fullName}</b> will stay on GitHub.{/if}</p>
  {/if}
  {#if mode === 'remote' || mode === 'both'}
    <p>The GitHub repository <b>{repo.fullName}</b> and its history will be deleted.</p>
  {/if}
  <form onsubmit={(event) => { event.preventDefault(); void remove() }}>
    <label>Type <b class="break">{exact}</b> to confirm<input
      bind:value={confirmation} spellcheck="false" autocomplete="off"
    /></label>
    {#if error}<p class="inline-error" role="alert">{error}</p>{/if}
    <div class="dialog-actions">
      <button type="button" disabled={busy} onclick={close}>Cancel</button>
      <button class="danger" disabled={busy || confirmation !== exact}>
        {mode === 'remove' ? 'Remove repository' : mode === 'local' ? 'Delete local copy' : 'Confirm deletion'}
      </button>
    </div>
  </form>
</Modal>
