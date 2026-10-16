import { useState } from "react";
import { api } from "../api";
import { useAsync } from "../lib/hooks";
import { useNav } from "../lib/router";
import { AnalyzeDialog } from "../components/AnalyzeDialog";
import { AnalysisBody, AnalysisList, useAnalysisEvents } from "../components/AnalysisView";
import { Icon, Loading } from "../components/ui";

/** Analyses across all projects: overall prompting habits. */
export function InsightsPage() {
  const nav = useNav();
  const list = useAsync(() => api.listAnalyses("global", null), [nav.version]);
  const [analyze, setAnalyze] = useState(false);
  useAnalysisEvents((e) => e.scope === "global" && list.reload(), []);
  const latest = list.data?.find((a) => a.status !== "error") ?? null;

  return (
    <div className="page">
      <div className="page-header">
        <div className="titles">
          <h1>Overall insights</h1>
          <div className="subtitle">
            Your prompting habits across every project. Large histories are reduced to fit: lower detail first, then only the most recent sessions.
          </div>
        </div>
        <button className="btn primary" onClick={() => setAnalyze(true)}>
          <Icon name="sparkle" size={14} /> Analyse all projects
        </button>
      </div>
      {!list.data ? (
        <Loading />
      ) : list.data.length === 0 ? (
        <div className="card empty">
          <h2>No overall analysis yet</h2>
          <p>Run one to get a report on your recurring prompting habits.</p>
        </div>
      ) : (
        <div className="session-layout">
          <div>{latest && <AnalysisBody a={latest} />}</div>
          <div className="side-panel">
            <div className="card">
              <h3>History</h3>
              <AnalysisList items={list.data} onOpen={(a) => nav.go({ name: "analysis", id: a.id })} />
            </div>
          </div>
        </div>
      )}
      {analyze && (
        <AnalyzeDialog
          scope="global"
          targetId={null}
          title="all projects"
          onClose={() => setAnalyze(false)}
          onStarted={() => {
            setAnalyze(false);
            list.reload();
          }}
        />
      )}
    </div>
  );
}
