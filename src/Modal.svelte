<script lang="ts">
  import type { Snippet } from 'svelte'
  import { onMount } from 'svelte'
  import Icon from './Icon.svelte'
  let { title, close, children }: { title: string; close: () => void; children: Snippet } = $props()
  let dialog: HTMLDialogElement
  onMount(() => {
    dialog.showModal()
    return () => dialog.close()
  })
</script>

<dialog
  bind:this={dialog}
  aria-label={title}
  oncancel={(event) => {
    event.preventDefault()
    close()
  }}
>
  <div class="dialog-heading">
    <h2>{title}</h2>
    <button class="icon" aria-label="Close dialog" onclick={close}><Icon name="close" /></button>
  </div>
  {@render children()}
</dialog>
