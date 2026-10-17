import { useEffect, useState } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import { api, onEvent, type AnalysisEvent, type AnalysisRow, type Criterion, type RefEntry } from "../api";
import { errorMessage, fmtCost, fmtDate, fmtDuration, fmtNum, totalTokens } from "../lib/format";
import { useAsync } from "../lib/hooks";
import { useNav } from "../lib/router";
import { AnalyzeDialog } from "./AnalyzeDialog";
import { ScoreBar } from "./charts";
import { ErrorBox, Icon, Loading, ScoreBadge } from "./ui";

const MODE_LABEL: Record<string, string> = { prompts: "Prompts only", condensed: "Condensed", full: "Full" };
const SCOPE_LABEL: Record<string, string> = {
  prompt: "Prompt",
  session: "Session",
  project: "Project",
  global: "All projects",
  draft: "Draft",
};

export function useAnalysisEvents(handler: (e: AnalysisEvent) => void, deps: unknown[]) {
  // eslint-disable-next-line react-hooks/exhaustive-deps
  useEffect(() => onEvent<AnalysisEvent>("analysis-updated", handler), deps);
}

function RefLink({ r, refMap }: { r: string; refMap: RefEntry[] | null }) {
  const nav = useNav();
  const entry = refMap?.find((e) => e.ref === r);
  if (!entry?.messageId || !entry.sessionId) return <span className="ref">{r}</span>;
  return (
    <button className="ref" title="Open this prompt" onClick={() => nav.go({ name: "session", id: entry.sessionId!, focus: entry.messageId })}>
      {r}
    </button>
  );
}

async function copy(text: string, toast: (m: string) => void) {
  try {
    await navigator.clipboard.writeText(text);
    toast("Copied to clipboard");
  } catch {
    toast("Could not copy");
  }
}

export async function exportAnalysisFile(a: AnalysisRow, format: "md" | "json", toast: (m: string) => void) {
  const base = `analysis-${a.scope}-${a.id}`;
  const path = await save({
    defaultPath: `${base}.${format}`,
    filters: [format === "md" ? { name: "Markdown", extensions: ["md"] } : { name: "JSON", extensions: ["json"] }],
  });
  if (!path) return;
  try {
    await api.exportAnalysis(a.id, format, path);
    toast("Exported");
  } catch (e) {
    toast(errorMessage(e));
  }
}

let criteriaCache: Promise<Criterion[]> | null = null;

export function invalidateCriteriaCache() {
  criteriaCache = null;
}

/** Display names for rubric keys (criteria can be renamed or deleted later; fall back to the key). */
function useCriterionNames(): Map<string, string> {
  const [names, setNames] = useState<Map<string, string>>(new Map());
  useEffect(() => {
    criteriaCache ??= api.listCriteria();
    criteriaCache.then((list) => setNames(new Map(list.map((c) => [c.key, c.name]))), () => (criteriaCache = null));
  }, []);
  return names;
}

/** Running / failed / finished analysis, rendered in full. */
export function AnalysisBody({ a, compact, showUsage = true }: { a: AnalysisRow; compact?: boolean; showUsage?: boolean }) {
  const nav = useNav();
  const names = useCriterionNames();
  if (a.status === "running") {
    return (
      <div className="card">
        <div className="row">
          <span className="spinner" />
          <div className="grow">
            <b>Analysing with {a.model}…</b>
            <div className="muted small">This usually takes 20 seconds to a few minutes. You can keep using the app.</div>
          </div>
          <button className="btn" onClick={() => api.cancelAnalysis(a.id)}>
            Cancel
          </button>
        </div>
      </div>
    );
  }
  if (a.status === "error") {
    return (
      <div className="card">
        <ErrorBox error={a.error ?? "Unknown error"} />
        {a.rawOutput && (
          <details>
            <summary className="muted small">Raw CLI output</summary>
            <pre className="preview-text">{a.rawOutput}</pre>
          </details>
        )}
      </div>
    );
  }
  const r = a.result ?? {};
  return (
    <div className="col" style={{ gap: 12 }}>
      <div className="card">
        <div className="analysis-head">
          <ScoreBadge score={a.overallScore} big />
          <div className="grow" style={{ minWidth: 260 }}>
            <div className="dir-auto" style={{ fontSize: 15 }}>
              {r.summary || <span className="muted">No summary</span>}
            </div>
            {showUsage && a.tokens && (
              <div className="meta-line" style={{ marginTop: 8 }}>
                <span>{a.model}</span>
                <span>
                  {fmtNum(a.tokens.input)} in · {fmtNum(a.tokens.output)} out · {fmtNum(a.tokens.cacheRead + a.tokens.cacheCreation)} cache
                </span>
                {a.costUsd != null && <span>{fmtCost(a.costUsd)}</span>}
                {a.durationMs != null && <span>{fmtDuration(a.durationMs)}</span>}
              </div>
            )}
          </div>
        </div>
        {a.truncated && (
          <div className="banner warn" style={{ marginTop: 12, marginBottom: 0 }}>
            <Icon name="alert" /> The input was too large and was reduced before sending, so some sessions or details were not seen.
          </div>
        )}
      </div>

      {!!r.criteria?.length && (
        <div className="card">
          <h2>Scores by criterion</h2>
          <div className="criteria">
            {r.criteria.map((c, i) => (
              <div key={(c.key ?? "") + i}>
                <div className="crit-row">
                  <div className="ellipsis" style={{ fontWeight: 550 }} title={c.key}>
                    {names.get(c.key ?? "") ?? c.key}
                  </div>
                  <ScoreBar score={c.score} />
                  <div style={{ textAlign: "right", fontVariantNumeric: "tabular-nums" }}>{c.score ?? "—"}</div>
                  {!compact && c.comment && <div className="comment">{c.comment}</div>}
                </div>
              </div>
            ))}
          </div>
        </div>
      )}

      {!!r.mistakes?.length && (
        <div className="card">
          <h2>Mistakes ({r.mistakes.length})</h2>
          {r.mistakes.map((m, i) => {
            const sev = (m.severity ?? "low").toLowerCase();
            return (
              <div className={`mistake ${sev}`} key={i}>
                <div className="row wrap">
                  <span className={`sev ${sev}`}>{sev}</span>
                  <b className="grow">{m.title}</b>
                  {m.refs?.map((ref) => <RefLink key={ref} r={ref} refMap={a.refMap} />)}
                </div>
                {m.quote && <div className="quote">“{m.quote}”</div>}
                <dl>
                  {m.why && (
                    <>
                      <dt>Why</dt>
                      <dd>{m.why}</dd>
                    </>
                  )}
                  {m.impact && (
                    <>
                      <dt>Impact</dt>
                      <dd>{m.impact}</dd>
                    </>
                  )}
                  {m.fix && (
                    <>
                      <dt>Fix</dt>
                      <dd>{m.fix}</dd>
                    </>
                  )}
                </dl>
              </div>
            );
          })}
        </div>
      )}

      {!!r.rewrites?.length && (
        <div className="card">
          <h2>Suggested rewrites</h2>
          {r.rewrites.map((w, i) => (
            <div key={i}>
              <div className="row">
                {w.ref && w.ref !== "DRAFT" ? <RefLink r={w.ref} refMap={a.refMap} /> : null}
                <span className="muted small grow">{w.explanation}</span>
              </div>
              <div className="rewrite">
                <div className="pane">
                  <div className="label">Original</div>
                  {w.original || (a.scope === "draft" ? a.inputText : "")}
                </div>
                <div className="pane improved">
                  <div className="row" style={{ alignItems: "flex-start" }}>
                    <div className="label grow">Improved</div>
                    <button className="btn ghost sm" onClick={() => copy(w.improved ?? "", nav.toast)} title="Copy">
                      <Icon name="copy" size={14} />
                    </button>
                  </div>
                  {w.improved}
                </div>
              </div>
            </div>
          ))}
        </div>
      )}

      <div className="row wrap" style={{ alignItems: "stretch", gap: 12 }}>
        {[
          ["Strengths", r.strengths],
          ["Recurring patterns", r.patterns],
          ["Tips", r.tips],
        ]
          .filter(([, items]) => (items as string[] | undefined)?.length)
          .map(([title, items]) => (
            <div className="card grow" key={title as string} style={{ minWidth: 260, marginTop: 0 }}>
              <h2>{title as string}</h2>
              <ul className="bullets">
                {(items as string[]).map((s, i) => (
                  <li key={i} className="dir-auto">
                    {s}
                  </li>
                ))}
              </ul>
            </div>
          ))}
      </div>
    </div>
  );
}

function Compare({ a, b }: { a: AnalysisRow; b: AnalysisRow }) {
  const keys = Array.from(new Set([...(a.result?.criteria ?? []), ...(b.result?.criteria ?? [])].map((c) => c.key ?? "")));
  const score = (x: AnalysisRow, k: string) => x.result?.criteria?.find((c) => c.key === k)?.score;
  return (
    <div className="card">
      <h2>Comparison</h2>
      <table className="list">
        <thead>
          <tr>
            <th>Criterion</th>
            <th className="num">
              #{a.id} · {a.model}
            </th>
            <th className="num">
              #{b.id} · {b.model}
            </th>
          </tr>
        </thead>
        <tbody>
          <tr>
            <td>
              <b>Overall</b>
            </td>
            <td className="num">
              <ScoreBadge score={a.overallScore} />
            </td>
            <td className="num">
              <ScoreBadge score={b.overallScore} />
            </td>
          </tr>
          {keys.map((k) => (
            <tr key={k}>
              <td>{k}</td>
              <td className="num">{score(a, k) ?? "—"}</td>
              <td className="num">{score(b, k) ?? "—"}</td>
            </tr>
          ))}
        </tbody>
      </table>
      <div className="compare" style={{ marginTop: 12 }}>
        <div className="dir-auto small">{a.result?.summary}</div>
        <div className="dir-auto small">{b.result?.summary}</div>
      </div>
    </div>
  );
}

/** Full analysis page. */
export function AnalysisPage({ id }: { id: number }) {
  const nav = useNav();
  const { data: a, error, reload } = useAsync(() => api.getAnalysis(id), [id, nav.version]);
  const { data: siblings } = useAsync(
    () => (a && a.scope !== "draft" ? api.listAnalyses(a.scope, a.targetId) : Promise.resolve([])),
    [a?.id, a?.status],
  );
  const [compareId, setCompareId] = useState<number | null>(null);
  const [other, setOther] = useState<AnalysisRow | null>(null);
  const [rerun, setRerun] = useState(false);

  useAnalysisEvents((e) => e.id === id && reload(), [id]);
  useEffect(() => {
    if (compareId == null) setOther(null);
    else api.getAnalysis(compareId).then(setOther);
  }, [compareId]);

  if (error) return <div className="page"><ErrorBox error={error} onRetry={reload} /></div>;
  if (!a) return <Loading />;

  const targetLink = () => {
    if (a.targetId == null) return null;
    if (a.scope === "project") return () => nav.go({ name: "project", id: a.targetId! });
    if (a.scope === "session") return () => nav.go({ name: "session", id: a.targetId! });
    if (a.scope === "prompt")
      return () => api.messageSession(a.targetId!).then((sid) => nav.go({ name: "session", id: sid, focus: a.targetId! }));
    return null;
  };
  const open = targetLink();
  const others = (siblings ?? []).filter((s) => s.id !== a.id && s.status === "done");

  const remove = async () => {
    if (!confirm("Delete this analysis?")) return;
    await api.deleteAnalysis(a.id);
    nav.back();
  };

  return (
    <div className="page">
      {nav.canGoBack && (
        <button className="back" onClick={nav.back}>
          <Icon name="back" size={14} /> Back
        </button>
      )}
      <div className="page-header">
        <div className="titles">
          <h1 className="dir-auto">
            {SCOPE_LABEL[a.scope]} analysis{a.targetLabel ? ": " : ""}
            {open ? <a onClick={open}>{a.targetLabel}</a> : a.targetLabel}
          </h1>
          <div className="meta-line" style={{ marginTop: 6 }}>
            <span>Model: {a.model}</span>
            <span>Context: {MODE_LABEL[a.contextMode] ?? a.contextMode}</span>
            <span>{fmtDate(a.createdAt)}</span>
            {a.durationMs != null && <span>Took {fmtDuration(a.durationMs)}</span>}
            {a.costUsd != null && <span title="As reported by the Claude CLI (list price; subscriptions are not billed per token)">CLI cost {fmtCost(a.costUsd)}</span>}
            {a.projectPath && <span title="The model explored this folder (read-only)">Project: {a.projectPath}</span>}
          </div>
        </div>
        <div className="row wrap">
          {a.status === "done" && (
            <>
              <button className="btn" onClick={() => exportAnalysisFile(a, "md", nav.toast)}>
                <Icon name="download" size={14} /> Markdown
              </button>
              <button className="btn" onClick={() => exportAnalysisFile(a, "json", nav.toast)}>
                JSON
              </button>
            </>
          )}
          {a.scope !== "draft" && a.status !== "running" && (
            <button className="btn" onClick={() => setRerun(true)}>
              <Icon name="refresh" size={14} /> Re-run
            </button>
          )}
          {a.status !== "running" && (
            <button className="btn danger" onClick={remove}>
              <Icon name="trash" size={14} />
            </button>
          )}
        </div>
      </div>

      {a.tokens && (
        <div className="tiles" style={{ marginBottom: 12 }}>
          <div className="tile" title="New (uncached) input tokens">
            <div className="label">Input tokens</div>
            <div className="value">{fmtNum(a.tokens.input)}</div>
          </div>
          <div className="tile">
            <div className="label">Output tokens</div>
            <div className="value">{fmtNum(a.tokens.output)}</div>
          </div>
          <div className="tile" title="Input read from / written to the prompt cache">
            <div className="label">Cache read / write</div>
            <div className="value">
              {fmtNum(a.tokens.cacheRead)} / {fmtNum(a.tokens.cacheCreation)}
            </div>
          </div>
          <div className="tile">
            <div className="label">Total tokens</div>
            <div className="value">{fmtNum(totalTokens(a.tokens))}</div>
            <div className="sub">{a.costUsd != null ? `${fmtCost(a.costUsd)} at list price` : ""}</div>
          </div>
        </div>
      )}

      {a.status === "done" && others.length > 0 && (
        <div className="row" style={{ marginBottom: 12 }}>
          <span className="muted small">Compare with</span>
          <select className="select" value={compareId ?? ""} onChange={(e) => setCompareId(e.target.value ? Number(e.target.value) : null)}>
            <option value="">—</option>
            {others.map((o) => (
              <option key={o.id} value={o.id}>
                #{o.id} · {o.model} · {MODE_LABEL[o.contextMode]} · {fmtDate(o.createdAt)} · {o.overallScore?.toFixed(1) ?? "—"}
              </option>
            ))}
          </select>
        </div>
      )}
      {other && <div style={{ marginBottom: 12 }}><Compare a={a} b={other} /></div>}

      <AnalysisBody a={a} showUsage={false} />

      {rerun && (
        <AnalyzeDialog
          scope={a.scope}
          targetId={a.targetId}
          title={a.targetLabel ?? SCOPE_LABEL[a.scope]}
          onClose={() => setRerun(false)}
          onStarted={(nid) => {
            setRerun(false);
            nav.go({ name: "analysis", id: nid });
          }}
        />
      )}
    </div>
  );
}

/** Compact list of analyses for a target, used in side panels and tabs. */
export function AnalysisList({ items, onOpen }: { items: AnalysisRow[]; onOpen: (a: AnalysisRow) => void }) {
  if (!items.length) return <div className="muted small">No analyses yet.</div>;
  return (
    <div className="analysis-list">
      {items.map((a) => (
        <div className="item" key={a.id} onClick={() => onOpen(a)}>
          {a.status === "running" ? <span className="spinner" /> : a.status === "error" ? <span className="score bad">err</span> : <ScoreBadge score={a.overallScore} />}
          <div className="grow" style={{ minWidth: 0 }}>
            <div className="ellipsis small" style={{ fontWeight: 550 }}>
              {a.scope === "prompt" || a.scope === "draft" ? a.targetLabel ?? a.inputText?.slice(0, 80) : a.model}
            </div>
            <div className="muted small">
              {a.scope === "prompt" || a.scope === "draft" ? `${a.model} · ` : ""}
              {MODE_LABEL[a.contextMode]} · {fmtDate(a.createdAt)}
              {a.tokens ? ` · ${fmtNum(totalTokens(a.tokens))} tok` : ""}
            </div>
          </div>
        </div>
      ))}
    </div>
  );
}
