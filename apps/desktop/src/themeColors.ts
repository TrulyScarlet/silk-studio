import type { CSSProperties } from "react";
import type { AppTheme, ThemeColors } from "./types";

export const COLOR_FIELDS = [
  ["main", "Main color"], ["background", "Background color"], ["text", "Text color"],
  ["subtext", "Subtext color"], ["capture", "Capture color"], ["replayBuffer", "Replay buffer color"],
] as const;

export function defaultColors(theme: AppTheme): Required<ThemeColors> {
  const [main, background, signal] = {
    studio: ["#efefed", "#080808", "#efefed"], classic: ["#e4d29b", "#10110e", "#dfd497"],
    ember: ["#eab17c", "#140f0c", "#e9b079"], vamp: ["#eaa5b0", "#100d11", "#ed939e"],
  }[theme];
  return {main, background, text: "#f2f2f0", subtext: "#ababaa", capture: theme === "studio" ? main : "#75d5b2", replayBuffer: signal};
}

export function colorVariables(colors: ThemeColors = {}): CSSProperties {
  const vars: Record<string, string> = {};
  const valid = (value: string | null | undefined): value is string => !!value && /^#[0-9a-f]{6}$/i.test(value);
  if (valid(colors.main)) {
    vars["--accent"] = colors.main;
    const rgb = [1, 3, 5].map((index) => parseInt(colors.main!.slice(index, index + 2), 16) / 255)
      .map((value) => value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4);
    vars["--on-accent"] = rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722 > 0.179 ? "#000000" : "#ffffff";
    vars["--accent-hover"] = `color-mix(in srgb, ${colors.main}, white 15%)`;
    vars["--accent-soft"] = `color-mix(in srgb, ${colors.main} 15%, var(--bg))`;
    vars["--accent-subtle"] = `color-mix(in srgb, ${colors.main} 8%, var(--bg))`;
    vars["--accent-line"] = `color-mix(in srgb, ${colors.main} 25%, var(--bg))`;
  }
  if (valid(colors.background)) {
    vars["--bg"] = colors.background;
    vars["--bg-deep"] = `color-mix(in srgb, ${colors.background}, black 35%)`;
    for (const [token, amount] of [["surface", 5], ["surface-raised", 10], ["surface-input", 3], ["surface-hover", 15]] as const) {
      vars[`--${token}`] = `color-mix(in srgb, ${colors.background}, white ${amount}%)`;
    }
  }
  if (valid(colors.text)) vars["--text"] = colors.text;
  if (valid(colors.subtext)) for (const token of ["text-soft", "text-muted", "text-dim"]) vars[`--${token}`] = colors.subtext;
  if (valid(colors.capture)) vars["--capture-color"] = colors.capture;
  if (valid(colors.replayBuffer)) vars["--signal"] = colors.replayBuffer;
  return vars as CSSProperties;
}
