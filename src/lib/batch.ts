// Runs several analyses one after another (so they don't hit rate limits all at once).
// Lives outside React so it keeps going while the user switches pages.

import { useEffect, useState } from "react";
import { api, onEvent, type AnalysisEvent, type AnalysisRequest } from "../api";
import { errorMessage } from "./format";

export type BatchState = { running: boolean; total: number; done: number; failed: number; lastError: string | null };

let state: BatchState = { running: false, total: 0, done: 0, failed: 0, lastError: null };
let cancelled = false;
const listeners = new Set<(s: BatchState) => void>();

function set(patch: Partial<BatchState>) {
  state = { ...state, ...patch };
  listeners.forEach((l) => l(state));
}

/** Resolves with the final status; the event is the fast path, polling is the safety net. */
function waitFor(id: number): Promise<string> {
  return new Promise((resolve) => {
    let finished = false;
    const finish = (status: string) => {
      if (finished) return;
      finished = true;
      off();
      clearInterval(timer);
      resolve(status);
    };
    const off = onEvent<AnalysisEvent>("analysis-updated", (e) => {
      if (e.id === id && e.status !== "running") finish(e.status);
    });
    const timer = setInterval(() => {
      api.getAnalysis(id).then((a) => a.status !== "running" && finish(a.status), () => {});
    }, 10_000);
  });
}

export async function runBatch(requests: AnalysisRequest[]): Promise<void> {
  if (state.running || requests.length === 0) return;
  cancelled = false;
  set({ running: true, total: requests.length, done: 0, failed: 0, lastError: null });
  for (const req of requests) {
    if (cancelled) break;
    try {
      const id = await api.startAnalysis(req);
      const status = await waitFor(id);
      set(status === "done" ? { done: state.done + 1 } : { done: state.done + 1, failed: state.failed + 1 });
    } catch (e) {
      set({ done: state.done + 1, failed: state.failed + 1, lastError: errorMessage(e) });
    }
  }
  set({ running: false });
}

/** Stops after the analysis that is currently running. */
export function cancelBatch() {
  cancelled = true;
}

export function useBatch(): BatchState {
  const [s, setS] = useState(state);
  useEffect(() => {
    listeners.add(setS);
    return () => {
      listeners.delete(setS);
    };
  }, []);
  return s;
}
