<script lang="ts">
  import { api, errorText, type Change, type Status, type Notice } from './api'
  import Icon from './Icon.svelte'
  import Modal from './Modal.svelte'
  let {
    id,
    status,
    refresh,
    notify,
    locked,
  }: { id: string; status: Status; refresh: () => Promise<void>; notify: Notice; locked: boolean } =
    $props()
  let unstaged = $derived(
    status.changes.filter((c) => c.worktree.trim() && c.worktree !== '.' && c.worktree !== '!'),
  )
  let staged = $derived(
    status.changes.filter((c) => c.index.trim() && !['?', '.', '!'].includes(c.index)),
  )
  let workingPaths = $state<string[]>([]),
    stagedPaths = $state<string[]>([])
  let title = $state(''),
    description = $state(''),
    busy = $state(false)
  let warnings = $state<string[]>([]),
    confirmSensitive = $state(false),
    askAuthor = $state(false)
  let authorName = $state(''),
    authorEmail = $state(''),
    dialogError = $state('')
  const labels: Record<string, string> = {
    '?': 'Untracked',
    A: 'Added',
    M: 'Modified',
    D: 'Deleted',
    R: 'Renamed',
    C: 'Copied',
    T: 'Type change',
    U: 'Conflict',
  }
  function validSelection(list: Change[], selection: string[]) {
    return list.filter((c) => selection.includes(c.path)).map((c) => c.path)
  }
  async function changeStage(staging: boolean) {
    const paths = staging
      ? validSelection(unstaged, workingPaths)
      : validSelection(staged, stagedPaths)
    if (!paths.length || busy) return
    busy = true
    try {
      await api(staging ? 'stage' : 'unstage', { id, paths })
      if (staging) workingPaths = []
      else stagedPaths = []
      await refresh()
    } catch (e) {
      notify(errorText(e), true)
    } finally {
      busy = false
    }
  }
  function setSelection(staging: boolean, paths: string[]) {
    if (staging) workingPaths = paths
    else stagedPaths = paths
  }
  function toggle(staging: boolean, path: string, checked: boolean) {
    const selected = staging ? workingPaths : stagedPaths
    setSelection(staging, checked ? [...selected, path] : selected.filter((p) => p !== path))
  }
  async function saveAuthor(e: SubmitEvent) {
    e.preventDefault()
    if (busy) return
    busy = true
    dialogError = ''
    try {
      await api('setAuthor', { id, name: authorName.trim(), email: authorEmail.trim() })
      await refresh()
      askAuthor = false
    } catch (e) {
      dialogError = errorText(e)
      return
    } finally {
      busy = false
    }
    await checkCommit(true)
  }
  async function commit(allowSensitive: boolean) {
    try {
      await api('commit', { id, title: title.trim(), description, allowSensitive })
      title = ''
      description = ''
      confirmSensitive = false
      notify('Commit created.')
      await refresh()
    } catch (e) {
      confirmSensitive = false
      notify(errorText(e), true)
    }
  }
  async function checkCommit(authorSet = false) {
    if (busy || !title.trim() || !staged.length) return
    if (!authorSet && (!status.authorName || !status.authorEmail)) {
      authorName = status.authorName || ''
      authorEmail = status.authorEmail || ''
      dialogError = ''
      askAuthor = true
      return
    }
    busy = true
    try {
      warnings = await api<string[]>('scan', { id })
      if (warnings.length) confirmSensitive = true
      else await commit(false)
    } catch (e) {
      notify(errorText(e), true)
    } finally {
      busy = false
    }
  }
  async function proceed() {
    if (busy) return
    busy = true
    try {
      await commit(true)
    } finally {
      busy = false
    }
  }
</script>

{#snippet fileGroup(list: Change[], staging: boolean)}
  {@const checked = validSelection(list, staging ? workingPaths : stagedPaths)}
  <section class="panel change-group">
    <div class="group-head">
      <h2>
        {staging ? 'Unstaged changes' : 'Staged changes'} <span class="count">{list.length}</span>
      </h2>
      <button disabled={busy || locked || !checked.length} onclick={() => changeStage(staging)}
        ><Icon name={staging ? 'plus' : 'back'} size={14} />{staging
          ? 'Stage selected'
          : 'Unstage selected'}</button
      >
    </div>
    {#if list.length}
      <div class="selection-tools">
        <button
          onclick={() =>
            setSelection(
              staging,
              list.map((c) => c.path),
            )}>Select all</button
        ><span aria-hidden="true">·</span><button onclick={() => setSelection(staging, [])}
          >Deselect all</button
        >
      </div>
      <div class="change-list">
        {#each list as change (change.path)}
          {@const code = (staging ? change.worktree : change.index).trim()}
          <label class="change-row"
            ><input
              type="checkbox"
              checked={checked.includes(change.path)}
              onchange={(e) => toggle(staging, change.path, e.currentTarget.checked)}
            />
            <Icon name="file" size={15} /><span class="filepath"
              >{#if change.oldPath}<span class="muted"
                  >{change.oldPath} →
                </span>{/if}{change.path}</span
            >
            <span class={'change-code code-' + code}>{labels[code] ?? code}</span></label
          >
        {/each}
      </div>
    {:else}
      <p class="empty-inline">
        <Icon name={staging ? 'check' : 'commit'} size={16} />{staging
          ? 'No unstaged changes.'
          : 'No changes staged for commit.'}
      </p>
    {/if}
  </section>
{/snippet}
<div class="change-layout">
  <div class="change-column">
    {@render fileGroup(unstaged, true)}{@render fileGroup(staged, false)}
  </div>
  <section class="panel commit-panel">
    <h2><Icon name="commit" />Commit</h2>
    <label>Title<input bind:value={title} required placeholder="Summary of changes" /></label>
    <label
      >Description <span class="optional">Optional</span><textarea bind:value={description} rows="4"
      ></textarea></label
    >
    <div class="staged-summary">
      <Icon name="file" size={15} />{staged.length} staged {staged.length === 1 ? 'file' : 'files'}
    </div>
    <button
      class="primary full"
      onclick={() => checkCommit()}
      disabled={locked || busy || !staged.length || !title.trim()}
      ><Icon name="commit" size={16} />Commit staged changes</button
    >
  </section>
</div>
<p class="helper-note">
  <Icon name="info" size={15} />Add or replace files in the repository folder. GitHub blocks regular
  Git files over 100 MiB.
</p>
{#if askAuthor}
  <Modal
    title="Set commit author"
    close={() => {
      if (!busy) askAuthor = false
    }}
  >
    <p>Choose the identity Git will record for commits in this repository.</p>
    <form onsubmit={saveAuthor}>
      <label>Author name<input bind:value={authorName} required /></label><label
        >Email address<input type="email" bind:value={authorEmail} required /></label
      >
      {#if dialogError}<p class="inline-error" role="alert">{dialogError}</p>{/if}
      <div class="dialog-actions">
        <button type="button" disabled={busy} onclick={() => (askAuthor = false)}>Cancel</button
        ><button class="primary" disabled={busy || !authorName.trim() || !authorEmail.trim()}
          >Save and continue</button
        >
      </div>
    </form>
  </Modal>
{/if}
{#if confirmSensitive}
  <Modal
    title="Review sensitive files"
    close={() => {
      if (!busy) confirmSensitive = false
    }}
  >
    <p>
      The staged changes include possible credentials or sensitive files. Detection is incomplete.
      Review the files before committing.
    </p>
    <ul class="warning-list">
      {#each warnings as warning}<li>{warning}</li>{/each}
    </ul>
    <div class="dialog-actions">
      <button disabled={busy} onclick={() => (confirmSensitive = false)}>Cancel</button><button
        class="danger"
        disabled={busy}
        onclick={proceed}>Commit anyway</button
      >
    </div>
  </Modal>
{/if}
