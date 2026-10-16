import { useState } from "react";
import { api } from "../api";
import { errorMessage, fmtCost, fmtDate, fmtNum, fmtRelative, firstLine, locationLabel, totalTokens } from "../lib/format";
import { useAsync } from "../lib/hooks";
import { useNav } from "../lib/router";
import { AnalyzeDialog } from "../components/AnalyzeDialog";
import { AnalysisList, useAnalysisEvents } from "../components/AnalysisView";
import { Dialog, ErrorBox, Icon, KindChip, Loading, ScoreBadge } from "../components/ui";

function MergeDialog({ projectId, onClose, onMerged }: { projectId: number; onClose: () => void; onMerged: (target: number) => void }) {
  const { data } = useAsync(() => api.listProjects(), []);
  const [target, setTarget] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  const others = (data ?? []).filter((p) => p.id !== projectId);
  const merge = async () => {
    if (target == null) return;
    try {
      await api.mergeProjects(projectId, target);
      onMerged(target);
    } catch (e) {
      setError(errorMessage(e));
    }
  };
  return (
    <Dialog title="Merge into another project" onClose={onClose}>
      <p className="muted small" style={{ marginTop: 0 }}>
        All sessions of this project move into the selected one, and future sessions from this folder go there too. Useful when the same
        project lives in several folders (e.g. Windows and WSL, or git worktrees).
      </p>
      <select className="select" style={{ width: "100%" }} value={target ?? ""} onChange={(e) => setTarget(e.target.value ? Number(e.target.value) : null)}>
        <option value="">Choose a project…</option>
        {others.map((p) => (
          <option key={p.id} value={p.id}>
            {p.name} — {p.path}
          </option>
        ))}
      </select>
      {error && <div className="error-text small">{error}</div>}
      <div className="actions">
        <button className="btn" onClick={onClose}>
          Cancel
        </button>
        <button className="btn primary" disabled={target == null} onClick={merge}>
          Merge
        </button>
      </div>
    </Dialog>
  );
}

export function ProjectPage({ id, tab: initialTab }: { id: number; tab?: "sessions" | "analyses" }) {
  const nav = useNav();
  const project = useAsync(() => api.getProject(id), [id, nav.version]);
  const sessions = useAsync(() => api.listSessions(id), [id, nav.version]);
  const analyses = useAsync(() => api.listAnalyses("project", id), [id, nav.version]);
  const [tab, setTab] = useState<"sessions" | "analyses">(initialTab ?? "sessions");
  const [analyze, setAnalyze] = useState(false);
  const [merge, setMerge] = useState(false);
  const [editing, setEditing] = useState<string | null>(null);

  useAnalysisEvents((e) => e.scope === "project" && e.targetId === id && analyses.reload(), [id]);

  const p = project.data;
  if (project.error) return <div className="page"><ErrorBox error={project.error} onRetry={project.reload} /></div>;
  if (!p) {
    return project.loading ? (
      <Loading />
    ) : (
      <div className="page">
        <div className="card empty">This project no longer exists (it may have been merged).</div>
      </div>
    );
  }

  const saveName = async () => {
    if (editing == null) return;
    try {
      await api.renameProject(id, editing);
      setEditing(null);
      project.reload();
    } catch (e) {
      nav.toast(errorMessage(e));
    }
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
          {editing != null ? (
            <div className="row">
              <input
                className="input"
                style={{ fontSize: 18, fontWeight: 600 }}
                value={editing}
                autoFocus
                onChange={(e) => setEditing(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") saveName();
                  if (e.key === "Escape") setEditing(null);
                }}
              />
              <button className="btn primary sm" onClick={saveName}>
                Save
              </button>
              <button className="btn ghost sm" onClick={() => setEditing(null)}>
                Cancel
              </button>
            </div>
          ) : (
            <div className="row">
              <h1>{p.name}</h1>
              <button className="btn ghost sm" title="Rename" onClick={() => setEditing(p.name)}>
                <Icon name="pen" size={14} />
              </button>
            </div>
          )}
          <div className="subtitle">
            {p.path} · {locationLabel(p.location)}
          </div>
        </div>
        <div className="row">
          <button className="btn" onClick={() => setMerge(true)}>
            <Icon name="merge" size={14} /> Merge…
          </button>
          <button className="btn primary" onClick={() => setAnalyze(true)}>
            <Icon name="sparkle" size={14} /> Analyse project
          </button>
        </div>
      </div>

      <div className="tiles">
        <div className="tile">
          <div className="label">Latest score</div>
          <div className="value">
            <ScoreBadge score={p.latestScore} />
          </div>
        </div>
        <div className="tile">
          <div className="label">Sessions</div>
          <div className="value">{p.sessionCount}</div>
        </div>
        <div className="tile">
          <div className="label">Prompts</div>
          <div className="value">{p.promptCount}</div>
        </div>
        <div className="tile">
          <div className="label">Tokens</div>
          <div className="value">{fmtNum(totalTokens(p.tokens))}</div>
          <div className="sub">{fmtNum(p.tokens.output)} output</div>
        </div>
        <div className="tile">
          <div className="label">Est. cost</div>
          <div className="value">{fmtCost(p.estCost)}</div>
          <div className="sub">at API list prices</div>
        </div>
        <div className="tile">
          <div className="label">Active</div>
          <div className="value" style={{ fontSize: 15 }}>
            {fmtDate(p.firstActivity, false)}
          </div>
          <div className="sub">to {fmtDate(p.lastActivity, false)}</div>
        </div>
      </div>

      <div className="tabs">
        <button className={tab === "sessions" ? "on" : ""} onClick={() => setTab("sessions")}>
          Sessions ({sessions.data?.length ?? "…"})
        </button>
        <button className={tab === "analyses" ? "on" : ""} onClick={() => setTab("analyses")}>
          Project analyses ({analyses.data?.length ?? "…"})
        </button>
      </div>

      {tab === "sessions" ? (
        sessions.error ? (
          <ErrorBox error={sessions.error} onRetry={sessions.reload} />
        ) : !sessions.data ? (
          <Loading />
        ) : (
          <table className="list">
            <thead>
              <tr>
                <th>Session</th>
                <th>Tool</th>
                <th>Started</th>
                <th className="num">Prompts</th>
                <th className="num">Tool calls</th>
                <th className="num">Errors</th>
                <th className="num">Est. cost</th>
                <th>Score</th>
              </tr>
            </thead>
            <tbody>
              {sessions.data.map((s) => (
                <tr key={s.id} className="clickable" onClick={() => nav.go({ name: "session", id: s.id })}>
                  <td className="title-cell">
                    <div className="t dir-auto">{s.title || firstLine(s.firstPrompt) || <span className="muted">(no prompts)</span>}</div>
                    <div className="s">
                      {s.gitBranch ? `⎇ ${s.gitBranch} · ` : ""}
                      {s.models ?? ""}
                    </div>
                  </td>
                  <td>
                    <KindChip source={s.source} location={s.location} />
                  </td>
                  <td className="nowrap" title={fmtDate(s.startedAt)}>
                    {fmtRelative(s.startedAt)}
                  </td>
                  <td className="num">{s.promptCount}</td>
                  <td className="num">{s.toolCallCount}</td>
                  <td className="num">{s.errorCount || ""}</td>
                  <td className="num">{fmtCost(s.estCost)}</td>
                  <td>
                    <ScoreBadge score={s.latestScore} />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )
      ) : (
        <div className="card">
          {analyses.data ? (
            <AnalysisList items={analyses.data} onOpen={(a) => nav.go({ name: "analysis", id: a.id })} />
          ) : (
            <Loading />
          )}
          <div style={{ marginTop: 12 }}>
            <button className="btn primary" onClick={() => setAnalyze(true)}>
              <Icon name="sparkle" size={14} /> New project analysis
            </button>
          </div>
        </div>
      )}

      {analyze && (
        <AnalyzeDialog
          scope="project"
          targetId={id}
          title={`project “${p.name}”`}
          onClose={() => setAnalyze(false)}
          onStarted={(aid) => {
            setAnalyze(false);
            nav.go({ name: "analysis", id: aid });
          }}
        />
      )}
      {merge && (
        <MergeDialog
          projectId={id}
          onClose={() => setMerge(false)}
          onMerged={(target) => {
            setMerge(false);
            nav.go({ name: "project", id: target });
          }}
        />
      )}
    </div>
  );
}
