import { useEffect, useState } from "react";
import { api, type AnalysisRequest, type BuiltContext, type ContextMode, type Scope } from "../api";
import { errorMessage, fmtNum } from "../lib/format";
import { Dialog, Icon } from "./ui";

const MODES: { id: ContextMode; title: string; desc: string }[] = [
  { id: "prompts", title: "Prompts only", desc: "Just what you typed. Cheapest, but the model cannot see what each prompt led to." },
  { id: "condensed", title: "Condensed", desc: "Prompts and replies, tool calls as one-liners, only failed tool output. Good balance." },
  { id: "full", title: "Full", desc: "Everything, including tool inputs and outputs (each item capped). Most accurate, most tokens." },
];

export function ModelPicker({ models, value, onChange }: { models: string[]; value: string; onChange: (m: string) => void }) {
  const custom = !models.includes(value);
  return (
    <div className="row">
      <select className="select grow" value={custom ? "__custom" : value} onChange={(e) => onChange(e.target.value === "__custom" ? "" : e.target.value)}>
        {models.map((m) => (
          <option key={m} value={m}>
            {m}
          </option>
        ))}
        <option value="__custom">Other…</option>
      </select>
      {custom && <input className="input grow" placeholder="model id or alias, e.g. opus" value={value} onChange={(e) => onChange(e.target.value)} autoFocus />}
    </div>
  );
}

export function AnalyzeDialog({
  scope,
  targetId,
  title,
  onClose,
  onStarted,
}: {
  scope: Scope;
  targetId: number | null;
  title: string;
  onClose: () => void;
  onStarted: (analysisId: number) => void;
}) {
  const [models, setModels] = useState<string[]>([]);
  const [model, setModel] = useState("");
  const [mode, setMode] = useState<ContextMode>("condensed");
  const [preview, setPreview] = useState<BuiltContext | null>(null);
  const [previewErr, setPreviewErr] = useState<string | null>(null);
  const [inputText, setInputText] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.getSettings().then((s) => {
      setModels(s.models);
      setModel(s.defaultModel || s.models[0] || "");
      setMode(s.defaultMode);
    });
  }, []);

  const request: AnalysisRequest = { scope, targetId, mode, model };

  useEffect(() => {
    setPreview(null);
    setPreviewErr(null);
    api.previewAnalysis({ scope, targetId, mode, model: "preview" }).then(setPreview, (e) => setPreviewErr(errorMessage(e)));
  }, [scope, targetId, mode]);

  const start = async () => {
    setBusy(true);
    setError(null);
    try {
      const id = await api.startAnalysis(request);
      onStarted(id);
    } catch (e) {
      setError(errorMessage(e));
      setBusy(false);
    }
  };

  if (inputText != null) {
    return (
      <Dialog title="Exact input sent to the model" onClose={() => setInputText(null)} wide>
        <p className="muted small" style={{ marginTop: 0 }}>
          Secrets are masked as [REDACTED] when redaction is on (Settings). Your local database keeps the original text.
        </p>
        <div className="preview-text">{inputText}</div>
        <div className="actions">
          <button className="btn" onClick={() => setInputText(null)}>
            Back
          </button>
        </div>
      </Dialog>
    );
  }

  return (
    <Dialog title={`Analyse ${title}`} onClose={onClose}>
      <div className="col" style={{ gap: 14 }}>
        <label className="field">
          Model
          <ModelPicker models={models} value={model} onChange={setModel} />
        </label>
        <div className="field">
          <span style={{ fontSize: 13, color: "var(--text-2)" }}>How much context to send</span>
          <div className="col" style={{ gap: 6 }}>
            {MODES.map((m) => (
              <label key={m.id} className={`mode-option${mode === m.id ? " on" : ""}`}>
                <input type="radio" name="mode" checked={mode === m.id} onChange={() => setMode(m.id)} />
                <div>
                  <div style={{ fontWeight: 600 }}>{m.title}</div>
                  <div className="muted small">{m.desc}</div>
                </div>
              </label>
            ))}
          </div>
        </div>
        <div className="small">
          {previewErr ? (
            <span className="error-text">{previewErr}</span>
          ) : preview ? (
            <div className="col" style={{ gap: 4 }}>
              <div className="row wrap">
                <span>
                  ≈ <b>{fmtNum(preview.estTokens)}</b> input tokens · {preview.promptCount} prompt{preview.promptCount === 1 ? "" : "s"}
                </span>
                <span className="spacer" />
                <a onClick={() => api.previewAnalysisText({ ...request, model: "preview" }).then(setInputText)}>
                  <span className="row" style={{ gap: 4 }}>
                    <Icon name="eye" size={14} /> View exact input
                  </span>
                </a>
              </div>
              {preview.notes.map((n) => (
                <div key={n} className="row" style={{ color: "var(--warn)" }}>
                  <Icon name="alert" size={14} /> {n}
                </div>
              ))}
            </div>
          ) : (
            <span className="muted">Measuring…</span>
          )}
        </div>
        {error && <div className="error-text small">{error}</div>}
      </div>
      <div className="actions">
        <button className="btn" onClick={onClose}>
          Cancel
        </button>
        <button className="btn primary" onClick={start} disabled={busy || !model.trim() || !!previewErr}>
          <Icon name="play" size={14} /> {busy ? "Starting…" : "Analyse"}
        </button>
      </div>
    </Dialog>
  );
}
