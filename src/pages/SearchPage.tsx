import { useState } from "react";
import { api } from "../api";
import { fmtDate } from "../lib/format";
import { useAsync, useDebounced } from "../lib/hooks";
import { useNav } from "../lib/router";
import { ErrorBox, Loading } from "../components/ui";

const KINDS: { id: string; label: string }[] = [
  { id: "prompt", label: "My prompts" },
  { id: "response", label: "Replies" },
  { id: "tool_call", label: "Tool calls" },
  { id: "tool_result", label: "Tool output" },
];
const KIND_LABEL: Record<string, string> = { prompt: "Prompt", response: "Reply", tool_call: "Tool call", tool_result: "Tool output", command: "Command", system: "System" };

/** Render FTS snippet markers (\u0001 … \u0002) as <mark>. */
function Snippet({ text }: { text: string }) {
  const parts = text.split(/(\u0001[^\u0002]*\u0002)/g);
  return (
    <>
      {parts.map((p, i) => (p.startsWith("\u0001") ? <mark key={i}>{p.slice(1, -1)}</mark> : <span key={i}>{p}</span>))}
    </>
  );
}

export function SearchPage({ initial }: { initial?: string }) {
  const nav = useNav();
  const [q, setQ] = useState(initial ?? "");
  const [kinds, setKinds] = useState<string[]>(["prompt"]);
  const [projectId, setProjectId] = useState<number | null>(null);
  const query = useDebounced(q, 250);
  const projects = useAsync(() => api.listProjects(), []);
  const results = useAsync(
    () => (query.trim() ? api.search(query, kinds, projectId) : Promise.resolve([])),
    [query, kinds.join(","), projectId, nav.version],
  );
  const toggle = (k: string) => setKinds((ks) => (ks.includes(k) ? ks.filter((x) => x !== k) : [...ks, k]));

  return (
    <div className="page">
      <div className="page-header">
        <div className="titles">
          <h1>Search</h1>
          <div className="subtitle">Full-text search across every imported session.</div>
        </div>
      </div>
      <div className="row wrap" style={{ marginBottom: 14 }}>
        <input className="input" style={{ flex: "1 1 320px" }} autoFocus placeholder="Search… (words are matched as prefixes)" value={q} onChange={(e) => setQ(e.target.value)} />
        <select className="select" value={projectId ?? ""} onChange={(e) => setProjectId(e.target.value ? Number(e.target.value) : null)}>
          <option value="">All projects</option>
          {(projects.data ?? []).map((p) => (
            <option key={p.id} value={p.id}>
              {p.name}
            </option>
          ))}
        </select>
      </div>
      <div className="row wrap" style={{ marginBottom: 14 }}>
        {KINDS.map((k) => (
          <button key={k.id} className={`chip${kinds.includes(k.id) ? " on" : ""}`} onClick={() => toggle(k.id)}>
            {k.label}
          </button>
        ))}
        {kinds.length === 0 && <span className="muted small">(no filter: searching everything)</span>}
      </div>
      {results.error && <ErrorBox error={results.error} />}
      {!query.trim() ? (
        <div className="muted">Type to search.</div>
      ) : results.loading && !results.data ? (
        <Loading />
      ) : results.data && results.data.length === 0 ? (
        <div className="muted">No matches.</div>
      ) : (
        <div className="card" style={{ padding: 0 }}>
          {(results.data ?? []).map((h) => (
            <div key={h.messageId} className="hit-row" onClick={() => nav.go({ name: "session", id: h.sessionId, focus: h.messageId })}>
              <div className="row small muted">
                <span className="chip">{KIND_LABEL[h.kind] ?? h.kind}</span>
                <b style={{ color: "var(--text-2)" }}>{h.projectName}</b>
                {h.sessionTitle && <span className="ellipsis">· {h.sessionTitle}</span>}
                <span className="spacer" />
                <span className="nowrap">{fmtDate(h.timestamp)}</span>
              </div>
              <div className="snippet">
                <Snippet text={h.snippet} />
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
