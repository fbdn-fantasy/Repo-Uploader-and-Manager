<script lang="ts">
  import { onMount } from 'svelte'
  import { listen } from '@tauri-apps/api/event'
  import SyncStatus from './SyncStatus.svelte'
  import {
    api,
    errorText,
    suggestsGitHubAccess,
    type AppState,
    type AutoSyncItem,
    type Repo,
    type Page,
    type Account,
    type Theme,
  } from './api'
  import Icon from './Icon.svelte'
  import UpdateButton from './UpdateButton.svelte'
  import appIcon from '../src-tauri/icons/128x128.png'
  import Auth from './Auth.svelte'
  import Repository from './Repository.svelte'
  import RepoEntry from './RepoEntry.svelte'
  import DeleteRepositoryDialog from './DeleteRepositoryDialog.svelte'
  import Modal from './Modal.svelte'
  let appState = $state<AppState | null>(null),
    page = $state<Page>('repositories'),
    selected = $state(''),
    search = $state('')
  let starting = $state(true),
    busy = $state(false),
    notice = $state(''),
    isError = $state(false),
    offerAccess = $state(false),
    menu = $state(false)
  let signing = $state(false),
    reauth = $state<Account | undefined>(undefined),
    signingOut = $state<Account | null>(null)
  let localDelete = $state<Repo | null>(null)
  let active = $derived(appState?.accounts.find((a) => a.id === appState?.activeAccount))
  let repos = $derived((appState?.repos ?? []).filter((r) => r.accountId === active?.id))
  let repo = $derived(repos.find((r) => r.id === selected))
  let visible = $derived(
    repos.filter((r) =>
      (r.name + ' ' + (r.fullName || '') + ' ' + r.path)
        .toLowerCase()
        .includes(search.toLowerCase()),
    ),
  )
  let nav: [Page, string, string][] = [
    ['repositories', 'Repositories', 'repo'],
    ['new', 'New repository', 'new'],
    ['open', 'Open repository', 'folder'],
    ['clone', 'Clone repository', 'clone'],
    ['accounts', 'Account selection', 'accounts'],
    ['settings', 'Settings', 'settings'],
  ]
  let inAuth = $derived(
    (!active || signing) &&
      page !== 'settings' &&
      (signing || !appState?.accounts.length || page !== 'accounts'),
  )
  let heading = $derived(inAuth ? 'GitHub account' : nav.find((n) => n[0] === page)?.[1] || '')
  let syncById = $state<Record<string, AutoSyncItem>>({})
  let syncRead = 0
  async function refreshSyncStatus() {
    const request = ++syncRead
    const account = appState?.activeAccount
    if (!account || !appState?.autoSyncEnabled) {
      syncById = {}
      return
    }
    const items = await api<AutoSyncItem[]>('autoSyncStatus')
    if (request === syncRead && appState?.activeAccount === account) {
      syncById = Object.fromEntries(items.map((item) => [item.id, item]))
    }
  }
  let timer: ReturnType<typeof setTimeout>
  function notify(message: string, error = false, suggestAccess = true) {
    clearTimeout(timer)
    notice = message
    isError = error
    offerAccess = error && suggestAccess && suggestsGitHubAccess(message)
    if (!error) timer = setTimeout(() => (notice = ''), 6000)
  }
  async function reload() {
    const data = await api<AppState>('state')
    if (appState?.activeAccount !== data.activeAccount) syncById = {}
    appState = data
    void refreshSyncStatus().catch((error) => notify(errorText(error), true, false))
    document.documentElement.dataset.theme = data.theme
    if (
      selected &&
      !data.repos.some((r) => r.id === selected && r.accountId === data.activeAccount)
    )
      selected = ''
  }
  async function init() {
    starting = true
    notice = ''
    try {
      await reload()
    } catch (e) {
      notify(errorText(e), true)
    } finally {
      starting = false
    }
  }
  onMount(() => {
    void init()
    const subscription = listen('auto-sync', () => {
      void refreshSyncStatus().catch((error) => notify(errorText(error), true, false))
    }).then((stop) => {
      // Read again after binding in case a check finished during startup.
      void refreshSyncStatus().catch((error) => notify(errorText(error), true, false))
      return stop
    }).catch((error) => notify(errorText(error), true, false))
    return () => {
      clearTimeout(timer)
      void subscription.then((stop) => stop?.())
    }
  })
  function navigate(next: Page) {
    page = next
    selected = ''
    notice = ''
    search = ''
    menu = false
    signing = false
    reauth = undefined
  }
  function begin(account?: Account) {
    page = 'accounts'
    signing = true
    reauth = account
    menu = false
    notice = ''
  }
  async function signedIn() {
    await reload()
    navigate('repositories')
    notify('GitHub account connected.')
  }
  async function opened(r: Repo, warning?: string | null) {
    await reload()
    page = 'repositories'
    selected = r.id
    search = ''
    menu = false
    notice = ''
    if (warning) notify(warning, true)
  }
  async function accountAction(action: 'switchAccount' | 'signOut', account: Account) {
    if (busy) return
    busy = true
    try {
      await api(action, { accountId: account.id })
      signingOut = null
      await reload()
      navigate(action === 'switchAccount' ? 'repositories' : 'accounts')
    } catch (e) {
      signingOut = null
      notify(errorText(e), true)
    } finally {
      busy = false
    }
  }
  async function setAutoSync(enabled: boolean) {
    if (busy) return
    busy = true
    try {
      await api('setAutoSyncEnabled', { enabled })
      await reload()
    } catch (error) {
      notify(errorText(error), true, false)
    } finally {
      busy = false
    }
  }
  async function changeTheme(value: string) {
    busy = true
    try {
      await api('theme', { theme: value as Theme })
      await reload()
    } catch (e) {
      notify(errorText(e), true)
    } finally {
      busy = false
    }
  }
</script>

<div class="shell">
  <aside class:opened={menu} class="sidebar">
    <div class="brand">
      <img class="app-mark" src={appIcon} alt="" width="40" height="40" /><span
        >Repo Uploader<br />and Manager</span
      >
    </div>
    <nav aria-label="Main navigation">
      {#each nav as [id, label, icon], i}
        {#if i === 4}<div class="nav-divider"></div>{/if}
        <button
          class:current={(inAuth ? 'accounts' : page) === id}
          aria-current={(inAuth ? 'accounts' : page) === id ? 'page' : undefined}
          disabled={!active && i < 4}
          onclick={() => navigate(id)}><Icon name={icon} /><span>{label}</span></button
        >
      {/each}
    </nav>
    <button class="account-foot" onclick={() => navigate('accounts')}
      ><span class="avatar">{active ? active.login.slice(0, 1).toUpperCase() : 'G'}</span><span
        ><b>{active ? '@' + active.login : 'GitHub account'}</b><small
          >{active ? 'Active account' : 'Signed out'}</small
        ></span
      ><Icon name="down" size={13} /></button
    >
    <UpdateButton notify={(message, error) => notify(message, error, false)} closeMenu={() => (menu = false)} />
  </aside>
  {#if menu}<button class="scrim" aria-label="Close navigation" onclick={() => (menu = false)}
    ></button>{/if}
  <main>
    <div class="breadcrumb">
      <button class="menu-toggle icon" aria-label="Open navigation" onclick={() => (menu = !menu)}
        ><Icon name="menu" /></button
      >
      <Icon name={inAuth ? 'accounts' : nav.find((n) => n[0] === page)?.[2] || 'repo'} size={15} />
      {#if repo && page === 'repositories'}<button onclick={() => navigate('repositories')}
          >Repositories</button
        ><span>/</span><strong>{repo.name}</strong>
      {:else}<span>{heading}</span>{/if}
    </div>
    <div class="content">
      {#if notice}<div
          role={isError ? 'alert' : 'status'}
          class={isError ? 'error-banner' : 'notice-banner'}
        >
          <Icon name={isError ? 'info' : 'check'} size={17} /><span>{notice}</span>
          {#if offerAccess}
            <button onclick={() => api('openInstall').catch((e) => notify(errorText(e), true))}>
              Manage GitHub App access
            </button>
          {/if}
          <button
            class="icon"
            aria-label="Dismiss message"
            onclick={() => (notice = '')}><Icon name="close" size={15} /></button
          >
        </div>{/if}
      {#if starting}<p class="muted">Opening workspace…</p>
      {:else if !appState}<button onclick={init}>Retry opening workspace</button>
      {:else if inAuth}
        {#key reauth?.id}<Auth
            accountId={reauth?.id}
            login={reauth?.login}
            done={signedIn}
            back={active || appState.accounts.length ? () => navigate('accounts') : undefined}
          />{/key}
      {:else if page === 'settings'}
        <header class="page-head">
          <div class="section-symbol"><Icon name="settings" /></div>
          <div>
            <h1>Settings</h1>
            <p>Appearance, repository syncing, and application details.</p>
          </div>
        </header>
        <div class="settings-stack">
          <section class="panel setting-line">
            <div>
              <Icon name="monitor" />
              <div>
                <h2>Appearance</h2>
                <p>Application theme</p>
              </div>
            </div>
            <select
              aria-label="Theme"
              value={appState.theme}
              disabled={busy}
              onchange={(e) => changeTheme(e.currentTarget.value)}
              ><option value="system">Follow system</option><option value="light">Light</option
              ><option value="dark">Dark</option></select
            >
          </section>
          <section class="panel setting-line">
            <div>
              <Icon name="refresh" />
              <div>
                <h2 id="auto-sync-label">Automatically sync repositories</h2>
                <p id="auto-sync-description">Check added local repositories for GitHub changes on startup and every 30 minutes. Clean branches update automatically.</p>
              </div>
            </div>
            <button class="auto-sync-switch" role="switch" aria-checked={appState.autoSyncEnabled}
              aria-labelledby="auto-sync-label" aria-describedby="auto-sync-description" disabled={busy}
              onclick={() => setAutoSync(!appState!.autoSyncEnabled)}>
              <span class="switch-track" aria-hidden="true"></span>{appState.autoSyncEnabled ? 'On' : 'Off'}
            </button>
          </section>
          <section class="panel setting-line">
            <div>
              <Icon name="accounts" />
              <div>
                <h2>GitHub accounts</h2>
                <p>{active ? 'Active account: @' + active.login : 'No active account'}</p>
              </div>
            </div>
            <button onclick={() => navigate('accounts')}>Manage accounts</button>
          </section>
          <section class="panel form-panel">
            <h2>System information</h2>
            <dl class="properties">
              <div>
                <dt>Git installation</dt>
                <dd>
                  {appState.gitVersion || 'Git was not found. Install Git and restart this app.'}
                </dd>
              </div>
              <div>
                <dt>Application version</dt>
                <dd>{appState.version}</dd>
              </div>
            </dl>
          </section>
        </div>
      {:else if page === 'accounts'}
        <header class="page-head">
          <div class="section-symbol"><Icon name="accounts" /></div>
          <div>
            <h1>Account selection</h1>
            <p>Manage the GitHub accounts connected to this device.</p>
          </div>
        </header>
        <section class="panel">
          <div class="group-head">
            <h2>GitHub accounts</h2>
            <button class="primary" onclick={() => begin()}
              ><Icon name="plus" size={15} />Add account</button
            >
          </div>
          {#each appState.accounts as a (a.id)}<div class="item-row account-row">
              <span class="avatar">{a.login.slice(0, 1).toUpperCase()}</span><b>@{a.login}</b
              >{#if a.id === active?.id}<span class="tag active">Active</span>{/if}
              <div class="row-actions">
                {#if a.id !== active?.id}<button
                    disabled={busy}
                    onclick={() => accountAction('switchAccount', a)}>Switch</button
                  >{/if}<button disabled={busy} onclick={() => begin(a)}>Reauthorize</button><button
                  class="subtle"
                  disabled={busy}
                  onclick={() => (signingOut = a)}>Sign out</button
                >
              </div>
            </div>{/each}
        </section>
        <section class="panel form-panel install">
          <h2>GitHub App installation</h2>
          <p>
            Install the app on your account and grant access to the repositories you want to manage.
            Organization access may need an administrator’s approval.
          </p>
          <div>
            <button onclick={() => api('openInstall').catch((e) => notify(errorText(e), true))}
              >Manage GitHub App access <Icon name="external" size={14} /></button
            >
          </div>
        </section>
      {:else if page === 'new' || page === 'open' || page === 'clone'}
        {#key page}<RepoEntry mode={page} {opened} changed={reload} />{/key}
      {:else if repo}
        {#key repo.id + repo.path}<Repository
            {repo}
            autoSync={appState.autoSyncEnabled ? syncById[repo.id] : undefined}
            accounts={appState.accounts}
            login={active?.login || ''}
            changed={reload}
            {notify}
          />{/key}
      {:else}
        <header class="repo-heading">
          <div class="heading-main">
            <div class="section-symbol"><Icon name="repo" /></div>
            <div>
              <h1>Repositories</h1>
              <p>Local repositories for @{active?.login}.</p>
            </div>
          </div>
          <button class="primary" onclick={() => navigate('new')}
            ><Icon name="plus" size={16} />New repository</button
          >
        </header>
        {#if !appState.gitVersion}<p class="inline-error" role="alert">
            Git was not found. Install Git and restart the application.
          </p>{/if}
        {#if repos.length}<div class="list-filter">
            <div class="search">
              <Icon name="search" size={16} /><input
                aria-label="Filter local repositories"
                placeholder="Filter repositories"
                bind:value={search}
              />
            </div>
          </div>{/if}
        <section class="panel repo-list">
          <div class="group-head">
            <h2>Local repositories <span class="count">{repos.length}</span></h2>
          </div>
          {#each visible as r (r.id)}
            <div class="repository-row">
              <button
                class="repo-item"
                onclick={() => {
                  selected = r.id
                  notice = ''
                }}
                ><span class="repo-symbol"><Icon name="repo" size={21} /></span><span
                  ><b>{r.name}</b><small>{r.path}</small>
                    {#if appState.autoSyncEnabled && r.fullName && syncById[r.id]}<SyncStatus item={syncById[r.id]} />{/if}</span
                ><Icon name="next" size={17} /></button
              >
              <button
                class="danger-outline"
                aria-label={`Delete local copy of ${r.fullName || r.name}`}
                onclick={() => (localDelete = r)}
                ><Icon name="trash" size={15} />Delete local copy</button
              >
            </div>
          {:else}{#if search}<p class="empty-inline">No matching repositories.</p>
            {:else}<div class="list-empty">
                <span class="empty-icon"><Icon name="folder" size={27} /></span>
                <h3>No local repositories</h3>
                <p>Open a local repository or clone one from GitHub.</p>
                <div class="form-actions">
                  <button onclick={() => navigate('open')}
                    ><Icon name="folder" size={16} />Open repository</button
                  ><button onclick={() => navigate('clone')}
                    ><Icon name="clone" size={16} />Clone repository</button
                  >
                </div>
              </div>{/if}{/each}
        </section>
      {/if}
    </div>
  </main>
</div>
{#if signingOut}
  <Modal
    title="Sign out of GitHub?"
    close={() => {
      if (!busy) signingOut = null
    }}
    ><p>
      Remove the saved credentials for <b>@{signingOut.login}</b> from this device. Local repository folders
      are kept.
    </p>
    <div class="dialog-actions">
      <button disabled={busy} onclick={() => (signingOut = null)}>Cancel</button><button
        class="primary"
        disabled={busy}
        onclick={() => {
          if (signingOut) void accountAction('signOut', signingOut)
        }}>Sign out</button
      >
    </div></Modal
  >
{/if}
{#if localDelete}
  <DeleteRepositoryDialog
    repo={localDelete}
    mode="local"
    title="Delete local copy"
    close={() => (localDelete = null)}
    changed={reload}
    {notify}
  />
{/if}
