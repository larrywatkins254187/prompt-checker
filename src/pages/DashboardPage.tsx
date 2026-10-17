import { useEffect, useMemo, useState } from "react";
import { api, onEvent, type IngestStatus, type RetentionStatus, type SessionSummary } from "../api";
import { errorMessage, fmtCost, fmtDate, fmtNum, fmtRelative, firstLine, totalTokens } from "../lib/format";
import { useAsync } from "../lib/hooks";
import { bucketLabel, bucketTitle, isoDay, parseDay, presetBounds, toUsageRange } from "../lib/range";
import { useNav } from "../lib/router";
import { cancelBatch, runBatch, useBatch } from "../lib/batch";
import { useUpdateStatus } from "../lib/update";
import { AnalyzeDialog } from "../components/AnalyzeDialog";
import { AnalysisList, useAnalysisEvents } from "../components/AnalysisView";
import { ColumnChart, type ColumnDatum } from "../components/charts";
import { Dialog, ErrorBox, Icon, KindChip, Loading, ScoreBadge } from "../components/ui";
import { IngestIndicator } from "./ProjectsPage";

const TREND_DAYS = 30;

function StatusCard() {
  const nav = useNav();
  const update = useUpdateStatus();
  const batch = useBatch();
  const [cli, setCli] = useState<{ ok: boolean; text: string } | null>(null);
  const [retention, setRetention] = useState<RetentionStatus | null>(null);
  const [ingest, setIngest] = useState<IngestStatus | null>(null);

  useEffect(() => {
    api.claudeCliStatus().then(
      (t) => setCli({ ok: true, text: t }),
      (e) => setCli({ ok: false, text: errorMessage(e) }),
    );
    api.retentionStatus().then(setRetention, () => {});
    api.ingestStatus().then(setIngest);
    return onEvent<IngestStatus>("ingest-status", setIngest);
  }, []);

  const report = ingest?.lastReport;
  const restart = async () => {
    try {
      await api.installUpdate();
    } catch (e) {
      nav.toast(errorMessage(e));
    }
  };

  return (
    <div className="card">
      <h2>Status</h2>
      <div className="status-list">
        <div className="status-item">
          <span className={`dot ${cli == null ? "info" : cli.ok ? "good" : "bad"}`} />
          <span className="grow">
            Claude CLI: {cli == null ? "checking…" : cli.ok ? cli.text : <span className="error-text">{cli.text}</span>}
          </span>
          {cli && !cli.ok && (
            <button className="btn sm" onClick={() => nav.go({ name: "settings" })}>
              Fix
            </button>
          )}
        </div>
        <div className="status-item">
          <span className={`dot ${report?.errors.length ? "warn" : "good"}`} />
          <span className="grow">
            {ingest?.running
              ? "Importing sessions…"
              : report
                ? `Last import ${fmtRelative(report.finishedAt)}${report.errors.length ? ` · ${report.errors.length} files failed` : ""}`
                : "Import not run yet"}
          </span>
        </div>
        {retention && (
          <div className="status-item">
            <span className={`dot ${retention.atRisk ? "warn" : "good"}`} />
            <span className="grow">
              {retention.atRisk
                ? `Claude Code deletes sessions after ${retention.effectiveDays} days`
                : "Claude Code keeps your sessions"}
            </span>
            {retention.atRisk && (
              <button className="btn sm" onClick={() => api.fixRetention().then(setRetention, (e) => nav.toast(errorMessage(e)))}>
                Keep sessions
              </button>
            )}
          </div>
        )}
        {update && (
          <div className="status-item">
            <span className={`dot ${update.state === "ready" ? "info" : update.state === "error" ? "warn" : "good"}`} />
            <span className="grow">
              {update.state === "ready"
                ? `Version ${update.latestVersion} downloaded`
                : update.state === "downloading"
                  ? `Downloading version ${update.latestVersion}… ${update.progress != null ? Math.round(update.progress * 100) + "%" : ""}`
                  : update.state === "error"
                    ? `Update check failed: ${update.error}`
                    : `Prompt Checker ${update.currentVersion}${update.state === "up-to-date" ? " (latest)" : ""}`}
            </span>
            {update.state === "ready" && (
              <button className="btn primary sm" onClick={restart}>
                Restart to update
              </button>
            )}
          </div>
        )}
        {batch.running && (
          <div className="status-item">
            <span className="spinner" />
            <span className="grow">
              Analysing sessions: {batch.done} of {batch.total} done{batch.failed ? ` · ${batch.failed} failed` : ""}
            </span>
            <button className="btn sm" onClick={cancelBatch} title="Stop after the current analysis">
              Stop
            </button>
          </div>
        )}
      </div>
    </div>
  );
}

function BatchDialog({ sessions, onClose }: { sessions: SessionSummary[]; onClose: () => void }) {
  const nav = useNav();
  const { data: settings } = useAsync(() => api.getSettings(), []);
  const unanalysed = sessions.filter((s) => s.latestScore == null);
  const [onlyNew, setOnlyNew] = useState(true);
  const list = onlyNew ? unanalysed : sessions;
  const start = () => {
    if (!settings) return;
    runBatch(list.map((s) => ({ scope: "session", targetId: s.id, mode: settings.defaultMode, model: settings.defaultModel })));
    nav.toast(`Analysing ${list.length} session${list.length === 1 ? "" : "s"} in the background`);
    onClose();
  };
  return (
    <Dialog title="Analyse today's sessions" onClose={onClose}>
      {!settings ? (
        <Loading />
      ) : (
        <>
          <p style={{ marginTop: 0 }}>
            Runs one session analysis after another with <b>{settings.defaultModel}</b> in <b>{settings.defaultMode}</b> mode (change the defaults in
            Settings). It continues in the background while you use the app.
          </p>
          <label className="check">
            <input type="checkbox" checked={onlyNew} onChange={(e) => setOnlyNew(e.target.checked)} />
            Skip sessions that already have an analysis ({sessions.length - unanalysed.length})
          </label>
          <div className="muted small" style={{ marginTop: 8 }}>
            {list.length} session{list.length === 1 ? "" : "s"} will be analysed.
          </div>
          <div className="actions">
            <button className="btn" onClick={onClose}>
              Cancel
            </button>
            <button className="btn primary" disabled={list.length === 0} onClick={start}>
              <Icon name="play" size={14} /> Start
            </button>
          </div>
        </>
      )}
    </Dialog>
  );
}

export function DashboardPage() {
  const nav = useNav();
  const batch = useBatch();
  const today = useMemo(() => {
    const { start, end } = presetBounds("today");
    return { start, range: toUsageRange(start, end) };
    // Recompute when data changes so the page rolls over at midnight.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [nav.version]);
  const trendSince = useMemo(() => new Date(Date.now() - TREND_DAYS * 86_400_000).toISOString(), []);
  const usage = useAsync(() => api.usage(today.range), [today.range.from, nav.version]);
  const dash = useAsync(() => api.dashboard(trendSince), [trendSince, nav.version]);
  const projects = useAsync(() => api.listProjects(), [nav.version]);
  const todaySessions = useAsync(() => api.sessionsActiveSince(today.range.from), [today.range.from, nav.version]);
  const [analyze, setAnalyze] = useState<{ scope: "session" | "global"; targetId: number | null; title: string } | null>(null);
  const [batchOpen, setBatchOpen] = useState(false);
  const [draft, setDraft] = useState("");
  useAnalysisEvents(() => dash.reload(), []);

  const allTime = useMemo(() => {
    const p = projects.data ?? [];
    return {
      tokens: p.reduce((n, x) => n + totalTokens(x.tokens), 0),
      output: p.reduce((n, x) => n + x.tokens.output, 0),
      cost: p.reduce((n, x) => n + (x.estCost ?? 0), 0),
    };
  }, [projects.data]);

  const hourly = useMemo<ColumnDatum[]>(
    () =>
      (usage.data?.buckets ?? []).map((b) => ({
        key: b.key,
        label: bucketLabel(b.key),
        value: b.prompts,
        tooltip: (
          <>
            <b>{bucketTitle(b.key)}</b>
            <div>
              {b.prompts} prompts · {b.sessions} sessions
            </div>
            <div className="muted">{fmtNum(totalTokens(b.tokens))} tokens</div>
          </>
        ),
      })),
    [usage.data],
  );

  const trend = useMemo<ColumnDatum[]>(() => {
    const byDay = new Map<string, number[]>();
    for (const p of dash.data?.scoreHistory ?? []) {
      const day = isoDay(new Date(p.createdAt));
      byDay.set(day, [...(byDay.get(day) ?? []), p.score]);
    }
    const out: ColumnDatum[] = [];
    for (let i = TREND_DAYS - 1; i >= 0; i--) {
      const day = isoDay(new Date(Date.now() - i * 86_400_000));
      const scores = byDay.get(day) ?? [];
      const avg = scores.length ? scores.reduce((a, b) => a + b, 0) / scores.length : 0;
      out.push({
        key: day,
        label: parseDay(day).toLocaleDateString(undefined, { month: "short", day: "numeric" }),
        value: Math.round(avg * 10) / 10,
        tooltip: (
          <>
            <b>{bucketTitle(day)}</b>
            <div>{scores.length ? `Average score ${avg.toFixed(1)} (${scores.length} analyses)` : "No analyses"}</div>
          </>
        ),
      });
    }
    return out;
  }, [dash.data]);

  const t = usage.data?.totals;
  const c = dash.data?.counts;

  return (
    <div className="page">
      <div className="page-header">
        <div className="titles">
          <h1>Dashboard</h1>
          <div className="subtitle">{new Date().toLocaleDateString(undefined, { weekday: "long", year: "numeric", month: "long", day: "numeric" })}</div>
        </div>
        <IngestIndicator />
      </div>

      {usage.error && <ErrorBox error={usage.error} onRetry={usage.reload} />}
      <h2>Today</h2>
      <div className="tiles">
        <div className="tile">
          <div className="label">Sessions</div>
          <div className="value">{t ? t.sessions : "…"}</div>
          <div className="sub">{t ? `${t.activeProjects} ${t.activeProjects === 1 ? "project" : "projects"}` : ""}</div>
        </div>
        <div className="tile">
          <div className="label">Prompts</div>
          <div className="value">{t ? t.prompts : "…"}</div>
        </div>
        <div className="tile">
          <div className="label">Input / output tokens</div>
          <div className="value">{t ? `${fmtNum(t.tokens.input)} / ${fmtNum(t.tokens.output)}` : "…"}</div>
          <div className="sub">{t ? `+ ${fmtNum(t.tokens.cacheRead + t.tokens.cacheCreation)} cache` : ""}</div>
        </div>
        <div className="tile">
          <div className="label">Est. cost</div>
          <div className="value">{t ? fmtCost(t.estCost) : "…"}</div>
        </div>
        <div className="tile">
          <div className="label">Tool calls</div>
          <div className="value">{t ? t.toolCalls : "…"}</div>
          <div className="sub">{t ? `${t.toolErrors} failed` : ""}</div>
        </div>
        <div className="tile">
          <div className="label">Analyses today</div>
          <div className="value">{usage.data ? usage.data.analyses.runs : "…"}</div>
          <div className="sub">{usage.data ? `${fmtNum(totalTokens(usage.data.analyses.tokens))} tokens` : ""}</div>
        </div>
      </div>

      <h2>All time</h2>
      <div className="tiles">
        <div className="tile">
          <div className="label">Projects found</div>
          <div className="value">{c?.projects ?? "…"}</div>
        </div>
        <div className="tile">
          <div className="label">Sessions</div>
          <div className="value">{c ? fmtNum(c.sessions) : "…"}</div>
        </div>
        <div className="tile">
          <div className="label">Prompts</div>
          <div className="value">{c ? fmtNum(c.prompts) : "…"}</div>
        </div>
        <div className="tile">
          <div className="label">Tokens</div>
          <div className="value">{projects.data ? fmtNum(allTime.tokens) : "…"}</div>
          <div className="sub">{projects.data ? `${fmtNum(allTime.output)} output` : ""}</div>
        </div>
        <div className="tile">
          <div className="label">Est. cost</div>
          <div className="value">{projects.data ? fmtCost(allTime.cost) : "…"}</div>
        </div>
        <div className="tile">
          <div className="label">Analyses</div>
          <div className="value">{c?.analyses ?? "…"}</div>
          <div className="sub">{c?.runningAnalyses ? `${c.runningAnalyses} running` : ""}</div>
        </div>
      </div>

      <div className="dash-grid">
        <div className="col" style={{ gap: 12 }}>
          <StatusCard />
          <div className="card">
            <h2>Quick actions</h2>
            <div className="row wrap">
              <button className="btn" onClick={() => api.rescan(false).then((r) => nav.toast(`Imported ${r.filesImported} changed files`), (e) => nav.toast(errorMessage(e)))}>
                <Icon name="refresh" size={14} /> Rescan
              </button>
              <button className="btn" disabled={batch.running || !todaySessions.data?.length} onClick={() => setBatchOpen(true)}>
                <Icon name="sparkle" size={14} /> Analyse today's sessions ({todaySessions.data?.length ?? 0})
              </button>
              <button className="btn" onClick={() => setAnalyze({ scope: "global", targetId: null, title: "all projects" })}>
                <Icon name="sparkle" size={14} /> Overall analysis
              </button>
              <button className="btn" onClick={() => nav.go({ name: "stats" })}>
                <Icon name="chart" size={14} /> Usage
              </button>
            </div>
            <div className="col" style={{ marginTop: 14, gap: 6 }}>
              <span className="small muted">Check a prompt before sending it</span>
              <textarea className="textarea dir-auto" style={{ minHeight: 70 }} placeholder="Paste a prompt…" value={draft} onChange={(e) => setDraft(e.target.value)} />
              <div className="row">
                <span className="spacer" />
                <button
                  className="btn primary sm"
                  disabled={!draft.trim()}
                  onClick={() => {
                    try {
                      localStorage.setItem("rewriterDraft", draft);
                    } catch {
                      /* storage unavailable */
                    }
                    nav.go({ name: "rewriter" });
                  }}
                >
                  Open in rewriter
                </button>
              </div>
            </div>
          </div>
          <div className="card">
            <h2>Prompts today, by hour</h2>
            {usage.data ? <ColumnChart data={hourly} height={150} ariaLabel="Prompts per hour today" /> : <Loading />}
          </div>
        </div>

        <div className="col" style={{ gap: 12 }}>
          <div className="card">
            <div className="row" style={{ marginBottom: 6 }}>
              <h2 className="grow" style={{ margin: 0 }}>
                Recent sessions
              </h2>
              <a className="small" onClick={() => nav.go({ name: "projects" })}>
                All projects
              </a>
            </div>
            {!dash.data ? (
              <Loading />
            ) : dash.data.recentSessions.length === 0 ? (
              <div className="muted small">No sessions yet.</div>
            ) : (
              dash.data.recentSessions.map((s) => (
                <div className="mini-row" key={s.id}>
                  <ScoreBadge score={s.latestScore} />
                  <div className="grow" style={{ minWidth: 0, cursor: "pointer" }} onClick={() => nav.go({ name: "session", id: s.id })}>
                    <div className="ellipsis dir-auto" style={{ fontWeight: 550 }}>
                      {s.title || firstLine(s.firstPrompt, 90) || "(no prompts)"}
                    </div>
                    <div className="muted small">
                      {fmtRelative(s.endedAt)} · {s.promptCount} prompts · <KindChip source={s.source} location={s.location} />
                    </div>
                  </div>
                  <button className="btn sm" onClick={() => setAnalyze({ scope: "session", targetId: s.id, title: "this session" })}>
                    Analyse
                  </button>
                </div>
              ))
            )}
          </div>
          <div className="card">
            <h2>Score trend (last {TREND_DAYS} days)</h2>
            {dash.data ? (
              dash.data.scoreHistory.length ? (
                <ColumnChart data={trend} height={140} ariaLabel="Average analysis score per day" />
              ) : (
                <div className="muted small">Run some analyses to see how your prompting changes over time.</div>
              )
            ) : (
              <Loading />
            )}
          </div>
          <div className="card">
            <h2>Recent analyses</h2>
            {dash.data ? <AnalysisList items={dash.data.recentAnalyses} onOpen={(a) => nav.go({ name: "analysis", id: a.id })} /> : <Loading />}
            {dash.data && dash.data.recentAnalyses.length > 0 && (
              <div className="muted small" style={{ marginTop: 6 }}>
                Latest: {fmtDate(dash.data.recentAnalyses[0].createdAt)}
              </div>
            )}
          </div>
        </div>
      </div>

      {analyze && (
        <AnalyzeDialog
          scope={analyze.scope}
          targetId={analyze.targetId}
          title={analyze.title}
          onClose={() => setAnalyze(null)}
          onStarted={(id) => {
            setAnalyze(null);
            nav.go({ name: "analysis", id });
          }}
        />
      )}
      {batchOpen && todaySessions.data && <BatchDialog sessions={todaySessions.data} onClose={() => setBatchOpen(false)} />}
    </div>
  );
}
