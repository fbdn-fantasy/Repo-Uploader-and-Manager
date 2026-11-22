<script lang="ts">
  let { name, size = 18 }: { name: string; size?: number } = $props()
  const shapes: Record<string, string> = {
    repo: 'M4 4.5A2.5 2.5 0 0 1 6.5 2H20v19H7a3 3 0 0 1-3-3zm0 13.5a3 3 0 0 1 3-3h13M9 2v7l2-1.5L13 9V2',
    plus: 'M12 5v14M5 12h14',
    new: 'M19 10V4H7a3 3 0 0 0-3 3v11a3 3 0 0 1 3-3h4M4 18a3 3 0 0 0 3 3h4M9 4v5M17.5 14v7M14 17.5h7',
    folder: 'M3 18.5V6a1.5 1.5 0 0 1 1.5-1.5H9l2.5 3H19A1.5 1.5 0 0 1 20.5 9v1M3 19l3-8h16l-3 8z',
    clone: 'M12 3v10m-4-4 4 4 4-4M4 15v5h16v-5',
    accounts:
      'M16 20H2v-2a5 5 0 0 1 5-5h4a5 5 0 0 1 5 5zM13 7a4 4 0 1 1-8 0 4 4 0 0 1 8 0zm5 6a5 5 0 0 1 4 5v2m-5-17a4 4 0 0 1 0 8',
    settings: 'M3 6h7m4 0h7M3 12h11m4 0h3M3 18h3m4 0h11M10 3.5v5m4 1v5m-8 1v5',
    branch:
      'M6 8v8m0 0c0-5 12-1 12-9M8 5a2 2 0 1 1-4 0 2 2 0 0 1 4 0zm0 14a2 2 0 1 1-4 0 2 2 0 0 1 4 0zM20 5a2 2 0 1 1-4 0 2 2 0 0 1 4 0z',
    lock: 'M5 10h14v10H5zM8 10V7a4 4 0 1 1 8 0v3',
    user: 'M16 7a4 4 0 1 1-8 0 4 4 0 0 1 8 0zM4 21v-2a7 7 0 0 1 7-7h2a7 7 0 0 1 7 7v2',
    next: 'm9 5 7 7-7 7',
    down: 'm6 9 6 6 6-6',
    back: 'm15 5-7 7 7 7',
    refresh: 'M20 8a8 8 0 0 0-14-2L3 9m0-5v5h5m-4 7a8 8 0 0 0 14 2l3-3m0 5v-5h-5',
    changes: 'M6 5h15M6 12h15M6 19h15M2 5h.1M2 12h.1M2 19h.1',
    history: 'M3 7v5h5M3 12a9 9 0 1 1 2.5 6M12 6v6l4 2',
    fetch: 'M12 2v12m-5-5 5 5 5-5M4 19h16',
    push: 'M12 20V7m-5 5 5-5 5 5M4 3h16',
    pull: 'M12 3v13m-5-5 5 5 5-5M4 21h16',
    file: 'M6 2h8l5 5v15H6zM14 2v6h5',
    check: 'm4 12 5 5L20 6',
    copy: 'M8 8h12v13H8zM4 16H3V3h12v1',
    close: 'm6 6 12 12M18 6 6 18',
    external: 'M14 3h7v7m0-7-11 11m0-9H4v16h16v-6',
    info: 'M22 12a10 10 0 1 1-20 0 10 10 0 0 1 20 0zm-10-1v6m0-10v.2',
    globe: 'M22 12a10 10 0 1 1-20 0 10 10 0 0 1 20 0zm-20 0h20M12 2c5 6 5 14 0 20-5-6-5-14 0-20',
    commit: 'M8 12H2m20 0h-6m0 0a4 4 0 1 1-8 0 4 4 0 0 1 8 0z',
    search: 'M17 10a7 7 0 1 1-14 0 7 7 0 0 1 14 0zm-2 5 6 6',
    shield: 'M12 2 4 5v6c0 6 8 11 8 11s8-5 8-11V5zM8 12l3 3 5-6',
    terminal: 'M2 4h20v16H2zm4 5 3 3-3 3m5 0h5',
    monitor: 'M3 3h18v13H3zm9 13v5m-5 0h10',
    edit: 'm4 16 12-12 4 4L8 20l-5 1z',
    trash: 'M3 6h18M9 6V3h6v3M6 6l1 15h10l1-15M10 10v7m4-7v7',
    signin: 'M13 3h7v18h-7M3 12h12m-5-5 5 5-5 5',
    code: 'm8 7-5 5 5 5m8-10 5 5-5 5',
    menu: 'M3 5h18M3 12h18M3 19h18',
  }
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="1.65"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"><path d={shapes[name] || shapes.repo} /></svg
>
