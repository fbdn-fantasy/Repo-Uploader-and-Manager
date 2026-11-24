<script lang="ts">
  import { onMount } from 'svelte'
  import {
    api,
    errorText,
    type Repo,
    type Account,
    type Info,
    type DeleteMode,
    type Notice,
  } from './api'
  import Modal from './Modal.svelte'
  import DeleteRepositoryDialog from './DeleteRepositoryDialog.svelte'
  import Icon from './Icon.svelte'
  let {
    repo,
    accounts,
    login,
    changed,
    notify,
  }: {
    repo: Repo
    accounts: Account[]
    login: string
    changed: () => Promise<void>
    notify: Notice
  } = $props()
  let info = $state<Info | null>(null),
    name = $state(''),
    description = $state(''),
    visibility = $state('private')
  let busy = $state(false),
    error = $state(''),
    visibilityConfirm = $state(false)
  let deletion = $state<DeleteMode>('remove'),
    confirmDelete = $state(false),
    dialogError = $state('')
  let newAccount = $state(''),
    associationConfirm = $state(false)
  let otherAccounts = $derived(accounts.filter((a) => a.id !== repo.accountId))
  async function locate() {
    if (busy) return
    busy = true
    try {
      const path = await api<string | null>('pickFolder')
      if (!path) return
      await api('repoRelocate', { id: repo.id, path })
      await changed()
      notify('Local repository location updated.')
    } catch (e) {
      notify(errorText(e), true)
    } finally {
      busy = false
    }
  }
  async function associate() {
    if (busy || !otherAccounts.some((a) => a.id === newAccount)) return
    busy = true
    dialogError = ''
    try {
      await api('repoAssociate', { id: repo.id, accountId: newAccount })
      associationConfirm = false
      await changed()
      notify('Repository account updated.')
    } catch (e) {
      dialogError = errorText(e)
    } finally {
      busy = false
    }
  }
  let exact = $derived(repo.fullName || repo.name)
  const modes: [DeleteMode, string][] = [
    ['remove', 'Remove from this app'],
    ['local', 'Delete local copy'],
    ['remote', 'Delete GitHub repository'],
    ['both', 'Delete local and GitHub copies'],
  ]
  async function load() {
    if (!repo.fullName) return
    busy = true
    error = ''
    try {
      info = await api<Info>('repoInfo', { id: repo.id })
      name = info.name
      description = info.description || ''
      visibility = info.private ? 'private' : 'public'
    } catch (e) {
      error = errorText(e)
    } finally {
      busy = false
    }
  }
  onMount(() => {
    void load()
  })
  async function save() {
    if (busy) return
    busy = true
    error = ''
    dialogError = ''
    try {
      await api('repoUpdate', {
        id: repo.id,
        name: name.trim(),
        description,
        private: visibility === 'private',
      })
      visibilityConfirm = false
      await changed()
      await load()
      notify('Repository settings saved.')
    } catch (e) {
      if (visibilityConfirm) dialogError = errorText(e)
      else error = errorText(e)
    } finally {
      busy = false
    }
  }
</script>

<div class="settings-stack">
  <section class="panel form-panel">
    <h2>Local repository</h2>
    <dl class="properties">
      <div>
        <dt>Location</dt>
        <dd>
          {repo.path}
          <div>
            <button class="text-button" disabled={busy} onclick={locate}
              ><Icon name="folder" size={14} />Locate moved folder</button
            >
          </div>
        </dd>
      </div>
      <div>
        <dt>Associated account</dt>
        <dd>
          @{login}
          {#if otherAccounts.length}<div class="association-field">
              <select aria-label="New associated account" bind:value={newAccount}
                ><option value="">Choose another account</option>{#each otherAccounts as a}<option
                    value={a.id}>@{a.login}</option
                  >{/each}</select
              ><button
                disabled={busy || !newAccount}
                onclick={() => {
                  dialogError = ''
                  associationConfirm = true
                }}>Change</button
              >
            </div>{/if}
        </dd>
      </div>
      <div>
        <dt>GitHub repository</dt>
        <dd>{repo.fullName || 'No associated GitHub repository'}</dd>
      </div>
    </dl>
  </section>
  {#if repo.fullName}
    <form
      class="panel form-panel"
      onsubmit={(e) => {
        e.preventDefault()
        if (info?.private !== (visibility === 'private')) {
          dialogError = ''
          visibilityConfirm = true
        } else void save()
      }}
    >
      <h2>GitHub settings</h2>
      {#if info}
        <label>Repository name<input bind:value={name} maxlength="100" required /></label>
        <label
          >Description<textarea bind:value={description} maxlength="350" rows="2"></textarea></label
        >
        <label
          >Visibility<select bind:value={visibility}
            ><option value="private">Private</option><option
              disabled={repo.firstPush}
              value="public">Public</option
            ></select
          ></label
        >
        {#if repo.firstPush}<small class="muted"
            >Public visibility becomes available with the first push.</small
          >{/if}
        <div class="form-actions">
          <button
            class="primary"
            disabled={busy ||
              !name.trim() ||
              (name.trim() === info.name &&
                description === (info.description || '') &&
                (visibility === 'private') === info.private)}>Save changes</button
          >
        </div>
      {:else if busy}<p class="muted">Loading repository settings…</p>
      {:else}<div class="form-actions">
          <button type="button" onclick={load}><Icon name="refresh" size={15} />Retry</button
          ><button
            type="button"
            onclick={() => api('openInstall').catch((e) => (error = errorText(e)))}
            >Manage GitHub App access <Icon name="external" size={14} /></button
          >
        </div>{/if}
      {#if error}<p class="inline-error" role="alert">{error}</p>{/if}
    </form>
  {/if}
  <section class="panel form-panel">
    <h2>Remove or delete repository</h2>
    <label
      >Action<select bind:value={deletion}
        >{#each modes as [value, label]}<option
            {value}
            disabled={!repo.fullName && (value === 'remote' || value === 'both')}>{label}</option
          >{/each}</select
      ></label
    >
    <div class="form-actions">
      <button
        class="danger-outline"
        disabled={busy}
        onclick={() => {
          confirmDelete = true
        }}>Review {deletion === 'remove' ? 'removal' : 'deletion'}</button
      >
    </div>
  </section>
</div>
{#if visibilityConfirm}
  <Modal
    title={visibility === 'public' ? 'Make repository public?' : 'Make repository private?'}
    close={() => {
      if (!busy) visibilityConfirm = false
    }}
  >
    <p>
      {visibility === 'public'
        ? 'Anyone will be able to view and copy this repository’s contents and commit history.'
        : 'Future access will be restricted. Anyone who already downloaded this repository can keep their copy.'}
    </p>
    <p class="break"><b>{exact}</b></p>
    {#if dialogError}<p class="inline-error" role="alert">{dialogError}</p>{/if}
    <div class="dialog-actions">
      <button disabled={busy} onclick={() => (visibilityConfirm = false)}>Cancel</button><button
        class="primary"
        disabled={busy}
        onclick={save}>Make {visibility}</button
      >
    </div>
  </Modal>
{/if}
{#if confirmDelete}
  <DeleteRepositoryDialog
    {repo}
    mode={deletion}
    title={modes.find(([value]) => value === deletion)![1]}
    close={() => (confirmDelete = false)}
    {changed}
    {notify}
  />
{/if}

{#if associationConfirm}
  <Modal
    title="Change associated account?"
    close={() => {
      if (!busy) associationConfirm = false
    }}
  >
    <p>
      This repository will appear under <b
        >@{otherAccounts.find((a) => a.id === newAccount)?.login}</b
      >. Local files stay in place. Future GitHub operations will use the selected account.
    </p>
    {#if dialogError}<p class="inline-error" role="alert">{dialogError}</p>{/if}
    <div class="dialog-actions">
      <button disabled={busy} onclick={() => (associationConfirm = false)}>Cancel</button><button
        class="primary"
        disabled={busy}
        onclick={associate}>Change account</button
      >
    </div>
  </Modal>
{/if}
