import definitions from "../../../crates/configuration/src/shortcuts.json";
import type { HotkeysSettings } from "./types";

export const SHORTCUTS = definitions;
export type ShortcutBindings = Record<string, string | null>;

const CODE_TOKENS: Record<string, string> = {
  ArrowUp: "UP", ArrowDown: "DOWN", ArrowLeft: "LEFT", ArrowRight: "RIGHT",
  Space: "SPACE", Tab: "TAB", Enter: "ENTER", NumpadEnter: "ENTER", Escape: "ESC",
  Comma: "COMMA", Period: "PERIOD", Slash: "SLASH", Semicolon: "SEMICOLON",
  Quote: "QUOTE", BracketLeft: "BRACKETLEFT", BracketRight: "BRACKETRIGHT",
  Backslash: "BACKSLASH", Backquote: "BACKQUOTE", Minus: "MINUS", Equal: "EQUAL",
  Backspace: "BACKSPACE", Delete: "DELETE", Insert: "INSERT", Home: "HOME", End: "END",
  PageUp: "PAGEUP", PageDown: "PAGEDOWN", Pause: "PAUSE", CapsLock: "CAPSLOCK",
  ScrollLock: "SCROLLLOCK", NumLock: "NUMLOCK", PrintScreen: "PRINTSCREEN",
  NumpadAdd: "NUMPADADD", NumpadSubtract: "NUMPADSUBTRACT", NumpadMultiply: "NUMPADMULTIPLY",
  NumpadDivide: "NUMPADDIVIDE", NumpadDecimal: "NUMPADDECIMAL",
};

type KeyEvent = Pick<KeyboardEvent, "code" | "key" | "ctrlKey" | "altKey" | "shiftKey" | "metaKey">;

export function eventChord(event: KeyEvent): string | null {
  let token = CODE_TOKENS[event.code];
  if (/^Key[A-Z]$/.test(event.code)) token = event.code.slice(3);
  else if (/^Digit[0-9]$/.test(event.code)) token = event.code.slice(5);
  else if (/^Numpad[0-9]$/.test(event.code)) token = event.code.toUpperCase();
  else if (/^F([1-9]|1[0-9]|2[0-4])$/.test(event.code)) token = event.code;
  if (!token) return null;
  return [event.ctrlKey && "CTRL", event.altKey && "ALT", event.shiftKey && "SHIFT",
    event.metaKey && "WIN", token].filter(Boolean).join("+");
}

export function resolveShortcuts(overrides?: ShortcutBindings, global?: HotkeysSettings): ShortcutBindings {
  const globalChords = [global?.saveReplay, global?.startCapture, global?.stopCapture].filter(Boolean).map((chord) => canonical(chord!));
  return Object.fromEntries(SHORTCUTS.map(({ id, default: fallback }) => {
    const inherited = fallback && globalChords.includes(canonical(fallback)) ? null : fallback;
    return [id, overrides?.[id] === undefined ? inherited : overrides[id]];
  }));
}

function canonical(chord: string): string {
  const tokens = chord.toUpperCase().split("+").map((token) => token.trim());
  const modifiers = ["CTRL", "ALT", "SHIFT", "WIN"];
  return [...modifiers.filter((token) => tokens.includes(token)),
    ...tokens.filter((token) => !modifiers.includes(token))].join("+");
}

export function shortcutAction(event: KeyEvent, bindings: ShortcutBindings): string | undefined {
  const chord = eventChord(event);
  return chord ? Object.keys(bindings).find((action) => bindings[action] && canonical(bindings[action]!) === chord) : undefined;
}

export function shortcutConflicts(hotkeys: HotkeysSettings): string[] {
  const entries: [string, string | null][] = [
    ["Save Replay", hotkeys.saveReplay], ["Start Capture", hotkeys.startCapture], ["Stop Capture", hotkeys.stopCapture],
  ];
  const bindings = resolveShortcuts(hotkeys.shortcuts, hotkeys);
  for (const definition of SHORTCUTS) entries.push([definition.label, bindings[definition.id]]);
  const seen = new Map<string, string>();
  const conflicts: string[] = [];
  for (const [label, chord] of entries) {
    if (!chord) continue;
    const key = canonical(chord);
    const previous = seen.get(key);
    if (previous) conflicts.push(`${label} and ${previous} both use ${chord}. Reassign or clear one.`);
    else seen.set(key, label);
  }
  return conflicts;
}

export function shortcutTitle(label: string, action: string, bindings: ShortcutBindings): string {
  return bindings[action] ? `${label} (${bindings[action]})` : label;
}
