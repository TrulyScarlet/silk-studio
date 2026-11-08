type IconName = "share" | "keyboard" | "settings" | "save" | "search" | "grid" | "list" | "game" | "video" | "audio" | "bell" | "palette" | "activity" | "trash" | "edit" | "shield" | "star" | "play" | "check" | "close" | "folder" | "warning" | "mute" | "expand" | "collapse" | "chevronLeft" | "chevronRight";

const paths: Record<IconName, string> = {
  share: "M12 15V3m-4 4 4-4 4 4M5 11H3v10h18V11h-2",
  chevronLeft: "m15 5-7 7 7 7",
  chevronRight: "m9 5 7 7-7 7",
  check: "m5 12 4 4L19 6",
  close: "m6 6 12 12M6 18 18 6",
  folder: "M3 5h7l2 3h9v12H3V5Z",
  warning: "m12 3 10 18H2L12 3Zm0 6v5m0 3v.01",
  mute: "M4 9h4l5-4v14l-5-4H4V9Zm13 1 4 4m0-4-4 4",
  expand: "M4 9V4h5m6 0h5v5M4 15v5h5m6 0h5v-5",
  collapse: "M9 4v5H4m11-5v5h5M4 15h5v5m6 0v-5h5",
  settings: "M9 3h6l1 3 3 1 2 5-2 5-3 1-1 3H9l-1-3-3-1-2-5 2-5 3-1 1-3Z M16 12a4 4 0 1 1-8 0 4 4 0 0 1 8 0",
  save: "M12 3v12m-5-5 5 5 5-5 M5 15v5h14v-5",
  search: "M16 10a6 6 0 1 1-12 0 6 6 0 0 1 12 0m-2 4 6 6",
  grid: "M4 4h6v6H4V4Zm10 0h6v6h-6V4ZM4 14h6v6H4v-6Zm10 0h6v6h-6v-6Z",
  list: "M9 6h11M9 12h11M9 18h11M4 6h1M4 12h1M4 18h1",
  game: "M7 7h10c2 0 3 2 4 9 0 3-2 4-4 1l-2-2H9l-2 2c-2 3-4 2-4-1 1-7 2-9 4-9ZM7 10v4m-2-2h4m7-1h.01m2 2h.01",
  video: "M3 5h18v14H3V5Zm0 4h18M7 5v4m5-4v4m5-4v4",
  audio: "M4 9h4l5-4v14l-5-4H4V9Zm12-1c3 2 3 6 0 8m3-11c5 4 5 10 0 14",
  bell: "M5 16h14l-2-3V8a5 5 0 0 0-10 0v5l-2 3Zm5 4h4",
  palette: "M21 12a9 9 0 1 0-9 9h2a2 2 0 0 0 0-4h-1a2 2 0 0 1 0-4h5c2 0 3 0 3-1ZM7 9h.01M10 6h.01M15 6h.01M18 9h.01",
  activity: "M3 12h4l3-8 4 16 3-8h4",
  trash: "M3 6h18M9 6V3h6v3M5 6l1 15h12l1-15M10 10v7m4-7v7",
  edit: "m4 16-1 5 5-1L20 8l-4-4L4 16Zm9-9 4 4",
  shield: "m12 3 8 3v6c0 5-8 9-8 9s-8-4-8-9V6l8-3Zm-4 9 3 3 5-6",
  keyboard: "M3 5h18v14H3z M6 9h1m3 0h1m3 0h1m3 0h1M6 12h1m3 0h1m3 0h1m3 0h1M7 16h10",
  star: "m12 3 3 6 6 1-4.5 4.5 1 6.5L12 18l-5.5 3 1-6.5L3 10l6-1 3-6Z",
  play: "m8 5 11 7-11 7V5Z",
};

export function Icon({ name }: { name: IconName }) {
  return <svg className="ui-icon" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><path d={paths[name]} /></svg>;
}
