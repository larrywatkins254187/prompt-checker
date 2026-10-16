import { useEffect, useState } from "react";
import { api, type AnalysisRow } from "../api";
import { errorMessage } from "../lib/format";
import { useAsync } from "../lib/hooks";
import { useNav } from "../lib/router";
import { ModelPicker } from "../components/AnalyzeDialog";
import { AnalysisBody, AnalysisList, useAnalysisEvents } from "../components/AnalysisView";
import { Icon } from "../components/ui";

const DRAFT_KEY = "rewriterDraft";

/** Get feedback on a prompt before sending it. */
export function RewriterPage() {
  const nav = useNav();
  const [draft, setDraft] = useState(() => localStorage.getItem(DRAFT_KEY) ?? "");
  const [context, setContext] = useState("");
  const [models, setModels] = useState<string[]>([]);
  const [model, setModel] = useState("");
  const [current, setCurrent] = useState<AnalysisRow | null>(null);
  const [error, setError] = useState<string | null>(null);
  const history = useAsync(() => api.listAnalyses("draft", null), [nav.version]);

  useEffect(() => {
    api.getSettings().then((s) => {
      setModels(s.models);
      setModel(s.defaultModel);
    });
  }, []);
  useEffect(() => {
    try {
      localStorage.setItem(DRAFT_KEY, draft);
    } catch {
      /* storage unavailable */
    }
  }, [draft]);
  useAnalysisEvents(
    (e) => {
      if (e.scope !== "draft") return;
      history.reload();
      if (current && e.id === current.id) api.getAnalysis(e.id).then(setCurrent);
    },
    [current?.id],
  );

  const run = async () => {
    setError(null);
    try {
      const id = await api.startAnalysis({ scope: "draft", targetId: null, mode: "full", model, draftText: draft, draftContext: context });
      setCurrent(await api.getAnalysis(id));
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  return (
    <div className="page">
      <div className="page-header">
        <div className="titles">
          <h1>Prompt rewriter</h1>
          <div className="subtitle">Paste a prompt before you send it to Claude Code or Codex, and get a score, the problems, and an improved version.</div>
        </div>
      </div>
      <div className="session-layout">
        <div className="col" style={{ gap: 12 }}>
          <div className="card col">
            <label className="field">
              Your prompt
              <textarea className="textarea dir-auto" style={{ minHeight: 180 }} value={draft} onChange={(e) => setDraft(e.target.value)} placeholder="e.g. add login to the app" />
            </label>
            <label className="field">
              Context (optional): what the project is, what you have tried, constraints
              <textarea className="textarea dir-auto" style={{ minHeight: 70 }} value={context} onChange={(e) => setContext(e.target.value)} />
            </label>
            <div className="row">
              <div style={{ width: 280 }}>
                <ModelPicker models={models} value={model} onChange={setModel} />
              </div>
              <span className="spacer" />
              <button className="btn primary" disabled={!draft.trim() || !model.trim() || current?.status === "running"} onClick={run}>
                <Icon name="sparkle" size={14} /> Review prompt
              </button>
            </div>
            {error && <div className="error-text small">{error}</div>}
          </div>
          {current && <AnalysisBody a={current} />}
        </div>
        <div className="side-panel">
          <div className="card">
            <h3>Previous drafts</h3>
            {history.data && (
              <AnalysisList
                items={history.data}
                onOpen={(a) => {
                  setCurrent(a);
                  if (a.inputText) setDraft(a.inputText);
                }}
              />
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
