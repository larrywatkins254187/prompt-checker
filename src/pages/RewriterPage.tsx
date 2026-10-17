import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { api, type AnalysisRow } from "../api";
import { errorMessage } from "../lib/format";
import { useAsync } from "../lib/hooks";
import { useNav } from "../lib/router";
import { ModelPicker } from "../components/AnalyzeDialog";
import { AnalysisBody, AnalysisList, useAnalysisEvents } from "../components/AnalysisView";
import { Icon } from "../components/ui";

const DRAFT_KEY = "rewriterDraft";
const PROJECT_KEY = "rewriterProject";

/** Project the draft is reviewed against: an imported project, a browsed folder, or none. */
type ProjectChoice = { kind: "none" } | { kind: "project"; id: number } | { kind: "folder"; path: string };

function loadChoice(): ProjectChoice {
  try {
    const v = JSON.parse(localStorage.getItem(PROJECT_KEY) ?? "null");
    if (v?.kind === "project" && typeof v.id === "number") return v;
    if (v?.kind === "folder" && typeof v.path === "string") return v;
  } catch {
    /* ignore */
  }
  return { kind: "none" };
}

/** Get feedback on a prompt before sending it. */
export function RewriterPage() {
  const nav = useNav();
  const [draft, setDraft] = useState(() => {
    try {
      return localStorage.getItem(DRAFT_KEY) ?? "";
    } catch {
      return "";
    }
  });
  const [context, setContext] = useState("");
  const [choice, setChoice] = useState<ProjectChoice>(loadChoice);
  const [models, setModels] = useState<string[]>([]);
  const [model, setModel] = useState("");
  const [current, setCurrent] = useState<AnalysisRow | null>(null);
  const [error, setError] = useState<string | null>(null);
  const history = useAsync(() => api.listAnalyses("draft", null), [nav.version]);
  const projects = useAsync(() => api.listProjects(), []);

  useEffect(() => {
    api.getSettings().then((s) => {
      setModels(s.models);
      setModel(s.defaultModel);
    });
  }, []);
  useEffect(() => {
    try {
      localStorage.setItem(DRAFT_KEY, draft);
      localStorage.setItem(PROJECT_KEY, JSON.stringify(choice));
    } catch {
      /* storage unavailable */
    }
  }, [draft, choice]);
  useAnalysisEvents(
    (e) => {
      if (e.scope !== "draft") return;
      history.reload();
      if (current && e.id === current.id) api.getAnalysis(e.id).then(setCurrent);
    },
    [current?.id],
  );

  const browse = async () => {
    const dir = await open({ directory: true, title: "Select the project folder" });
    if (dir && !Array.isArray(dir)) setChoice({ kind: "folder", path: dir });
  };

  const selectedProject = choice.kind === "project" ? projects.data?.find((p) => p.id === choice.id) : undefined;
  const folderLabel = choice.kind === "folder" ? choice.path : selectedProject?.path ?? null;

  const run = async () => {
    setError(null);
    try {
      const id = await api.startAnalysis({
        scope: "draft",
        targetId: null,
        mode: "full",
        model,
        draftText: draft,
        draftContext: context,
        projectId: choice.kind === "project" ? choice.id : null,
        projectDir: choice.kind === "folder" ? choice.path : null,
      });
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
          <div className="subtitle">
            Paste a prompt before you send it to Claude Code or Codex, and get a score, the problems, and an improved version.
          </div>
        </div>
      </div>
      <div className="session-layout">
        <div className="col" style={{ gap: 12 }}>
          <div className="card col">
            <label className="field">
              Your prompt
              <textarea
                className="textarea dir-auto"
                style={{ minHeight: 180 }}
                value={draft}
                onChange={(e) => setDraft(e.target.value)}
                placeholder="e.g. add login to the app"
              />
            </label>

            <div className="field">
              <span>Project (optional): the model reads the code to make the rewrite concrete</span>
              <div className="row wrap">
                <select
                  className="select grow"
                  style={{ minWidth: 240 }}
                  value={choice.kind === "project" ? String(choice.id) : choice.kind === "folder" ? "__folder" : ""}
                  onChange={(e) => {
                    const v = e.target.value;
                    if (v === "") setChoice({ kind: "none" });
                    else if (v === "__folder") browse();
                    else setChoice({ kind: "project", id: Number(v) });
                  }}
                >
                  <option value="">No project</option>
                  {(projects.data ?? [])
                    .filter((p) => p.path)
                    .map((p) => (
                      <option key={p.id} value={p.id}>
                        {p.name}
                      </option>
                    ))}
                  {choice.kind === "folder" && <option value="__folder">{choice.path}</option>}
                </select>
                <button className="btn" onClick={browse}>
                  <Icon name="folder" size={14} /> Browse…
                </button>
              </div>
              {folderLabel && (
                <div className="muted small" style={{ wordBreak: "break-all" }}>
                  The model will explore <code>{folderLabel}</code> with read-only tools (Read, Glob, Grep). It never changes files. This takes longer and
                  uses more tokens than a review without a project.
                </div>
              )}
            </div>

            <label className="field">
              Extra context (optional): what you have tried, constraints, background
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
