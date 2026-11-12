// Пользовательские темы: несколько базовых цветов, остальное выводится из них.
// Стоковые темы (светлая/тёмная/системная) задаются в app.css и не трогаются.

export type CustomTheme = {
  id: string;
  name: string;
  /** Основа: от неё берутся цвета, которые не задаются вручную (терминал, тени) */
  base: "dark" | "light";
  bg: string;
  panel: string;
  text: string;
  accent: string;
  /** Градиент фона окна */
  gradient: boolean;
  g1: string;
  g2: string;
  angle: number;
};

/** Готовые темы. Их нельзя изменить, но можно скопировать в свою. */
export const PRESETS: CustomTheme[] = [
  {
    id: "coffee", name: "Кофе", base: "dark",
    bg: "#1c1511", panel: "#251c17", text: "#efe3d6", accent: "#d08c4c",
    gradient: true, g1: "#2a1b12", g2: "#140f0c", angle: 160,
  },
  {
    id: "midnight", name: "Полночь", base: "dark",
    bg: "#0f1424", panel: "#151b2e", text: "#e3e8f7", accent: "#818cf8",
    gradient: true, g1: "#111a3a", g2: "#1e1238", angle: 135,
  },
  {
    id: "aurora", name: "Аврора", base: "dark",
    bg: "#0c1a1a", panel: "#11211f", text: "#dff5ef", accent: "#2dd4bf",
    gradient: true, g1: "#0b2622", g2: "#1b1236", angle: 120,
  },
  {
    id: "sunset", name: "Закат", base: "dark",
    bg: "#1d1218", panel: "#26171e", text: "#f6e4e8", accent: "#fb7185",
    gradient: true, g1: "#33121f", g2: "#2b1a0d", angle: 145,
  },
  {
    id: "nord", name: "Север", base: "dark",
    bg: "#2e3440", panel: "#353c4a", text: "#eceff4", accent: "#88c0d0",
    gradient: false, g1: "#2e3440", g2: "#3b4252", angle: 180,
  },
  {
    id: "graphite", name: "Графит", base: "dark",
    bg: "#121212", panel: "#1a1a1a", text: "#e6e6e6", accent: "#f59e0b",
    gradient: false, g1: "#121212", g2: "#1f1f1f", angle: 180,
  },
  {
    id: "latte", name: "Латте", base: "light",
    bg: "#f6efe7", panel: "#fffaf4", text: "#3a2a1f", accent: "#a5622b",
    gradient: true, g1: "#f8eadb", g2: "#efe7f4", angle: 150,
  },
  {
    id: "mint", name: "Мята", base: "light",
    bg: "#eef7f3", panel: "#ffffff", text: "#13302a", accent: "#0f9f7a",
    gradient: true, g1: "#e3f6ee", g2: "#e9edfb", angle: 135,
  },
];

const mix = (a: string, b: string, pct: number) => `color-mix(in srgb, ${a} ${pct}%, ${b})`;
const alpha = (c: string, pct: number) => `color-mix(in srgb, ${c} ${pct}%, transparent)`;

/** Все CSS-переменные, которые задаёт пользовательская тема (чтобы потом их снять) */
export const THEME_VARS = [
  "--bg", "--bg-panel", "--bg-sunken", "--bg-hover", "--bg-active", "--border", "--border-strong",
  "--text", "--text-muted", "--text-faint", "--accent", "--accent-hover", "--accent-soft",
  "--term-bg", "--app-bg", "--bg-solid",
];

export function themeVars(t: CustomTheme): Record<string, string> {
  const dark = t.base === "dark";
  const shade = dark ? "#000" : "#fff";
  const sunken = mix(t.bg, dark ? "#000" : t.text, dark ? 78 : 95);
  const v: Record<string, string> = {
    "--bg": t.bg,
    "--bg-panel": t.panel,
    "--bg-solid": t.panel,
    "--bg-sunken": sunken,
    "--bg-hover": mix(t.panel, t.text, 91),
    "--bg-active": mix(t.panel, t.accent, 76),
    "--border": mix(t.panel, t.text, 87),
    "--border-strong": mix(t.panel, t.text, 78),
    "--text": t.text,
    "--text-muted": mix(t.text, t.panel, 66),
    "--text-faint": mix(t.text, t.panel, 46),
    "--accent": t.accent,
    "--accent-hover": mix(t.accent, dark ? "#fff" : "#000", 85),
    "--accent-soft": alpha(t.accent, dark ? 18 : 13),
    "--term-bg": mix(t.bg, shade, dark ? 80 : 100),
    "--app-bg": t.bg,
  };
  if (t.gradient) {
    // Панели полупрозрачные, чтобы градиент просвечивал
    v["--app-bg"] = `linear-gradient(${t.angle}deg, ${t.g1}, ${t.g2})`;
    v["--bg"] = alpha(t.bg, 30);
    v["--bg-panel"] = alpha(t.panel, 78);
    v["--bg-sunken"] = alpha(sunken, 70);
  }
  return v;
}

export function newThemeId(): string {
  return "t" + Date.now().toString(36);
}
