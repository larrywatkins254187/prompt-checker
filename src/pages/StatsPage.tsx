import { useMemo, useState } from "react";
import { api } from "../api";
import { fmtCost, fmtNum, locationLabel, sourceLabel, totalTokens } from "../lib/format";
import { useAsync } from "../lib/hooks";
import { useNav } from "../lib/router";
import { ColumnChart, HBars } from "../components/charts";
import { ErrorBox, Loading } from "../components/ui";

function lastNDays(n: number): string[] {
  const out: string[] = [];
  const d = new Date();
  for (let i = n - 1; i >= 0; i--) {
    const x = new Date(d.getFullYear(), d.getMonth(), d.getDate() - i);
    out.push(`${x.getFullYear()}-${String(x.getMonth() + 1).padStart(2, "0")}-${String(x.getDate()).padStart(2, "0")}`);
  }
  return out;
}

export function StatsPage() {
  const nav = useNav();
  const [days, setDays] = useState(90);
  const { data: s, error, reload } = useAsync(() => api.stats(days), [days, nav.version]);
  const [metric, setMetric] = useState<"prompts" | "sessions">("prompts");

  const daily = useMemo(() => {
    if (!s) return [];
    const by = new Map(s.byDay.map((d) => [d.day, d]));
    return lastNDays(days).map((day) => {
      const d = by.get(day);
      const label = new Date(day + "T00:00:00").toLocaleDateString(undefined, { month: "short", day: "numeric" });
      return {
        key: day,
        label,
        value: d ? d[metric] : 0,
        tooltip: (
          <>
            <b>{label}</b>
            <div>
              {d?.sessions ?? 0} sessions · {d?.prompts ?? 0} prompts
            </div>
            <div className="muted">{fmtCost(d?.estCost ?? 0)} est.</div>
          </>
        ),
      };
    });
  }, [s, days, metric]);

  if (error) return <div className="page"><ErrorBox error={error} onRetry={reload} /></div>;
  if (!s) return <Loading />;

  return (
    <div className="page">
      <div className="page-header">
        <div className="titles">
          <h1>Usage</h1>
          <div className="subtitle">Costs are estimates at Anthropic API list prices; on a Pro/Max plan you are not billed per token.</div>
        </div>
      </div>
      <div className="tiles">
        <div className="tile">
          <div className="label">Projects</div>
          <div className="value">{s.projects}</div>
        </div>
        <div className="tile">
          <div className="label">Sessions</div>
          <div className="value">{fmtNum(s.sessions)}</div>
        </div>
        <div className="tile">
          <div className="label">Prompts</div>
          <div className="value">{fmtNum(s.prompts)}</div>
        </div>
        <div className="tile">
          <div className="label">Tool calls</div>
          <div className="value">{fmtNum(s.toolCalls)}</div>
          <div className="sub">{s.toolCalls ? ((s.toolErrors / s.toolCalls) * 100).toFixed(1) : 0}% failed</div>
        </div>
        <div className="tile">
          <div className="label">Tokens</div>
          <div className="value">{fmtNum(totalTokens(s.tokens))}</div>
          <div className="sub">{fmtNum(s.tokens.output)} output</div>
        </div>
        <div className="tile">
          <div className="label">Est. cost</div>
          <div className="value">{fmtCost(s.estCost)}</div>
        </div>
        <div className="tile">
          <div className="label">Analyses run</div>
          <div className="value">{s.analyses}</div>
        </div>
      </div>

      <div className="card">
        <div className="row wrap" style={{ marginBottom: 10 }}>
          <h2 style={{ margin: 0 }} className="grow">
            {metric === "prompts" ? "Prompts" : "Sessions"} per day
          </h2>
          <div className="seg">
            <button className={metric === "prompts" ? "on" : ""} onClick={() => setMetric("prompts")}>
              Prompts
            </button>
            <button className={metric === "sessions" ? "on" : ""} onClick={() => setMetric("sessions")}>
              Sessions
            </button>
          </div>
          <div className="seg">
            {[30, 90, 365].map((n) => (
              <button key={n} className={days === n ? "on" : ""} onClick={() => setDays(n)}>
                {n}d
              </button>
            ))}
          </div>
        </div>
        <ColumnChart data={daily} ariaLabel={`${metric} per day, last ${days} days`} />
      </div>

      <div className="card">
        <h2>Prompts by project</h2>
        <HBars
          data={s.byProject.slice(0, 20).map((p) => ({
            key: p.projectId,
            label: p.name,
            value: p.prompts,
            display: `${p.prompts}`,
            title: `${p.sessions} sessions · ${fmtNum(p.outputTokens)} output tokens · ${fmtCost(p.estCost)}`,
            onClick: () => nav.go({ name: "project", id: p.projectId }),
          }))}
        />
      </div>

      <div className="card">
        <h2>Estimated cost by project</h2>
        <HBars
          data={s.byProject
            .filter((p) => p.estCost > 0)
            .sort((a, b) => b.estCost - a.estCost)
            .slice(0, 20)
            .map((p) => ({ key: p.projectId, label: p.name, value: p.estCost, display: fmtCost(p.estCost), onClick: () => nav.go({ name: "project", id: p.projectId }) }))}
        />
      </div>

      <div className="row wrap" style={{ alignItems: "flex-start", gap: 12, marginTop: 12 }}>
        <div className="grow" style={{ minWidth: 340 }}>
          <table className="list">
            <thead>
              <tr>
                <th>Model</th>
                <th className="num">Responses</th>
                <th className="num">Output tokens</th>
                <th className="num">Est. cost</th>
              </tr>
            </thead>
            <tbody>
              {s.byModel.map((m) => (
                <tr key={m.model}>
                  <td>{m.model}</td>
                  <td className="num">{fmtNum(m.responses)}</td>
                  <td className="num">{fmtNum(m.outputTokens)}</td>
                  <td className="num">{fmtCost(m.estCost)}</td>
                </tr>
              ))}
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
              {s.bySource.map((x) => (
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
    </div>
  );
}
