<script lang="ts">
  import { api, errorText, type Branch, type Notice } from './api'
  import Modal from './Modal.svelte'
  import Icon from './Icon.svelte'
  let {
    id,
    items,
    refresh,
    notify,
  }: { id: string; items: Branch[]; refresh: () => Promise<void>; notify: Notice } = $props()
  let mode = $state<'' | 'create' | 'rename' | 'delete'>(''),
    target = $state(''),
    input = $state(''),
    busy = $state(false),
    error = $state('')
  function edit(op: typeof mode, name = '') {
    mode = op
    target = name
    input = op === 'rename' ? name : ''
    error = ''
  }
  async function action(op: string, name: string) {
    if (busy) return
    busy = true
    error = ''
    try {
      await api('branch', {
        id,
        operation: op,
        name,
        newName: op === 'rename' ? input.trim() : undefined,
      })
      mode = ''
      await refresh()
      notify(
        op === 'create'
          ? 'Branch created.'
          : op === 'switch'
            ? 'Switched branch.'
            : op === 'rename'
              ? 'Branch renamed.'
              : 'Branch deleted.',
      )
    } catch (e) {
      if (mode) error = errorText(e)
      else notify(errorText(e), true)
    } finally {
      busy = false
    }
  }
</script>

<div class="panel">
  <div class="group-head">
    <h2>Local branches</h2>
    <button onclick={() => edit('create')} disabled={busy}
      ><Icon name="plus" size={15} />New branch</button
    >
  </div>
  {#each items as branch (branch.name)}
    <div class="item-row">
      <Icon name="branch" /><strong class="break">{branch.name}</strong>{#if branch.current}<span
          class="tag active">Current</span
        >{/if}
      <div class="row-actions">
        {#if !branch.current}<button disabled={busy} onclick={() => action('switch', branch.name)}
            >Switch</button
          >{/if}
        <button
          class="icon"
          disabled={busy}
          aria-label={'Rename ' + branch.name}
          title="Rename branch"
          onclick={() => edit('rename', branch.name)}><Icon name="edit" size={16} /></button
        >
        <button
          class="icon danger-ink"
          disabled={busy || branch.current}
          aria-label={'Delete ' + branch.name}
          title="Delete branch"
          onclick={() => edit('delete', branch.name)}><Icon name="trash" size={16} /></button
        >
      </div>
    </div>
  {:else}<p class="empty-inline">Create the first commit to manage branches.</p>{/each}
</div>
{#if mode}
  <Modal
    title={mode === 'create'
      ? 'Create a branch'
      : mode === 'rename'
        ? 'Rename branch'
        : 'Delete branch?'}
    close={() => {
      if (!busy) mode = ''
    }}
  >
    <form
      onsubmit={(e) => {
        e.preventDefault()
        void action(mode, mode === 'create' ? input.trim() : target)
      }}
    >
      {#if mode === 'delete'}<p>
          Delete the local branch <strong>{target}</strong>? Branches with unmerged commits are
          protected.
        </p>
      {:else}<label
          >{mode === 'rename' ? 'New branch name' : 'Branch name'}<input
            bind:value={input}
            required
            spellcheck="false"
          /></label
        >{/if}
      {#if error}<p class="inline-error" role="alert">{error}</p>{/if}
      <div class="dialog-actions">
        <button type="button" disabled={busy} onclick={() => (mode = '')}>Cancel</button><button
          class={mode === 'delete' ? 'danger' : 'primary'}
          disabled={busy || (mode !== 'delete' && !input.trim())}
          >{mode === 'create'
            ? 'Create branch'
            : mode === 'rename'
              ? 'Rename branch'
              : 'Delete branch'}</button
        >
      </div>
    </form>
  </Modal>
{/if}
