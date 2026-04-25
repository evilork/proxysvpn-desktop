// src/components/ThemeToggle.tsx
import { useEffect, useState } from "react";
import { Sun, Moon } from "lucide-react";

type Theme = "light" | "dark";

const STORAGE_KEY = "proxysvpn_theme";

function getInitialTheme(): Theme {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (stored === "light" || stored === "dark") return stored;
  } catch {
    // localStorage unavailable, fall through
  }
  // Follow system preference
  if (
    typeof window !== "undefined" &&
    window.matchMedia &&
    window.matchMedia("(prefers-color-scheme: light)").matches
  ) {
    return "light";
  }
  return "dark";
}

function applyTheme(theme: Theme) {
  document.documentElement.setAttribute("data-theme", theme);
}

export default function ThemeToggle() {
  const [theme, setTheme] = useState<Theme>(getInitialTheme);

  useEffect(() => {
    applyTheme(theme);
    try {
      localStorage.setItem(STORAGE_KEY, theme);
    } catch {
      // ignore
    }
  }, [theme]);

  const toggle = () => setTheme((t) => (t === "dark" ? "light" : "dark"));

  return (
    <button
      type="button"
      onClick={toggle}
      title={theme === "dark" ? "Светлая тема" : "Тёмная тема"}
      aria-label="Переключить тему"
      className="theme-toggle nm-btn-sm"
    >
      {theme === "dark" ? (
        <Sun size={14} strokeWidth={2.2} />
      ) : (
        <Moon size={14} strokeWidth={2.2} />
      )}
    </button>
  );
}
