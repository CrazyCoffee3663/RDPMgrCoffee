<script lang="ts" module>
  const P: Record<string, string> = {
    monitor: "M3 4h18v12H3z M8 20h8 M12 16v4",
    terminal: "M3 4h18v16H3z M7 9l3 3-3 3 M12 15h5",
    plug: "M9 2v6 M15 2v6 M6 8h12v4a6 6 0 0 1-12 0z M12 18v4",
    cable: "M4 9h4v6H4z M16 9h4v6h-4z M8 12h8 M6 15v5 M18 4v5",
    folder: "M3 6a1 1 0 0 1 1-1h5l2 2h9a1 1 0 0 1 1 1v10a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1z",
    folderOpen: "M3 7V6a1 1 0 0 1 1-1h5l2 2h8a1 1 0 0 1 1 1v2 M3 19l2.5-8h16L19 19z",
    building: "M4 21V4h10v17 M14 9h6v12 M2 21h20 M7 8h4 M7 12h4 M7 16h4 M17 13h1 M17 17h1",
    key: "M14.5 9.5a4.5 4.5 0 1 0-4 4.47V14h2v2h2v2h3v-3l-3.5-3.5 M7.5 9.5h.01",
    user: "M12 12a4 4 0 1 0 0-8 4 4 0 0 0 0 8z M4 21a8 8 0 0 1 16 0",
    lock: "M5 11h14v10H5z M8 11V7a4 4 0 0 1 8 0v4",
    unlock: "M5 11h14v10H5z M8 11V7a4 4 0 0 1 7.5-2",
    search: "M11 18a7 7 0 1 0 0-14 7 7 0 0 0 0 14z M21 21l-5-5",
    plus: "M12 5v14 M5 12h14",
    folderPlus: "M3 6a1 1 0 0 1 1-1h5l2 2h9a1 1 0 0 1 1 1v10a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1z M12 10v6 M9 13h6",
    refresh: "M20 12a8 8 0 1 1-2.34-5.66L20 8.5 M20 3v5.5h-5.5",
    settings:
      "M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6z M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 0 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 0 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 0 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 0 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z",
    x: "M6 6l12 12 M18 6L6 18",
    chevron: "M9 6l6 6-6 6",
    chevronDown: "M6 9l6 6 6-6",
    edit: "M4 20h4L19 9l-4-4L4 16z M14 6l4 4",
    trash: "M4 7h16 M9 7V4h6v3 M6 7l1 13h10l1-13 M10 11v6 M14 11v6",
    copy: "M9 9h11v11H9z M5 15H4V4h11v1",
    eye: "M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7S2 12 2 12z M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6z",
    eyeOff: "M3 3l18 18 M10.6 5.1A10 10 0 0 1 12 5c6.5 0 10 7 10 7a17 17 0 0 1-3.2 4 M6.6 6.6A17 17 0 0 0 2 12s3.5 7 10 7a9.7 9.7 0 0 0 5.4-1.6 M9.9 9.9a3 3 0 0 0 4.2 4.2",
    dice: "M4 4h16v16H4z M8.5 8.5h.01 M15.5 8.5h.01 M12 12h.01 M8.5 15.5h.01 M15.5 15.5h.01",
    play: "M7 4l13 8-13 8z",
    star: "M12 3l2.8 5.7 6.2.9-4.5 4.4 1 6.2L12 17.3 6.5 20.2l1-6.2L3 9.6l6.2-.9z",
    zap: "M13 2L4 14h7l-1 8 9-12h-7z",
    logout: "M15 4h4v16h-4 M10 8l-4 4 4 4 M6 12h11",
    home: "M3 11l9-8 9 8 M5 9v11h14V9",
    cloud: "M7 18a5 5 0 0 1-.5-10A6 6 0 0 1 18 9a4.5 4.5 0 0 1-.5 9z",
    alert: "M12 3l10 18H2z M12 10v5 M12 18h.01",
    check: "M5 12l5 5L19 7",
    reconnect: "M4 4v6h6 M20 20v-6h-6 M5.6 15A8 8 0 0 0 19 17 M18.4 9A8 8 0 0 0 5 7",
    more: "M5 12h.01 M12 12h.01 M19 12h.01",
    sidebar: "M3 4h18v16H3z M9 4v16",
    download: "M12 3v12 M7 10l5 5 5-5 M4 20h16",
  };
  export type IconName = keyof typeof P;
</script>

<script lang="ts">
  let { name, size = 16, class: cls = "" }: { name: string; size?: number; class?: string } = $props();
</script>

<svg
  class="icon {cls}"
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="1.8"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
>
  <path d={P[name] ?? ""} />
</svg>

<style>
  .icon {
    flex-shrink: 0;
    display: block;
  }
</style>
