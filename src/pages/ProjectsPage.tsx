import { useEffect, useMemo, useState } from "react";
import { api, onEvent, type IngestStatus, type RetentionStatus } from "../api";
import { errorMessage, fmtCost, fmtNum, fmtRelative, locationLabel, sourceLabel, totalTokens } from "../lib/format";
import { useAsync } from "../lib/hooks";
import { useNav } from "../lib/router";
import { ErrorBox, Icon, Loading, ScoreBadge } from "../components/ui";

export function RetentionBanner() {
  const [status, setStatus] = useState<RetentionStatus | null>(null);
  const [dismissed, setDismissed] = useState(() => localStorage.getItem("retentionDismissed") === "1");
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    api.retentionStatus().then(setStatus, () => {});
  }, []);
  if (!status?.atRisk || dismissed) return null;
  const fix = async () => {
    try {
      setStatus(await api.fixRetention());
    } catch (e) {
      setError(errorMessage(e));
    }
  };
  return (
    <div className="banner warn">
      <Icon name="alert" />
      <div className="grow">
        <b>Claude Code deletes sessions older than {status.effectiveDays} days.</b>{" "}
        <span className="small">
          This app keeps everything it has already imported, but sessions it has not seen yet will be lost. Setting{" "}
          <code>cleanupPeriodDays</code> in <code>{status.settingsPath}</code> keeps them (a backup of the file is made).
        </span>
        {error && <div className="error-text small">{error}</div>}
      </div>
      <button className="btn primary sm" onClick={fix}>
        Keep sessions
      </button>
      <button
        className="btn ghost sm"
        onClick={() => {
          localStorage.setItem("retentionDismissed", "1");
          setDismissed(true);
        }}
      >
        Dismiss
      </button>
    </div>
  );
}

export function IngestIndicator() {
  const nav = useNav();
  const [status, setStatus] = useState<IngestStatus | null>(null);
  useEffect(() => {
    api.ingestStatus().then(setStatus);
    return onEvent<IngestStatus>("ingest-status", setStatus);
  }, []);
  const report = status?.lastReport;
  const rescan = async () => {
    try {
      const r = await api.rescan(false);
      nav.toast(`Scanned ${r.filesScanned} files, imported ${r.filesImported}`);
    } catch (e) {
      nav.toast(errorMessage(e));
    }
  };
  return (
    <div className="row small muted">
      {status?.running ? (
        <>
          <span className="spinner" /> Importing…
        </>
      ) : report ? (
        <span title={[...report.errors, ...report.sourcesSkipped].join("\n") || undefined}>
          Last import {fmtRelative(report.finishedAt)}
          {report.errors.length > 0 && <span style={{ color: "var(--bad)" }}> · {report.errors.length} errors</span>}
        </span>
      ) : null}
      <button className="btn sm" onClick={rescan} disabled={status?.running}>
        <Icon name="refresh" size={14} /> Rescan
      </button>
    </div>
  );
}

export function ProjectsPage() {
  const nav = useNav();
  const { data, error, loading, reload } = useAsync(() => api.listProjects(), [nav.version]);
  const [filter, setFilter] = useState("");
  const rows = useMemo(() => {
    const f = filter.trim().toLowerCase();
    return (data ?? []).filter((p) => !f || p.name.toLowerCase().includes(f) || (p.path ?? "").toLowerCase().includes(f));
  }, [data, filter]);

  return (
    <div className="page">
      <div className="page-header">
        <div className="titles">
          <h1>Projects</h1>
          <div className="subtitle">Every folder you have used Claude Code or Codex CLI in.</div>
        </div>
        <IngestIndicator />
      </div>
      <RetentionBanner />
      {error && <ErrorBox error={error} onRetry={reload} />}
      {loading && !data ? (
        <Loading />
      ) : data && data.length === 0 ? (
        <div className="card empty">
          <h2>No sessions imported yet</h2>
          <p>
            The app looks in <code>%USERPROFILE%\.claude\projects</code>, <code>%USERPROFILE%\.codex\sessions</code> and the same
            folders inside your WSL distributions.
          </p>
          <div className="row" style={{ justifyContent: "center" }}>
            <button className="btn primary" onClick={() => api.rescan(false).then(reload)}>
              Scan now
            </button>
            <button className="btn" onClick={() => nav.go({ name: "settings" })}>
              Check sources
            </button>
          </div>
        </div>
      ) : (
        <>
          <div className="row" style={{ marginBottom: 12 }}>
            <input className="input" style={{ width: 320 }} placeholder="Filter projects…" value={filter} onChange={(e) => setFilter(e.target.value)} />
            <span className="muted small">
              {rows.length} project{rows.length === 1 ? "" : "s"}
            </span>
          </div>
          <table className="list">
            <thead>
              <tr>
                <th>Project</th>
                <th>Sources</th>
                <th className="num">Sessions</th>
                <th className="num">Prompts</th>
                <th className="num" title="Input + output + cache tokens">Tokens</th>
                <th className="num" title="Estimated at API list prices">Est. cost</th>
                <th>Last activity</th>
                <th>Score</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((p) => (
                <tr key={p.id} className="clickable" onClick={() => nav.go({ name: "project", id: p.id })}>
                  <td className="title-cell">
                    <div className="t">{p.name}</div>
                    <div className="s" title={p.path ?? ""}>
                      {p.path}
                    </div>
                  </td>
                  <td>
                    <div className="row wrap" style={{ gap: 4 }}>
                      {p.sources.map((s) => (
                        <span className="chip" key={s}>
                          {sourceLabel(s)}
                        </span>
                      ))}
                      {p.location !== "windows" && <span className="chip">{locationLabel(p.location)}</span>}
                    </div>
                  </td>
                  <td className="num">{p.sessionCount}</td>
                  <td className="num">{p.promptCount}</td>
                  <td className="num">{fmtNum(totalTokens(p.tokens))}</td>
                  <td className="num">{fmtCost(p.estCost)}</td>
                  <td className="nowrap">{fmtRelative(p.lastActivity)}</td>
                  <td>
                    <ScoreBadge score={p.latestScore} />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </>
      )}
    </div>
  );
}
