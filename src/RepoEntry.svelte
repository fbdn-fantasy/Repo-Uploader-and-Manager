<script lang="ts">
  import { onMount } from 'svelte'
  import { api, errorText, suggestsGitHubAccess, type Repo, type Remote } from './api'
  import Icon from './Icon.svelte'
  let {
    mode,
    opened,
    changed,
  }: {
    changed: () => Promise<void>
    mode: 'new' | 'open' | 'clone'
    opened: (repo: Repo, warning?: string | null) => Promise<void>
  } = $props()
  let name = $state(''),
    description = $state(''),
    parent = $state(''),
    readme = $state(false)
  let license = $state(''),
    gitignore = $state(''),
    busy = $state(false),
    error = $state('')
  let remotes = $state<Remote[]>([]),
    fullName = $state(''),
    query = $state(''),
    loading = $state(false)
  let filtered = $derived(
    remotes.filter((r) => r.fullName.toLowerCase().includes(query.toLowerCase())),
  )
  const descriptionLimit = 350
  function updateDescription(field: HTMLInputElement) {
    // The backend counts Unicode code points. HTML maxlength counts UTF-16 units.
    if (Array.from(field.value).length > descriptionLimit) {
      const caret = field.selectionEnd ?? field.value.length
      const suffix = Array.from(field.value.slice(caret)).slice(0, descriptionLimit)
      const prefix = Array.from(field.value.slice(0, caret))
        .slice(0, descriptionLimit - suffix.length)
        .join('')
      field.value = prefix + suffix.join('')
      field.setSelectionRange(prefix.length, prefix.length)
    }
    description = field.value
  }
  const licenses = [
    ['', 'No license'],
    ['apache-2.0', 'Apache License 2.0'],
    ['mit', 'MIT'],
    ['gpl-3.0', 'GPL v3.0'],
    ['lgpl-3.0', 'LGPL v3.0'],
    ['agpl-3.0', 'AGPL v3.0'],
    ['bsd-3-clause', 'BSD 3-Clause'],
    ['mpl-2.0', 'Mozilla Public License 2.0'],
    ['unlicense', 'The Unlicense'],
  ]
  async function load() {
    loading = true
    error = ''
    try {
      remotes = await api<Remote[]>('remoteList')
    } catch (e) {
      error = errorText(e)
    } finally {
      loading = false
    }
  }
  onMount(() => {
    if (mode === 'clone') void load()
  })
  async function browse() {
    busy = true
    error = ''
    try {
      const path = await api<string | null>('pickFolder')
      if (path) parent = path
    } catch (e) {
      error = errorText(e)
    } finally {
      busy = false
    }
  }
  async function submit(e: SubmitEvent) {
    e.preventDefault()
    if (busy) return
    busy = true
    error = ''
    try {
      if (mode === 'new') {
        const { repo, warning } = await api<{ repo: Repo; warning: string | null }>('repoCreate', {
          name: name.trim(), description, parent, readme, license, gitignore,
        })
        await opened(repo, warning)
      } else {
        const repo = mode === 'open'
          ? await api<Repo>('repoOpen', { path: parent })
          : await api<Repo>('repoClone', { fullName, parent })
        await opened(repo)
      }
    } catch (e) {
      error = errorText(e)
      await changed().catch(() => {})
    } finally {
      busy = false
    }
  }
</script>

<header class="page-head">
  <div class="section-symbol">
    <Icon name={mode === 'new' ? 'new' : mode === 'clone' ? 'clone' : 'folder'} />
  </div>
  <div>
    <h1>
      {mode === 'new' ? 'New repository' : mode === 'open' ? 'Open repository' : 'Clone repository'}
    </h1>
    <p>
      {mode === 'new'
        ? 'Create a private GitHub repository and a local working folder.'
        : mode === 'open'
          ? 'Add an existing local Git repository to this account.'
          : 'Download a GitHub repository to a local folder.'}
    </p>
  </div>
</header>
<form class="panel form-panel" onsubmit={submit}>
  {#if mode === 'new'}
    <h2>Repository details</h2>
    <label
      >Repository name<input
        bind:value={name}
        required
        maxlength="100"
        placeholder="my-project"
        spellcheck="false"
      /></label
    >
    <label
      >Description <span class="optional">Optional</span><input
        id="repo-description"
        aria-describedby="description-count"
        value={description}
        oninput={(event) => {
          if (!(event instanceof InputEvent && event.isComposing)) updateDescription(event.currentTarget)
        }}
        oncompositionend={(event) => updateDescription(event.currentTarget)}
      /></label
    >
    <small id="description-count" class="muted">
      {Array.from(description).length} / {descriptionLimit} characters
    </small>
    <div class="info-line">
      <Icon name="lock" size={17} /><span
        >New repositories are private. You can change visibility after your first push.</span
      >
    </div>
  {:else if mode === 'clone'}
    <div class="row-between">
      <h2>GitHub repositories</h2>
      <button type="button" onclick={load} disabled={loading || busy}
        ><Icon name="refresh" size={15} />Refresh</button
      >
    </div>
    <div class="search">
      <Icon name="search" size={16} /><input
        aria-label="Filter GitHub repositories"
        placeholder="Filter repositories"
        bind:value={query}
      />
    </div>
    <div class="remote-list">
      {#if loading}<p class="muted">Loading repositories…</p>{:else}
        {#each filtered as r (r.fullName)}
          <label class="remote-row"
            ><input type="radio" name="remote" value={r.fullName} bind:group={fullName} /><span
              ><b>{r.fullName}</b>{#if r.description}<small>{r.description}</small>{/if}</span
            ><Icon name={r.private ? 'lock' : 'globe'} size={15} /></label
          >
        {:else}<p class="muted">
            {query ? 'No matching repositories.' : 'No accessible repositories found.'}
          </p>{/each}{/if}
    </div>
    <button
      type="button"
      class="text-button"
      onclick={() => api('openInstall').catch((e) => (error = errorText(e)))}
      >Manage GitHub App access <Icon name="external" size={14} /></button
    >
  {:else}
    <h2>Local repository</h2>
  {/if}
  <label
    >{mode === 'open' ? 'Repository folder' : 'Local parent folder'}<span class="path-field"
      ><input
        aria-label="Selected directory"
        bind:value={parent}
        placeholder="Choose a folder"
        required
      /><button type="button" disabled={busy} onclick={browse}
        ><Icon name="folder" size={16} /> Browse</button
      ></span
    ></label
  >
  {#if mode === 'new'}
    <div class="subsection">
      <h3>Initial files</h3>
      <label class="check"><input type="checkbox" bind:checked={readme} />Add a README</label>
      <div class="field-pair">
        <label
          >License<select bind:value={license}
            >{#each licenses as [value, label]}<option {value}>{label}</option>{/each}</select
          ></label
        ><label
          >.gitignore template<select bind:value={gitignore}
            ><option value="">No template</option>{#each ['Rust', 'Node', 'Python'] as t}<option
                value={t}>{t}</option
              >{/each}</select
          ></label
        >
      </div>
    </div>
  {/if}
  {#if error}<p class="inline-error" role="alert">{error}</p>
    {#if mode === 'new' && suggestsGitHubAccess(error)}<button
        type="button"
        class="text-button"
        onclick={() => api('openInstall').catch((e) => (error = errorText(e)))}
        >Manage GitHub App access <Icon name="external" size={14} /></button
      >{/if}{/if}
  <div class="form-actions">
    <button
      class="primary"
      disabled={busy ||
        !parent.trim() ||
        (mode === 'new' && !name.trim()) ||
        (mode === 'clone' && !fullName)}
      ><Icon
        name={mode === 'new' ? 'plus' : mode === 'clone' ? 'clone' : 'folder'}
        size={16}
      />{busy
        ? 'Working…'
        : mode === 'new'
          ? 'Create private repository'
          : mode === 'open'
            ? 'Open repository'
            : 'Clone repository'}</button
    >
  </div>
</form>
