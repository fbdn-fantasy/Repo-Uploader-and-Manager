<script lang="ts">
  import { onMount } from 'svelte'
  import { listen } from '@tauri-apps/api/event'
  import {
    api,
    watchRepository,
    errorText,
    type Account,
    type AutoSyncItem,
    type Notice,
    type Repo,
    type Status,
    type Commit,
    type Branch,
    type Info,
  } from './api'
  import Changes from './Changes.svelte'
  import SyncStatus from './SyncStatus.svelte'
  import Branches from './Branches.svelte'
  import RepoSettings from './RepoSettings.svelte'
  import RepositoryFolder from './RepositoryFolder.svelte'
  import Icon from './Icon.svelte'
  import Modal from './Modal.svelte'
  import ConnectRepository from './ConnectRepository.svelte'
  let {
    repo,
    accounts,
    autoSync,
    login,
    changed,
    folderChanged,
    notify,
  }: {
    repo: Repo
    accounts: Account[]
    autoSync?: AutoSyncItem
    login: string
    changed: () => Promise<void>
    folderChanged: () => Promise<void>
    notify: Notice
  } = $props()
  type Tab = 'changes' | 'history' | 'branches' | 'settings'
  const tabs: [Tab, string, string][] = [
    ['changes', 'Changes', 'changes'],
    ['history', 'History', 'history'],
    ['branches', 'Branches', 'branch'],
    ['settings', 'Repository settings', 'settings'],
  ]
  let tab = $state<Tab>('changes'),
    status = $state<Status | null>(null),
    commits = $state<Commit[]>([]),
    branches = $state<Branch[]>([])
  let busy = $state(false),
    error = $state(''),
    watcherError = $state(''),
    loadingTab = $state(false)
  let publishing = $state(false),
    name = $state(''),
    description = $state(''),
    visibility = $state('private'),
    publishError = $state('')
  let connecting = $state(false), folderOpen = $state(false)
  let current = true,
    statusRun = 0,
    tabRun = 0
  async function readStatus() {
    const run = ++statusRun
    try {
      const value = await api<Status>('repoStatus', { id: repo.id })
      if (current && run === statusRun) status = value
    } catch (e) {
      if (current && run === statusRun) error = errorText(e)
    }
  }
  async function readTab() {
    const selected = tab,
      run = ++tabRun
    loadingTab = selected === 'history' || selected === 'branches'
    try {
      if (selected === 'history') {
        const data = await api<Commit[]>('history', { id: repo.id })
        if (current && run === tabRun) commits = data
      }
      if (selected === 'branches') {
        const data = await api<Branch[]>('branches', { id: repo.id })
        if (current && run === tabRun) branches = data
      }
    } catch (e) {
      if (current && run === tabRun) error = errorText(e)
    } finally {
      if (current && run === tabRun) loadingTab = false
    }
  }
  async function refresh() {
    error = ''
    await Promise.all([readStatus(), readTab()])
  }
  function pickTab(next: Tab) {
    tab = next
    error = ''
    void readTab()
  }
  onMount(() => {
    current = true
    let stop: (() => void) | undefined, timer: ReturnType<typeof setTimeout> | undefined
    const id = repo.id
    const subscription = listen<{ id: string; error?: string }>('repo-change', (e) => {
      if (e.payload.id !== id || !current) return
      if (e.payload.error) {
        watcherError = e.payload.error
        return
      }
      clearTimeout(timer)
      timer = setTimeout(() => {
        void readStatus()
        if (tab === 'history' || tab === 'branches') void readTab()
      }, 150)
    })
      .then((unlisten) => {
        if (current) stop = unlisten
        else unlisten()
      })
      .catch((e) => {
        if (current) watcherError = errorText(e)
      })
    void subscription
      .then(() => {
        if (current) return watchRepository(id)
      })
      .catch((e) => {
        if (current) watcherError = errorText(e)
      })
      .then(() => {
        // Read the baseline after installing the watcher, including edits made while opening.
        if (current) return refresh()
      })
    return () => {
      current = false
      clearTimeout(timer)
      stop?.()
      void watchRepository('').catch(() => {})
    }
  })
  async function sync(operation: 'push' | 'pull' | 'fetch', confirmed = false) {
    if (busy) return
    if (operation === 'push' && !repo.fullName) {
      connecting = true
      return
    }
    busy = true
    error = ''
    publishError = ''
    try {
      if (operation === 'push' && repo.firstPush && !confirmed) {
        const info = await api<Info>('repoInfo', { id: repo.id })
        name = info.name
        description = info.description || ''
        visibility = 'private'
        publishing = true
        return
      }
      await api('sync', {
        id: repo.id,
        operation,
        ...(confirmed
          ? { publish: { name: name.trim(), description, private: visibility === 'private' } }
          : {}),
      })
      publishing = false
      await changed()
      await refresh()
      notify(
        operation === 'push'
          ? 'Push completed.'
          : operation === 'pull'
            ? 'Pull completed.'
            : 'Fetch completed.',
      )
    } catch (e) {
      if (confirmed) {
        publishing = false
      }
      const message = errorText(e)
      await changed()
      await readStatus()
      error = message
    } finally {
      busy = false
    }
  }
  async function connected(destination: Repo) {
    connecting = false
    busy = true
    try {
      await changed()
      await refresh()
      notify(`Connected to ${destination.fullName}. Select Push to upload your commits.`)
    } catch (e) {
      error = errorText(e)
    } finally {
      busy = false
    }
  }
  function displayDate(value: string) {
    const d = new Date(value)
    return isNaN(d.getTime()) ? value : d.toLocaleString()
  }
</script>

<header class="repo-heading">
  <div class="heading-main">
    <div class="section-symbol"><Icon name="repo" /></div>
    <div>
      <h1>{repo.name}</h1>
      <p class="path-text">{repo.path}</p>
    </div>
  </div>
  <button
    disabled={busy}
    onclick={() => (folderOpen = true)}
    ><Icon name="folder" size={16} />Open repository folder</button
  >
</header>
<div class="repo-toolbar">
  <div class="branch-label">
    <Icon name="branch" size={16} />{status?.branch || 'Branch unavailable'}
  </div>
  {#if status && status.unpushed > 0}<span class="unpushed">{status.unpushed} unpushed</span>{/if}
  <div class="sync-actions">
    <button disabled={busy || !repo.fullName} onclick={() => sync('fetch')}
      ><Icon name="fetch" size={15} />Fetch</button
    ><button disabled={busy || !repo.fullName} onclick={() => sync('pull')}
      ><Icon name="pull" size={15} />Pull</button
    ><button class="primary" disabled={busy} onclick={() => sync('push')}
      ><Icon name="push" size={15} />Push</button
    >
  </div>
</div>
{#if !repo.fullName}
  <p class="helper-note">Local repository. Select Push to connect it to GitHub.</p>
{/if}
{#if repo.fullName && autoSync}<div class="sync-summary" role="status"><SyncStatus item={autoSync} /></div>{/if}
<nav class="tabs" aria-label="Repository sections">
  {#each tabs as [key, label, icon]}<button
      aria-current={tab === key ? 'page' : undefined}
      class:chosen={tab === key}
      onclick={() => pickTab(key)}><Icon name={icon} size={15} />{label}</button
    >{/each}
</nav>
{#if error}<div class="error-banner" role="alert">
    <Icon name="info" size={16} /><span>{error}</span><button
      class="icon"
      aria-label="Dismiss error"
      onclick={() => (error = '')}><Icon name="close" size={14} /></button
    >
  </div>{/if}
{#if watcherError && watcherError !== error}<p class="inline-error" role="alert">
    {watcherError} Use Refresh to check the working tree.
  </p>{/if}
{#if tab === 'changes'}
  <div class="section-head">
    <h2>Working tree</h2>
    <button class="subtle" disabled={busy} onclick={refresh}
      ><Icon name="refresh" size={15} />Refresh</button
    >
  </div>
  {#if status}<Changes id={repo.id} {status} {refresh} {notify} locked={busy} />{:else if !error}<p
      class="muted"
    >
      Reading Git status…
    </p>{/if}
{:else if tab === 'history'}
  <div class="section-head">
    <h2>Commit history</h2>
    <button class="subtle" onclick={refresh}><Icon name="refresh" size={15} />Refresh</button>
  </div>
  <div class="panel">
    {#if loadingTab}<p class="empty-inline">Loading history…</p>{:else}{#each commits as c (c.hash)}
        <div class="item-row history-row">
          <span class="commit-symbol"><Icon name="commit" /></span>
          <div>
            <b>{c.title}</b><small
              >{c.author} <span aria-hidden="true">·</span> {displayDate(c.date)}</small
            >
          </div>
          <code>{c.hash}</code>
        </div>
      {:else}<p class="empty-inline">No commits on this branch.</p>{/each}{/if}
  </div>
{:else if tab === 'branches'}
  {#if loadingTab}<p class="muted">Loading branches…</p>{:else}<Branches
      id={repo.id}
      items={branches}
      {refresh}
      {notify}
    />{/if}
{:else}<RepoSettings {repo} {accounts} {login} {changed} {notify} locate={() => (folderOpen = true)} />{/if}
{#if publishing}
  <Modal
    title="Confirm first push"
    close={() => {
      if (!busy) publishing = false
    }}
  >
    <p>
      Confirm the GitHub repository details. Your commits will be pushed while the repository is
      private.
    </p>
    <form
      onsubmit={(e) => {
        e.preventDefault()
        void sync('push', true)
      }}
    >
      <label>Repository name<input bind:value={name} required maxlength="100" /></label>
      <label
        >Description<textarea bind:value={description} rows="2" maxlength="350"></textarea></label
      >
      <label
        >Visibility after push<select bind:value={visibility}
          ><option value="private">Private</option><option value="public">Public</option></select
        ></label
      >
      {#if visibility === 'public'}<p class="info-line">
          <Icon name="globe" size={16} />After a successful push, anyone can view and copy the
          contents and commit history.
        </p>{/if}
      {#if publishError}<p class="inline-error" role="alert">{publishError}</p>{/if}
      <div class="dialog-actions">
        <button type="button" disabled={busy} onclick={() => (publishing = false)}>Cancel</button
        ><button disabled={busy || !name.trim()} class="primary"
          ><Icon name="push" size={15} />{busy
            ? 'Pushing…'
            : visibility === 'public'
              ? 'Push and make public'
              : 'Push privately'}</button
        >
      </div>
    </form>
  </Modal>
{/if}
{#if connecting}
  <ConnectRepository {repo} {login} {connected} close={() => (connecting = false)} />
{/if}

{#if folderOpen}
  <RepositoryFolder {repo} close={() => (folderOpen = false)} saved={async (initialized) => {
    await folderChanged()
    notify(initialized ? 'Local Git repository initialized.' : 'Repository folder reconnected.')
  }} />
{/if}
