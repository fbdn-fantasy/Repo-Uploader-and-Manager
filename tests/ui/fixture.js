// Synthetic in-memory state for the browser smoke test. Never imported by the app.
const repo = {
  id: 'r1',
  name: 'sample',
  path: '/projects/sample',
  accountId: '1',
  fullName: null,
  firstPush: false,
}
const account = { id: '1', login: 'sample-owner' }
const app = {
  accounts: [],
  activeAccount: null,
  repos: [repo],
  theme: 'system',
  autoSyncEnabled: true,
  gitVersion: 'git version 2',
  version: '1.0.0',
}
const status = {
  branch: 'main',
  unpushed: 0,
  changes: [
    { path: 'src/one.txt', index: '?', worktree: '?' },
    { path: 'two.txt', index: '?', worktree: '?' },
  ],
  authorName: account.login,
  authorEmail: '1+sample-owner@users.noreply.github.com',
}
const calls = []
const tests = []
let syncMessage = 'Local branch is up to date.'
let nextCallback = 1
const callbacks = new Map()
const listeners = new Map()
function emit(event, payload) {
  for (const [id, subscribed] of listeners) {
    if (subscribed === event) callbacks.get(id)?.({ event, id, payload })
  }
}
let authorize = false
const info = {
  fullName: 'sample-owner/sample',
  name: repo.name,
  private: true,
  description: 'Sample repository',
}
window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} }
window.__TAURI_INTERNALS__ = {
  transformCallback(handler) {
    const id = nextCallback++
    callbacks.set(id, handler)
    return id
  },
  unregisterCallback(id) { callbacks.delete(id) },
  async invoke(cmd, request) {
    if (cmd === 'plugin:event|listen') {
      listeners.set(request.handler, request.event)
      return request.handler
    }
    if (cmd === 'plugin:event|unlisten') {
      listeners.delete(request.eventId)
      callbacks.delete(request.eventId)
      return
    }
    const { action, args } = request
    calls.push({ action, args })
    switch (action) {
      case 'state':
        return structuredClone(app)
      case 'autoSyncStatus':
        return app.repos.filter((r) => r.fullName && r.accountId === app.activeAccount)
          .map((r) => ({ id: r.id, status: r.firstPush ? 'skipped' : 'current',
            message: r.firstPush ? 'Waiting for first push.' : syncMessage, checkedAt: 1_800_000_000 }))
      case 'checkForUpdates':
        return "You're up to date (V1.0.0)."
      case 'authStart':
        return { session: '1', userCode: 'DEMO-CODE' }
      case 'authPoll':
        if (authorize) {
          app.accounts = [account, { id: '2', login: 'other-owner' }]
          app.activeAccount = '1'
        }
        return { done: authorize }
      case 'repoStatus':
        return structuredClone(status)
      case 'repoCreate': {
        if (args.name === 'sample') return { repo, warning: null }
        const retained = { ...repo, path: '/projects/' + args.name, id: 'r2', name: args.name, fullName: null, firstPush: false }
        app.repos.push(retained)
        return { repo: retained, warning: 'Local repository retained. GitHub denied permission to create the repository.' }
      }
      case 'repoConnect': {
        if (args.target === 'denied/private') throw Error('Check GitHub App access for this repository.')
        const local = app.repos.find((item) => item.id === args.id)
        local.fullName = args.mode === 'create' ? account.login + '/' + args.target : args.target
        local.firstPush = args.mode === 'create'
        return structuredClone(local)
      }
      case 'repoInfo':
        return { ...info }
      case 'stage':
      case 'unstage':
        for (const p of args.paths) {
          const c = status.changes.find((x) => x.path === p)
          c.index = action === 'stage' ? 'A' : '?'
          c.worktree = action === 'stage' ? ' ' : '?'
        }
        break
      case 'setAuthor':
        status.authorName = args.name
        status.authorEmail = args.email
        break
      case 'scan':
        return ['src/one.txt: possible sensitive content']
      case 'commit':
        status.changes = status.changes.filter((x) => x.index !== 'A')
        status.unpushed++
        break
      case 'sync':
        if (args.operation === 'push') repo.firstPush = false
        break
      case 'history':
        return [
          {
            hash: 'abc1234',
            title: 'First commit',
            author: 'Example',
            date: '2026-01-01T12:30:00Z',
          },
        ]
      case 'branches':
        return [
          { name: 'main', current: true },
          { name: 'feature', current: false },
        ]
      case 'setAutoSyncEnabled':
        app.autoSyncEnabled = args.enabled
        break
      case 'theme':
        app.theme = args.theme
        break
      case 'switchAccount':
        app.activeAccount = args.accountId
        break
      case 'remoteList':
        return [info]
      case 'repoDelete':
        if (args.mode !== 'local') throw Error('Unexpected deletion target')
        app.repos = app.repos.filter((item) => item.id !== args.id)
        break
      case 'repoUpdate':
        Object.assign(info, args)
        break
      case 'pickFolder':
        return '/projects/relocated-sample'
      case 'repoRelocate':
        repo.path = args.path
        break
      case 'repoAssociate':
        repo.accountId = args.accountId
        break
      case 'repoWatch':
      case 'authCancel':
      case 'branch':
        break
      default:
        throw Error('Unexpected IPC action ' + action)
    }
  },
}
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
const text = (node) => node?.textContent.trim().replace(/\s+/g, ' ')
const buttons = (name, scope = document) =>
  Array.from(scope.querySelectorAll('button')).filter((b) => text(b) === name)
async function wait(fn, label) {
  for (let i = 0; i < 150; i++) {
    const v = fn()
    if (v) return v
    await sleep(20)
  }
  throw Error('Missing ' + label + ' (' + text(document.querySelector('h1')) + ')')
}
const heading = (name) =>
  wait(() => Array.from(document.querySelectorAll('h1,h2,h3')).find((h) => text(h) === name), name)
async function click(name, scope = document) {
  const node = await wait(() => buttons(name, scope).find((b) => !b.disabled), 'enabled ' + name)
  node.click()
  await sleep(30)
}
function input(node, value) {
  node.value = value
  node.dispatchEvent(new Event('input', { bubbles: true }))
  node.dispatchEvent(new Event('change', { bubbles: true }))
}
function check(condition, message) {
  if (!condition) throw Error(message)
}
const count = (action) => calls.filter((c) => c.action === action).length
const last = (action) => calls.findLast((c) => c.action === action)?.args

async function run() {
  await heading('Sign in with GitHub')
  await wait(() => document.querySelector('.brand img').naturalWidth > 0, 'loaded application icon')
  check(count('checkForUpdates') === 0, 'Application updates must be checked manually')
  await click('Check For Updates')
  await wait(() => document.querySelector('.notice-banner')?.textContent.includes("up to date"), 'updater status')
  check(count('checkForUpdates') === 1, 'Expected exactly one app update check')
  tests.push('Manual app update check while signed out')
  await click('Sign in with GitHub')
  await heading('Authorize this device')
  check(
    document.querySelector('input[readonly]').value === 'DEMO-CODE',
    'Device code not displayed',
  )
  await click('Cancel')
  await heading('Sign in with GitHub')
  check(count('authCancel') === 1, 'Cancel did not reach backend')
  tests.push('Sign-in screen, supplied icon, device code, cancellation')
  authorize = true
  await click('Sign in with GitHub')
  await heading('Repositories')
  document.querySelector('.repo-item').click()
  await heading('Working tree')
  const group = document.querySelector('.change-group')
  group.querySelector('input[type=checkbox]').click()
  await click('Stage selected', group)
  check(last('stage').paths.join() === 'src/one.txt', 'Unexpected staged paths')
  let staged = document.querySelectorAll('.change-group')[1]
  await click('Select all', staged)
  await click('Unstage selected', staged)
  check(last('unstage').paths.join() === 'src/one.txt', 'Unexpected unstaged paths')
  group.querySelector('input[type=checkbox]').click()
  await click('Stage selected', group)
  tests.push('Selective staging, unstaging, nested pathname')
  input(document.querySelector('input[placeholder="Summary of changes"]'), 'First commit')
  await click('Commit staged changes')
  await heading('Review sensitive files')
  check(count('setAuthor') === 0, 'Signed-in account was asked to configure a Git author')
  await click('Cancel', document.querySelector('dialog'))
  check(count('commit') === 0, 'Cancellation committed files')
  await click('Commit staged changes')
  await click('Commit anyway', document.querySelector('dialog'))
  check(last('commit').allowSensitive, 'No warning confirmation')
  check(count('sync') === 0, 'Commit triggered sync')
  tests.push('Signed-in author, staged scan, warning cancellation and confirmation, manual push')
  await click('Push')
  await heading('Connect to GitHub')
  check(document.querySelector('dialog input').value === 'sample-owner/sample', 'Expected a matching destination suggestion')
  input(document.querySelector('dialog input'), 'denied/private')
  await click('Connect repository')
  check(buttons('Manage GitHub App access', document.querySelector('dialog')).length === 1, 'Missing access recovery')
  await click('Cancel', document.querySelector('dialog'))
  check(count('sync') === 0, 'Attempted connection uploaded commits')
  await click('Push')
  await heading('Connect to GitHub')
  input(document.querySelector('dialog select'), 'create')
  await click('Create and connect')
  check(last('repoConnect').mode === 'create' && last('repoConnect').target === 'sample', 'Wrong private creation target')
  check(count('sync') === 0, 'Private repository creation uploaded commits')
  await wait(() => document.querySelector('.sync-summary')?.textContent.includes('Waiting for first push.'), 'unpublished repository sync status')
  tests.push('Repository automatic sync status, private creation, connection permission recovery')
  await click('Push')
  await heading('Confirm first push')
  check(
    document.querySelector('dialog select').value === 'private',
    'First push was not private by default',
  )
  input(document.querySelector('dialog select'), 'public')
  await click('Push and make public')
  check(last('sync').publish.private === false, 'Visibility choice missing')
  syncMessage = 'Local branch updated from GitHub.'
  emit('auto-sync', null)
  await wait(() => document.querySelector('.sync-summary')?.textContent.includes(syncMessage), 'background sync completion')
  tests.push('Background sync status refreshes in the open repository')
  await click('Push')
  check(!last('sync').publish, 'Later push reopened publishing')
  await click('Fetch')
  await click('Pull')
  await click('History')
  await heading('Commit history')
  check(text(document.querySelector('.history-row')).includes('abc1234'), 'History hash missing')
  await click('Branches')
  await heading('Local branches')
  await click('New branch')
  input(document.querySelector('dialog input'), 'topic')
  await click('Create branch')
  check(last('branch').name === 'topic', 'Create branch mismatch')
  tests.push('First push dialog, subsequent push, fetch, pull, history, branch creation')
  await click('Repository settings')
  await heading('GitHub settings')
  input(document.querySelector('form select'), 'public')
  await click('Save changes')
  await heading('Make repository public?')
  await click('Cancel', document.querySelector('dialog'))
  check(!count('repoUpdate'), 'Cancelled visibility update was sent')
  await click('Save changes')
  await click('Make public')
  check(last('repoUpdate').private === false, 'Visibility update missing')
  await click('Review removal')
  await heading('Remove from this app')
  check(
    buttons('Remove repository', document.querySelector('dialog'))[0].disabled,
    'Deletion confirmation missing',
  )
  await click('Cancel', document.querySelector('dialog'))
  check(!count('repoDelete'), 'Cancelled removal was sent')
  tests.push('Visibility confirmation, cancellation, destructive target gate')
  input(document.querySelector('select[aria-label="New associated account"]'), '2')
  await click('Change')
  await heading('Change associated account?')
  await click('Cancel', document.querySelector('dialog'))
  check(!count('repoAssociate'), 'Cancelled account reassociation was sent')
  await click('Locate moved folder')
  check(
    last('repoRelocate').path === '/projects/relocated-sample',
    'Selected folder was not relinked',
  )
  await heading('Working tree')
  tests.push('Repository relocation, account reassociation confirmation')
  await click('New repository', document.querySelector('.sidebar'))
  await heading('Repository details')
  check(
    !Array.from(document.querySelectorAll('option')).some((o) => o.text === 'Public'),
    'Public creation option found',
  )
  check(
    document.querySelectorAll('.field-pair select')[0].options.length === 9,
    'License choices missing',
  )
  const description = document.getElementById('repo-description')
  input(description, 'a'.repeat(351))
  check(description.value === 'a'.repeat(350), 'Description exceeded 350 characters')
  const symbols = '😀'.repeat(350)
  input(description, symbols)
  await wait(() => text(document.getElementById('description-count')) === '350 / 350 characters', 'description limit')
  check(description.value === symbols, 'Unicode characters counted as multiple characters')
  description.setRangeText('extra', 2, 2, 'end')
  description.dispatchEvent(new InputEvent('input', { bubbles: true, inputType: 'insertFromPaste' }))
  check(description.value === symbols, 'Overflowing paste changed existing description text')
  input(document.querySelector('input[placeholder="my-project"]'), 'sample')
  await click('Browse')
  await click('Create private repository')
  check(last('repoCreate').description === symbols, 'Submission changed the 350-character description')
  tests.push('Repository description limit, Unicode, paste and creation payload')
  await click('New repository', document.querySelector('.sidebar'))
  await heading('Repository details')
  input(document.querySelector('input[placeholder="my-project"]'), 'retained')
  await click('Browse')
  await click('Create private repository')
  await heading('retained')
  check(text(document.querySelector('[role=alert]')).includes('Local repository retained'), 'Missing partial creation warning')
  check(buttons('Manage GitHub App access').length === 1, 'Missing access settings in warning')
  const syncsBeforeConnect = count('sync')
  await click('Push')
  await heading('Connect to GitHub')
  input(document.querySelector('dialog input'), 'sample-owner/private-project')
  await click('Connect repository')
  check(last('repoConnect').mode === 'existing' && last('repoConnect').target === 'sample-owner/private-project', 'Incorrect existing target')
  check(count('sync') === syncsBeforeConnect, 'Connecting existing repository uploaded commits')
  await click('Push')
  check(last('sync').id === 'r2' && last('sync').operation === 'push', 'Connected checkout push failed')
  tests.push('Retained checkout after GitHub error, existing private destination, manual push')
  await click('Repositories', document.querySelector('.sidebar'))
  const localDelete = await wait(() =>
    document.querySelector('button[aria-label="Delete local copy of sample-owner/private-project"]'), 'local delete action')
  localDelete.click()
  await heading('Delete local copy')
  let dialog = document.querySelector('dialog')
  check(text(dialog).includes('/projects/retained') && text(dialog).includes('will stay on GitHub'), 'Missing local deletion target or remote explanation')
  input(dialog.querySelector('input'), 'retained')
  check(buttons('Delete local copy', dialog)[0].disabled, 'Wrong name enabled local deletion')
  await click('Cancel', dialog)
  check(!count('repoDelete'), 'Cancellation deleted a checkout')
  localDelete.click()
  await heading('Delete local copy')
  dialog = document.querySelector('dialog')
  input(dialog.querySelector('input'), 'sample-owner/private-project')
  await click('Delete local copy', dialog)
  check(last('repoDelete').mode === 'local' && last('repoDelete').id === 'r2', 'Unexpected deletion target')
  check(document.querySelectorAll('.repo-item').length === 1 && text(document.querySelector('.repo-item')).includes('sample'), 'Wrong repository removed from list')
  tests.push('Local copy deletion from repository list, target path, typed confirmation and cancellation')
  await click('Clone repository', document.querySelector('.sidebar'))
  await heading('GitHub repositories')
  await wait(() => document.querySelector('.remote-row'), 'clone repository choice')
  await click('Settings', document.querySelector('.sidebar'))
  input(document.querySelector('select'), 'dark')
  await wait(() => document.documentElement.dataset.theme === 'dark', 'dark theme')
  const toggle = () => document.querySelector('button[role="switch"]')
  check(toggle()?.getAttribute('aria-checked') === 'true', 'Automatic sync default should be on')
  toggle().click()
  await wait(() => toggle()?.getAttribute('aria-checked') === 'false', 'automatic sync disabled')
  check(last('setAutoSyncEnabled').enabled === false, 'Switch must persist the disabled preference')
  await click('Repositories', document.querySelector('.sidebar'))
  check(!document.querySelector('.sync-result'), 'Disabled automatic sync status was displayed')
  await click('Settings', document.querySelector('.sidebar'))
  check(toggle()?.getAttribute('aria-checked') === 'false', 'Preference was lost on navigation')
  toggle().click()
  await wait(() => toggle()?.getAttribute('aria-checked') === 'true', 'automatic sync enabled')
  tests.push('Settings turns automatic repository sync off and on')
  await click('Manage accounts')
  await heading('Account selection')
  await click('Switch')
  await heading('No local repositories')
  check(!document.querySelector('.repo-item'), 'Foreign repository rendered')
  tests.push('Private creation, templates, clone list, theme change, account separation')
}
window.addEventListener('load', async () => {
  let error
  try {
    await run()
  } catch (e) {
    error = e.message
  }
  await fetch('/report', { method: 'POST', body: JSON.stringify({ tests, error }) })
})
