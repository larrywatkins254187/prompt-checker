import { createContext, useContext } from "react";

export type Route =
  | { name: "projects" }
  | { name: "project"; id: number; tab?: "sessions" | "analyses" }
  | { name: "session"; id: number; focus?: number }
  | { name: "analysis"; id: number }
  | { name: "search"; q?: string }
  | { name: "stats" }
  | { name: "insights" }
  | { name: "rewriter" }
  | { name: "settings" };

export type Nav = {
  route: Route;
  go: (r: Route) => void;
  back: () => void;
  canGoBack: boolean;
  /** Bumped whenever imported data or analyses change; pages re-fetch on it. */
  version: number;
  toast: (msg: string) => void;
};

export const NavContext = createContext<Nav | null>(null);

export function useNav(): Nav {
  const n = useContext(NavContext);
  if (!n) throw new Error("NavContext missing");
  return n;
}
