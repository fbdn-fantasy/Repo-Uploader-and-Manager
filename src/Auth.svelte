<script lang="ts">
  import { onDestroy } from 'svelte'
  import { api, errorText, type Device } from './api'
  import Icon from './Icon.svelte'
  let {
    accountId,
    login,
    done,
    back,
  }: {
    accountId?: string
    login?: string
    done: () => Promise<void>
    back?: () => void
  } = $props()
  let device = $state<Device | null>(null)
  let pending = $state(false),
    error = $state(''),
    copied = $state(false)
  let generation = 0
  let codeInput = $state<HTMLInputElement>()
  function cancel() {
    generation++
    const previous = device
    device = null
    pending = false
    copied = false
    if (previous) void api('authCancel', { session: previous.session }).catch(() => {})
  }
  onDestroy(cancel)
  async function start() {
    const run = ++generation
    pending = true
    error = ''
    try {
      const next = await api<Device>('authStart', accountId ? { accountId } : {})
      if (run !== generation) {
        await api('authCancel', { session: next.session })
        return
      }
      device = next
      while (run === generation) {
        const poll = await api<{ done: boolean }>('authPoll', { session: next.session })
        if (run !== generation) return
        if (poll.done) {
          device = null
          await done()
          return
        }
        await new Promise((resolve) => setTimeout(resolve, 1000))
      }
    } catch (e) {
      if (run === generation) {
        cancel()
        error = errorText(e)
      }
    } finally {
      if (run === generation) pending = false
    }
  }
  async function copyCode() {
    if (!device) return
    try {
      await navigator.clipboard.writeText(device.userCode)
      copied = true
    } catch {
      codeInput?.focus()
      codeInput?.select()
      copied = document.execCommand('copy')
      if (!copied) error = 'The code is selected. Use your system copy shortcut to copy it.'
    }
  }
</script>

<header class="page-head">
  <div class="section-symbol"><Icon name="accounts" /></div>
  <div>
    <h1>{login ? 'Reauthorize account' : 'GitHub account'}</h1>
    <p>
      {login
        ? 'Reconnect @' + login + ' to continue.'
        : 'Connect an account to manage your repositories.'}
    </p>
  </div>
</header>
<div class="auth-card panel">
  <div class="provider-head">
    <span class="github-mark"
      ><svg viewBox="0 0 24 24" width="30" height="30" fill="currentColor" aria-hidden="true"
        ><path
          d="M12 .9A11.1 11.1 0 0 0 8.5 22.5c.56.1.76-.24.76-.54v-2.08c-3.1.67-3.75-1.31-3.75-1.31-.51-1.28-1.24-1.62-1.24-1.62-1.01-.69.08-.68.08-.68 1.12.08 1.71 1.15 1.71 1.15 1 1.7 2.61 1.21 3.25.92.1-.72.39-1.21.71-1.49-2.48-.28-5.08-1.24-5.08-5.52 0-1.22.44-2.21 1.15-2.99-.11-.29-.5-1.42.11-2.95 0 0 .94-.3 3.06 1.14a10.6 10.6 0 0 1 5.56 0c2.12-1.44 3.06-1.14 3.06-1.14.61 1.53.22 2.66.11 2.95a4.3 4.3 0 0 1 1.15 2.99c0 4.29-2.61 5.23-5.09 5.51.4.35.76 1.03.76 2.08v3.04c0 .3.2.65.77.54A11.1 11.1 0 0 0 12 .9Z"
        /></svg
      ></span
    >
    <div>
      <h2>{device ? 'Authorize this device' : 'Sign in with GitHub'}</h2>
      <p>GitHub.com</p>
    </div>
  </div>
  {#if device}
    <p>Enter this temporary code on GitHub and approve access.</p>
    <div class="device">
      <input
        aria-label="Device authorization code"
        bind:this={codeInput}
        value={device.userCode}
        readonly
      /><button onclick={copyCode}
        ><Icon name={copied ? 'check' : 'copy'} />{copied ? 'Copied' : 'Copy code'}</button
      >
    </div>
    <button
      class="text-button"
      onclick={() => api('openDevice').catch((e) => (error = errorText(e)))}
      >Open GitHub authorization page <Icon name="external" size={14} /></button
    >
    <div class="auth-footer">
      <span class="working"><i class="spinner"></i>Waiting for GitHub…</span><button
        onclick={cancel}>Cancel</button
      >
    </div>
  {:else}
    <p>Your browser will open GitHub to authorize this device.</p>
    <div class="form-actions">
      <button class="primary" disabled={pending} onclick={start}
        >{pending ? 'Requesting device code…' : 'Sign in with GitHub'}
        <Icon name="external" size={15} /></button
      >{#if back}<button onclick={back}>Back to accounts</button>{/if}
    </div>
  {/if}
  {#if error}<p class="inline-error" role="alert">{error}</p>{/if}
</div>
