import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { onEvent, type AnalysisEvent, type IngestStatus } from "./api";
import { NavContext, type Nav, type Route } from "./lib/router";
import { Icon } from "./components/ui";
import { AnalysisPage } from "./components/AnalysisView";
import { ProjectsPage } from "./pages/ProjectsPage";
import { ProjectPage } from "./pages/ProjectPage";
import { SessionPage } from "./pages/SessionPage";
import { SearchPage } from "./pages/SearchPage";
import { StatsPage } from "./pages/StatsPage";
import { InsightsPage } from "./pages/InsightsPage";
import { RewriterPage } from "./pages/RewriterPage";
import { SettingsPage } from "./pages/SettingsPage";

type Theme = "system" | "light" | "dark";

function readTheme(): Theme {
  try {
    const t = localStorage.getItem("theme");
    return t === "light" || t === "dark" ? t : "system";
  } catch {
    return "system";
  }
}

const NAV: { route: Route; label: string; icon: string; match: Route["name"][] }[] = [
  { route: { name: "projects" }, label: "Projects", icon: "folder", match: ["projects", "project", "session"] },
  { route: { name: "search" }, label: "Search", icon: "search", match: ["search"] },
  { route: { name: "insights" }, label: "Overall insights", icon: "sparkle", match: ["insights"] },
  { route: { name: "rewriter" }, label: "Prompt rewriter", icon: "pen", match: ["rewriter"] },
  { route: { name: "stats" }, label: "Usage", icon: "chart", match: ["stats"] },
  { route: { name: "settings" }, label: "Settings", icon: "gear", match: ["settings"] },
];

export default function App() {
  const [stack, setStack] = useState<Route[]>([{ name: "projects" }]);
  const [version, setVersion] = useState(0);
  const [toastMsg, setToastMsg] = useState<string | null>(null);
  const [theme, setTheme] = useState<Theme>(readTheme);
  const mainRef = useRef<HTMLDivElement>(null);
  const toastTimer = useRef<number | undefined>(undefined);

  useEffect(() => {
    if (theme === "system") document.documentElement.removeAttribute("data-theme");
    else document.documentElement.setAttribute("data-theme", theme);
    try {
      localStorage.setItem("theme", theme);
    } catch {
      /* storage unavailable */
    }
  }, [theme]);

  // Refresh pages when the importer finishes with new data or an analysis changes state.
  useEffect(() => {
    const offIngest = onEvent<IngestStatus>("ingest-status", (s) => {
      if (!s.running && s.lastReport && s.lastReport.filesImported > 0) setVersion((v) => v + 1);
    });
    const offAnalysis = onEvent<AnalysisEvent>("analysis-updated", () => setVersion((v) => v + 1));
    return () => {
      offIngest();
      offAnalysis();
    };
  }, []);

  const toast = useCallback((msg: string) => {
    setToastMsg(msg);
    window.clearTimeout(toastTimer.current);
    toastTimer.current = window.setTimeout(() => setToastMsg(null), 2600);
  }, []);

  const route = stack[stack.length - 1];
  const nav: Nav = useMemo(
    () => ({
      route,
      version,
      toast,
      canGoBack: stack.length > 1,
      go: (r) => {
        setStack((s) => {
          // Sidebar destinations start a fresh history.
          if (NAV.some((n) => n.route.name === r.name && Object.keys(r).length === 1)) return [r];
          return [...s, r].slice(-50);
        });
        mainRef.current?.scrollTo(0, 0);
      },
      back: () => setStack((s) => (s.length > 1 ? s.slice(0, -1) : s)),
    }),
    [route, version, toast, stack.length],
  );

  const page = (() => {
    switch (route.name) {
      case "projects":
        return <ProjectsPage />;
      case "project":
        return <ProjectPage key={route.id} id={route.id} tab={route.tab} />;
      case "session":
        return <SessionPage key={route.id} id={route.id} focus={route.focus} />;
      case "analysis":
        return <AnalysisPage key={route.id} id={route.id} />;
      case "search":
        return <SearchPage initial={route.q} />;
      case "stats":
        return <StatsPage />;
      case "insights":
        return <InsightsPage />;
      case "rewriter":
        return <RewriterPage />;
      case "settings":
        return <SettingsPage />;
    }
  })();

  const nextTheme: Record<Theme, Theme> = { system: "light", light: "dark", dark: "system" };

  return (
    <NavContext.Provider value={nav}>
      <div className="app">
        <aside className="sidebar">
          <div className="brand">
            <svg width="22" height="22" viewBox="0 0 24 24" aria-hidden>
              <rect x="1" y="1" width="22" height="22" rx="5" fill="var(--accent)" />
              <path d="M6 7.5h12a1.5 1.5 0 0 1 1.5 1.5v5a1.5 1.5 0 0 1-1.5 1.5h-6l-3.5 3v-3H6A1.5 1.5 0 0 1 4.5 14V9A1.5 1.5 0 0 1 6 7.5z" fill="#fff" />
              <path d="M8.5 11.6l2 2 4.5-4.2" stroke="var(--accent)" strokeWidth="1.8" fill="none" strokeLinecap="round" strokeLinejoin="round" />
            </svg>
            Prompt Checker
          </div>
          {NAV.map((n) => (
            <button key={n.label} className={`nav-item${n.match.includes(route.name) ? " active" : ""}`} onClick={() => nav.go(n.route)}>
              <Icon name={n.icon} /> {n.label}
            </button>
          ))}
          <div className="sidebar-footer">
            <button className="nav-item" onClick={() => setTheme(nextTheme[theme])} title="Switch theme">
              <Icon name={theme === "dark" ? "moon" : "sun"} /> Theme: {theme}
            </button>
          </div>
        </aside>
        <main className="main" ref={mainRef}>
          {page}
        </main>
      </div>
      {toastMsg && <div className="toast">{toastMsg}</div>}
    </NavContext.Provider>
  );
}
