import { Fragment, useEffect, useMemo, useState } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import { api, type MessageRow, type SessionDetail } from "../api";
import { errorMessage, fmtCost, fmtDate, fmtNum, fmtTime, firstLine, locationLabel, sourceLabel } from "../lib/format";
import { useAsync } from "../lib/hooks";
import { useNav } from "../lib/router";
import { AnalyzeDialog } from "../components/AnalyzeDialog";
import { AnalysisList, useAnalysisEvents } from "../components/AnalysisView";
import { ErrorBox, Icon, Loading, ScoreBadge } from "../components/ui";

type Block =
  | { type: "msg"; m: MessageRow; label?: string }
  | { type: "tools"; items: MessageRow[]; key: string };

function buildBlocks(messages: MessageRow[], showSidechain: boolean): Block[] {
  const blocks: Block[] = [];
  let n = 0;
  for (const m of messages) {
    if (m.isSidechain && !showSidechain) continue;
    if (m.kind === "tool_call" || m.kind === "tool_result") {
      const last = blocks[blocks.length - 1];
      if (last?.type === "tools") last.items.push(m);
      else blocks.push({ type: "tools", items: [m], key: `t${m.id}` });
      continue;
    }
    const label = m.kind === "prompt" && !m.isSidechain ? `P${++n}` : undefined;
    blocks.push({ type: "msg", m, label });
  }
  return blocks;
}

function ToolResultText({ m }: { m: MessageRow }) {
  const [full, setFull] = useState<string | null>(null);
  return (
    <>
      <pre>{full ?? m.text}</pre>
      {m.truncated && full == null && (
        <button className="btn sm" style={{ marginTop: 4 }} onClick={() => api.getMessage(m.id).then((r) => setFull(r.text))}>
          Show full output
        </button>
      )}
    </>
  );
}

function ToolGroup({ items, open }: { items: MessageRow[]; open: boolean }) {
  const calls = items.filter((i) => i.kind === "tool_call");
  const errors = items.filter((i) => i.kind === "tool_result" && i.isError).length;
  const names = Array.from(new Set(calls.map((c) => c.toolName ?? "tool")));
  return (
    <details className="tool-group" open={open || undefined}>
      <summary>
        <Icon name="down" size={12} />
        {calls.length} tool call{calls.length === 1 ? "" : "s"}
        <span className="ellipsis">· {names.slice(0, 6).join(", ")}</span>
        {errors > 0 && <span style={{ color: "var(--bad)" }}>· {errors} failed</span>}
      </summary>
      {items.map((m) => (
        <div key={m.id} className={`tool-item${m.isError ? " error" : ""}`}>
          {m.kind === "tool_call" ? (
            <>
              <span className="name">{m.toolName}</span> <span className="mono">{firstLine(m.text, 300)}</span>
            </>
          ) : (
            <>
              <span className="name">{m.isError ? "✗ error" : "result"}</span>
              {m.toolName ? <span className="muted"> · {m.toolName}</span> : null}
              <ToolResultText m={m} />
            </>
          )}
        </div>
      ))}
    </details>
  );
}

async function exportSession(d: SessionDetail, format: "md" | "json", toast: (m: string) => void) {
  const name = (d.projectName + "-" + (d.session.startedAt ?? "").slice(0, 10)).replace(/[^\w.-]+/g, "_");
  const path = await save({
    defaultPath: `${name}.${format}`,
    filters: [format === "md" ? { name: "Markdown", extensions: ["md"] } : { name: "JSON", extensions: ["json"] }],
  });
  if (!path) return;
  try {
    await api.exportSession(d.session.id, format, path);
    toast("Exported");
  } catch (e) {
    toast(errorMessage(e));
  }
}

export function SessionPage({ id, focus }: { id: number; focus?: number }) {
  const nav = useNav();
  const detail = useAsync(() => api.getSession(id), [id, nav.version]);
  const scores = useAsync(() => api.promptScores(id), [id, nav.version]);
  const sessionAnalyses = useAsync(() => api.listAnalyses("session", id), [id, nav.version]);
  const promptAnalyses = useAsync(() => api.sessionPromptAnalyses(id), [id, nav.version]);
  const [showSide, setShowSide] = useState(false);
  const [openTools, setOpenTools] = useState(false);
  const [analyze, setAnalyze] = useState<{ scope: "session" | "prompt"; targetId: number; title: string } | null>(null);

  useAnalysisEvents(() => {
    scores.reload();
    sessionAnalyses.reload();
    promptAnalyses.reload();
  }, [id]);

  const d = detail.data;
  const blocks = useMemo(() => (d ? buildBlocks(d.messages, showSide) : []), [d, showSide]);
  const scoreBy = useMemo(() => new Map((scores.data ?? []).map((s) => [s.messageId, s])), [scores.data]);

  useEffect(() => {
    if (!focus || !d) return;
    const el = document.getElementById(`m${focus}`);
    if (el) {
      el.scrollIntoView({ block: "center" });
      el.classList.add("flash");
      setTimeout(() => el.classList.remove("flash"), 1700);
    }
  }, [focus, d]);

  if (detail.error) return <div className="page"><ErrorBox error={detail.error} onRetry={detail.reload} /></div>;
  if (!d) return <Loading />;
  const s = d.session;
  const sideCount = d.messages.filter((m) => m.isSidechain).length;

  return (
    <div className="page">
      <button className="back" onClick={() => (nav.canGoBack ? nav.back() : nav.go({ name: "project", id: s.projectId }))}>
        <Icon name="back" size={14} /> {d.projectName}
      </button>
      <div className="page-header">
        <div className="titles">
          <h1 className="dir-auto">{s.title || firstLine(s.firstPrompt, 100) || "Session"}</h1>
          <div className="meta-line" style={{ marginTop: 6 }}>
            <span>
              {sourceLabel(s.source)} · {locationLabel(s.location)}
            </span>
            <span>{fmtDate(s.startedAt)}</span>
            {s.gitBranch && <span>⎇ {s.gitBranch}</span>}
            {s.models && <span>{s.models}</span>}
            <span>
              {s.promptCount} prompts · {s.toolCallCount} tool calls{s.errorCount ? ` · ${s.errorCount} failed` : ""}
            </span>
          </div>
        </div>
        <div className="row">
          <button className="btn primary" onClick={() => setAnalyze({ scope: "session", targetId: s.id, title: "this session" })}>
            <Icon name="sparkle" size={14} /> Analyse session
          </button>
        </div>
      </div>

      <div className="session-layout">
        <div>
          <div className="row wrap" style={{ marginBottom: 10 }}>
            <label className="check small">
              <input type="checkbox" checked={openTools} onChange={(e) => setOpenTools(e.target.checked)} /> Expand tool calls
            </label>
            {sideCount > 0 && (
              <label className="check small">
                <input type="checkbox" checked={showSide} onChange={(e) => setShowSide(e.target.checked)} /> Show subagent messages ({sideCount})
              </label>
            )}
          </div>
          <div className="transcript">
            {blocks.map((b) => {
              if (b.type === "tools") return <ToolGroup key={b.key + (openTools ? "o" : "c")} items={b.items} open={openTools} />;
              const m = b.m;
              if (m.kind === "command") return <div key={m.id} id={`m${m.id}`} className="command">❯ {firstLine(m.text.replace(/<[^>]+>/g, " "), 300)}</div>;
              if (m.kind === "system") {
                if (!m.text.startsWith("[Request interrupted")) return null;
                return (
                  <div key={m.id} className="system-note">
                    — interrupted by you —
                  </div>
                );
              }
              const isPrompt = m.kind === "prompt";
              const score = scoreBy.get(m.id);
              return (
                <Fragment key={m.id}>
                  <div id={`m${m.id}`} className={`msg ${m.kind}${m.isSidechain ? " sidechain" : ""}`}>
                    <div className="msg-head">
                      {b.label && <span className="ref">{b.label}</span>}
                      <span className="who">
                        {isPrompt ? (m.isSidechain ? "Agent → subagent" : m.toolName ? `You (answer to ${m.toolName})` : "You") : m.isSidechain ? "Subagent" : "Assistant"}
                      </span>
                      <span>{fmtTime(m.timestamp)}</span>
                      {!isPrompt && m.model && <span>· {m.model}</span>}
                      <span className="spacer" />
                      {isPrompt && !m.isSidechain && (
                        <>
                          {score && (
                            <a onClick={() => nav.go({ name: "analysis", id: score.analysisId })} title="Open the latest analysis of this prompt">
                              <ScoreBadge score={score.score} />
                            </a>
                          )}
                          <button className="btn sm" onClick={() => setAnalyze({ scope: "prompt", targetId: m.id, title: `prompt ${b.label}` })}>
                            <Icon name="sparkle" size={12} /> Analyse
                          </button>
                        </>
                      )}
                    </div>
                    <div className="msg-body">{m.text}</div>
                  </div>
                </Fragment>
              );
            })}
          </div>
        </div>

        <div className="side-panel">
          <div className="card">
            <h3>Session analyses</h3>
            {sessionAnalyses.data ? <AnalysisList items={sessionAnalyses.data} onOpen={(a) => nav.go({ name: "analysis", id: a.id })} /> : <Loading />}
          </div>
          <div className="card">
            <h3>Prompt analyses</h3>
            {promptAnalyses.data ? <AnalysisList items={promptAnalyses.data} onOpen={(a) => nav.go({ name: "analysis", id: a.id })} /> : <Loading />}
          </div>
          <div className="card small">
            <h3>Details</h3>
            <div className="col" style={{ gap: 4 }}>
              <div>
                <span className="muted">Tokens:</span> {fmtNum(s.tokens.input)} in · {fmtNum(s.tokens.output)} out · {fmtNum(s.tokens.cacheRead)} cache read ·{" "}
                {fmtNum(s.tokens.cacheCreation)} cache write
              </div>
              <div>
                <span className="muted">Est. cost:</span> {fmtCost(s.estCost)}
              </div>
              <div style={{ wordBreak: "break-all" }}>
                <span className="muted">Folder:</span> {s.cwd}
              </div>
              {s.cliVersion && (
                <div>
                  <span className="muted">CLI version:</span> {s.cliVersion}
                </div>
              )}
              <div style={{ wordBreak: "break-all" }}>
                <span className="muted">Log file:</span> {s.filePath}
              </div>
              <div style={{ wordBreak: "break-all" }}>
                <span className="muted">Session id:</span> {s.externalId}
              </div>
            </div>
            <div className="row" style={{ marginTop: 10 }}>
              <button className="btn sm" onClick={() => exportSession(d, "md", nav.toast)}>
                <Icon name="download" size={12} /> Markdown
              </button>
              <button className="btn sm" onClick={() => exportSession(d, "json", nav.toast)}>
                JSON
              </button>
            </div>
          </div>
        </div>
      </div>

      {analyze && (
        <AnalyzeDialog
          scope={analyze.scope}
          targetId={analyze.targetId}
          title={analyze.title}
          onClose={() => setAnalyze(null)}
          onStarted={(aid) => {
            setAnalyze(null);
            nav.go({ name: "analysis", id: aid });
          }}
        />
      )}
    </div>
  );
}
