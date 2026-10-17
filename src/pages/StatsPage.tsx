import { useMemo, useState } from "react";
import { api, type Tokens, type UsageReport } from "../api";
import { fmtCost, fmtNum, locationLabel, sourceLabel, totalTokens } from "../lib/format";
import { useAsync } from "../lib/hooks";
import { bucketLabel, bucketTitle, isoDay, PRESETS, presetBounds, toUsageRange, type RangePreset } from "../lib/range";
import { useNav } from "../lib/router";
import { ColumnChart, HBars, type ColumnDatum } from "../components/charts";
import { ErrorBox, Loading } from "../components/ui";

const TOKEN_KINDS: { key: keyof Tokens; label: string; hint: string }[] = [
  { key: "input", label: "Input", hint: "New (uncached) input tokens" },
  { key: "output", label: "Output", hint: "Tokens the model wrote" },
  { key: "cacheRead", label: "Cache read", hint: "Context re-read from the prompt cache (cheap, usually the largest number)" },
  { key: "cacheCreation", label: "Cache write", hint: "Context written to the prompt cache" },
];

type ProjectMetric = "total" | keyof Tokens;

export function RangePicker({
  preset,
  setPreset,
  custom,
  setCustom,
}: {
  preset: RangePreset;
  setPreset: (p: RangePreset) => void;
  custom: { from: string; to: string };
  setCustom: (c: { from: string; to: string }) => void;
}) {
  return (
    <div className="row wrap">
      <div className="seg">
        {PRESETS.map((p) => (
          <button key={p.id} className={preset === p.id ? "on" : ""} onClick={() => setPreset(p.id)}>
            {p.label}
          </button>
        ))}
      </div>
      {preset === "custom" && (
        <div className="row">
          <input className="input" type="date" value={custom.from} max={custom.to} onChange={(e) => setCustom({ ...custom, from: e.target.value })} />
          <span className="muted">to</span>
          <input className="input" type="date" value={custom.to} min={custom.from} onChange={(e) => setCustom({ ...custom, to: e.target.value })} />
        </div>
      )}
    </div>
  );
}

/** One small chart per token kind: their scales differ by orders of magnitude. */
export function TokenCharts({ report }: { report: UsageReport }) {
  return (
    <div className="multiples">
      {TOKEN_KINDS.map((k) => {
        const total = report.totals.tokens[k.key];
        const data: ColumnDatum[] = report.buckets.map((b) => ({
          key: b.key,
          label: bucketLabel(b.key),
          value: b.tokens[k.key],
          tooltip: (
            <>
              <b>{bucketTitle(b.key)}</b>
              <div>
                {k.label}: {b.tokens[k.key].toLocaleString()}
              </div>
            </>
          ),
        }));
        return (
          <div className="multiple" key={k.key} title={k.hint}>
            <div className="row">
              <span className="grow" style={{ fontWeight: 600 }}>
                {k.label}
              </span>
              <span className="muted small">{fmtNum(total)} total</span>
            </div>
            <ColumnChart data={data} height={130} ariaLabel={`${k.label} tokens over time`} />
          </div>
        );
      })}
    </div>
  );
}

export function StatsPage() {
  const nav = useNav();
  const [preset, setPreset] = useState<RangePreset>("30d");
  const today = isoDay(new Date());
  const [custom, setCustom] = useState({ from: isoDay(new Date(Date.now() - 6 * 86_400_000)), to: today });
  const range = useMemo(() => {
    const { start, end } = presetBounds(preset, custom);
    return toUsageRange(start, end);
  }, [preset, custom]);
  const { data: r, error, reload } = useAsync(() => api.usage(range), [range.from, range.to, range.bucket, nav.version]);
  const [metric, setMetric] = useState<"prompts" | "sessions">("prompts");
  const [projectMetric, setProjectMetric] = useState<ProjectMetric>("total");

  const activity = useMemo<ColumnDatum[]>(
    () =>
      (r?.buckets ?? []).map((b) => ({
        key: b.key,
        label: bucketLabel(b.key),
        value: b[metric],
        tooltip: (
          <>
            <b>{bucketTitle(b.key)}</b>
            <div>
              {b.sessions} sessions · {b.prompts} prompts
            </div>
            <div className="muted">
              {fmtNum(totalTokens(b.tokens))} tokens · {fmtCost(b.estCost)} est.
            </div>
          </>
        ),
      })),
    [r, metric],
  );

  const projectValue = (t: Tokens) => (projectMetric === "total" ? totalTokens(t) : t[projectMetric]);
  const projects = useMemo(
    () =>
      [...(r?.byProject ?? [])]
        .filter((p) => projectValue(p.tokens) > 0)
        .sort((a, b) => projectValue(b.tokens) - projectValue(a.tokens))
        .slice(0, 15),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [r, projectMetric],
  );

  return (
    <div className="page">
      <div className="page-header">
        <div className="titles">
          <h1>Usage</h1>
          <div className="subtitle">Costs are estimates at Anthropic API list prices; on a Pro/Max plan you are not billed per token.</div>
        </div>
      </div>
      <div style={{ marginBottom: 16 }}>
        <RangePicker preset={preset} setPreset={setPreset} custom={custom} setCustom={setCustom} />
      </div>
      {error && <ErrorBox error={error} onRetry={reload} />}
      {!r ? (
        <Loading />
      ) : (
        <>
          <div className="tiles">
            <div className="tile">
              <div className="label">Sessions</div>
              <div className="value">{fmtNum(r.totals.sessions)}</div>
              <div className="sub">{r.totals.activeProjects} {r.totals.activeProjects === 1 ? "project" : "projects"} active</div>
            </div>
            <div className="tile">
              <div className="label">Prompts</div>
              <div className="value">{fmtNum(r.totals.prompts)}</div>
            </div>
            <div className="tile">
              <div className="label">Tool calls</div>
              <div className="value">{fmtNum(r.totals.toolCalls)}</div>
              <div className="sub">
                {r.totals.toolErrors} failed{r.totals.toolCalls ? ` (${((r.totals.toolErrors / r.totals.toolCalls) * 100).toFixed(1)}%)` : ""}
              </div>
            </div>
            <div className="tile">
              <div className="label">Input / output tokens</div>
              <div className="value">
                {fmtNum(r.totals.tokens.input)} / {fmtNum(r.totals.tokens.output)}
              </div>
              <div className="sub">+ {fmtNum(r.totals.tokens.cacheRead + r.totals.tokens.cacheCreation)} cache</div>
            </div>
            <div className="tile">
              <div className="label">Est. cost</div>
              <div className="value">{fmtCost(r.totals.estCost)}</div>
            </div>
            <div className="tile">
              <div className="label">This app's analyses</div>
              <div className="value">{r.analyses.runs}</div>
              <div className="sub">
                {fmtNum(totalTokens(r.analyses.tokens))} tokens · {fmtCost(r.analyses.costUsd)}
              </div>
            </div>
          </div>

          <div className="card">
            <div className="row wrap" style={{ marginBottom: 10 }}>
              <h2 style={{ margin: 0 }} className="grow">
                {metric === "prompts" ? "Prompts" : "Sessions"} per {range.bucket}
              </h2>
              <div className="seg">
                <button className={metric === "prompts" ? "on" : ""} onClick={() => setMetric("prompts")}>
                  Prompts
                </button>
                <button className={metric === "sessions" ? "on" : ""} onClick={() => setMetric("sessions")}>
                  Sessions
                </button>
              </div>
            </div>
            <ColumnChart data={activity} ariaLabel={`${metric} per ${range.bucket}`} />
          </div>

          <div className="card">
            <h2>Tokens per {range.bucket}</h2>
            <TokenCharts report={r} />
          </div>

          <div className="card">
            <div className="row wrap" style={{ marginBottom: 10 }}>
              <h2 style={{ margin: 0 }} className="grow">
                Projects by token use
              </h2>
              <div className="seg">
                {(["total", "input", "output", "cacheRead", "cacheCreation"] as ProjectMetric[]).map((m) => (
                  <button key={m} className={projectMetric === m ? "on" : ""} onClick={() => setProjectMetric(m)}>
                    {m === "total" ? "Total" : TOKEN_KINDS.find((k) => k.key === m)!.label}
                  </button>
                ))}
              </div>
            </div>
            {projects.length === 0 ? (
              <div className="muted small">No token use in this range.</div>
            ) : (
              <HBars
                data={projects.map((p) => ({
                  key: p.projectId,
                  label: p.name,
                  value: projectValue(p.tokens),
                  display: fmtNum(projectValue(p.tokens)),
                  title: `${p.sessions} sessions · ${p.prompts} prompts · in ${fmtNum(p.tokens.input)} · out ${fmtNum(p.tokens.output)} · cache read ${fmtNum(
                    p.tokens.cacheRead,
                  )} · cache write ${fmtNum(p.tokens.cacheCreation)} · ${fmtCost(p.estCost)}`,
                  onClick: () => nav.go({ name: "project", id: p.projectId }),
                }))}
              />
            )}
          </div>

          <div className="row wrap" style={{ alignItems: "flex-start", gap: 12, marginTop: 12 }}>
            <div className="grow" style={{ minWidth: 380 }}>
              <table className="list">
                <thead>
                  <tr>
                    <th>Model</th>
                    <th className="num">Responses</th>
                    <th className="num">Input</th>
                    <th className="num">Output</th>
                    <th className="num">Cache</th>
                    <th className="num">Est. cost</th>
                  </tr>
                </thead>
                <tbody>
                  {r.byModel.map((m) => (
                    <tr key={m.model}>
                      <td>{m.model}</td>
                      <td className="num">{m.responses ? fmtNum(m.responses) : "—"}</td>
                      <td className="num">{fmtNum(m.tokens.input)}</td>
                      <td className="num">{fmtNum(m.tokens.output)}</td>
                      <td className="num">{fmtNum(m.tokens.cacheRead + m.tokens.cacheCreation)}</td>
                      <td className="num">{fmtCost(m.estCost)}</td>
                    </tr>
                  ))}
                  {r.byModel.length === 0 && (
                    <tr>
                      <td colSpan={6} className="muted">
                        No model use in this range.
                      </td>
                    </tr>
                  )}
                </tbody>
              </table>
            </div>
            <div className="grow" style={{ minWidth: 300 }}>
              <table className="list">
                <thead>
                  <tr>
                    <th>Source</th>
                    <th className="num">Sessions</th>
                    <th className="num">Prompts</th>
                  </tr>
                </thead>
                <tbody>
                  {r.bySource.map((x) => (
                    <tr key={x.source + x.location}>
                      <td>
                        {sourceLabel(x.source)} · {locationLabel(x.location)}
                      </td>
                      <td className="num">{x.sessions}</td>
                      <td className="num">{x.prompts}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </div>

          <div className="card">
            <h2>Tokens used by this app's analyses</h2>
            <p className="muted small" style={{ marginTop: 0 }}>
              What the Claude CLI used to analyse your prompts in this range. Separate from your own Claude Code / Codex usage above.
            </p>
            <div className="tiles" style={{ marginBottom: 0 }}>
              <div className="tile">
                <div className="label">Analyses</div>
                <div className="value">{r.analyses.runs}</div>
                <div className="sub">{r.analyses.failed} failed</div>
              </div>
              {TOKEN_KINDS.map((k) => (
                <div className="tile" key={k.key}>
                  <div className="label">{k.label}</div>
                  <div className="value">{fmtNum(r.analyses.tokens[k.key])}</div>
                </div>
              ))}
              <div className="tile">
                <div className="label">Cost (reported by CLI)</div>
                <div className="value">{fmtCost(r.analyses.costUsd)}</div>
              </div>
            </div>
          </div>
        </>
      )}
    </div>
  );
}
