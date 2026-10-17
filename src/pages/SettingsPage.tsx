import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { api, type AppInfo, type AppSettings, type ContextMode, type Criterion, type RetentionStatus, type SourceKind } from "../api";
import { errorMessage, locationLabel } from "../lib/format";
import { useNav } from "../lib/router";
import { ErrorBox, Icon, Loading } from "../components/ui";
import { invalidateCriteriaCache } from "../components/AnalysisView";
import { useUpdateStatus } from "../lib/update";

function SourcesCard({ settings, update }: { settings: AppSettings; update: (s: AppSettings) => void }) {
  const nav = useNav();
  const [busy, setBusy] = useState(false);
  const [kind, setKind] = useState<SourceKind>("claude");

  const detect = async () => {
    setBusy(true);
    try {
      const sources = await api.detectSources();
      update({ ...settings, sources });
      nav.toast(`Found ${sources.length} source folder${sources.length === 1 ? "" : "s"}`);
    } catch (e) {
      nav.toast(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };
  const add = async () => {
    const dir = await open({ directory: true, title: kind === "claude" ? "Select a Claude Code 'projects' folder" : "Select a Codex 'sessions' folder" });
    if (!dir || Array.isArray(dir)) return;
    const location = dir.startsWith("\\\\wsl") ? `wsl:${dir.split("\\").filter(Boolean)[1] ?? "WSL"}` : "windows";
    const id = `${kind}:manual:${dir}`;
    if (settings.sources.some((s) => s.id === id)) return;
    update({ ...settings, sources: [...settings.sources, { id, kind, location, path: dir, enabled: true, auto: false }] });
  };
  const toggle = (id: string) =>
    update({ ...settings, sources: settings.sources.map((s) => (s.id === id ? { ...s, enabled: !s.enabled } : s)) });
  const remove = (id: string) => update({ ...settings, sources: settings.sources.filter((s) => s.id !== id) });

  return (
    <div className="card">
      <div className="row">
        <h2 className="grow" style={{ margin: 0 }}>
          Session sources
        </h2>
        <button className="btn sm" onClick={detect} disabled={busy}>
          {busy ? <span className="spinner" /> : <Icon name="refresh" size={14} />} Detect again
        </button>
      </div>
      <p className="muted small">
        Folders the app reads session logs from. New and changed files are imported automatically every {settings.pollSeconds} seconds. WSL folders
        are only scanned automatically while that distribution is running; <b>Rescan</b> on the Projects page scans everything.
      </p>
      {settings.sources.length === 0 && <div className="muted small">No folders configured. Click “Detect again” or add one.</div>}
      {settings.sources.map((s) => (
        <div className="source-row" key={s.id}>
          <input type="checkbox" checked={s.enabled} onChange={() => toggle(s.id)} title="Enabled" />
          <span className="chip">{s.kind === "claude" ? "Claude Code" : "Codex"}</span>
          <div style={{ minWidth: 0 }}>
            <div className="mono ellipsis" title={s.path}>
              {s.path}
            </div>
            <div className="muted small">
              {locationLabel(s.location)}
              {s.auto ? " · detected" : " · added by you"}
            </div>
          </div>
          <button className="btn ghost sm" onClick={() => remove(s.id)} title="Remove">
            <Icon name="x" size={14} />
          </button>
        </div>
      ))}
      <div className="row" style={{ marginTop: 10 }}>
        <select className="select" value={kind} onChange={(e) => setKind(e.target.value as SourceKind)}>
          <option value="claude">Claude Code folder</option>
          <option value="codex">Codex CLI folder</option>
        </select>
        <button className="btn sm" onClick={add}>
          <Icon name="plus" size={14} /> Add folder…
        </button>
      </div>
    </div>
  );
}

function CliCard({ settings, update }: { settings: AppSettings; update: (s: AppSettings) => void }) {
  const [status, setStatus] = useState<{ ok: boolean; text: string } | null>(null);
  const test = async () => {
    setStatus(null);
    try {
      setStatus({ ok: true, text: await api.claudeCliStatus() });
    } catch (e) {
      setStatus({ ok: false, text: errorMessage(e) });
    }
  };
  useEffect(() => {
    test();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  return (
    <div className="card">
      <h2>Claude Code CLI</h2>
      <p className="muted small" style={{ marginTop: 0 }}>
        Analyses run through <code>claude -p</code> with your existing Claude login, so they use your subscription and need no API key. Each run is
        isolated: no tools, no MCP servers, nothing saved as a session.
      </p>
      <div className="row">
        <input
          className="input grow"
          placeholder="Auto-detect (PATH, %USERPROFILE%\.local\bin, npm global)"
          value={settings.claudePath ?? ""}
          onChange={(e) => update({ ...settings, claudePath: e.target.value || null })}
        />
        <button className="btn" onClick={test}>
          Test
        </button>
      </div>
      {status && (
        <div className={`small ${status.ok ? "" : "error-text"}`} style={{ marginTop: 6, color: status.ok ? "var(--good)" : undefined }}>
          {status.ok ? "✓ " : ""}
          {status.text}
        </div>
      )}
    </div>
  );
}

function AnalysisCard({ settings, update }: { settings: AppSettings; update: (s: AppSettings) => void }) {
  const [newModel, setNewModel] = useState("");
  const addModel = () => {
    const m = newModel.trim();
    if (!m || settings.models.includes(m)) return;
    update({ ...settings, models: [...settings.models, m] });
    setNewModel("");
  };
  return (
    <div className="card">
      <h2>Analysis</h2>
      <div className="field" style={{ marginBottom: 12 }}>
        <span>Models offered</span>
        <div className="row wrap">
          {settings.models.map((m) => (
            <span className="chip" key={m}>
              {m}
              <button
                className="btn ghost sm"
                style={{ minHeight: 0, padding: 0 }}
                onClick={() => update({ ...settings, models: settings.models.filter((x) => x !== m) })}
                aria-label={`Remove ${m}`}
              >
                <Icon name="x" size={12} />
              </button>
            </span>
          ))}
          <input
            className="input"
            style={{ width: 200 }}
            placeholder="add model id or alias"
            value={newModel}
            onChange={(e) => setNewModel(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && addModel()}
          />
          <button className="btn sm" onClick={addModel}>
            Add
          </button>
        </div>
      </div>
      <div className="settings-grid">
        <label className="field">
          Default model
          <select className="select" value={settings.defaultModel} onChange={(e) => update({ ...settings, defaultModel: e.target.value })}>
            {settings.models.map((m) => (
              <option key={m}>{m}</option>
            ))}
          </select>
        </label>
        <label className="field">
          Default context
          <select className="select" value={settings.defaultMode} onChange={(e) => update({ ...settings, defaultMode: e.target.value as ContextMode })}>
            <option value="prompts">Prompts only</option>
            <option value="condensed">Condensed</option>
            <option value="full">Full</option>
          </select>
        </label>
        <label className="field">
          Max input size (tokens)
          <input
            className="input"
            type="number"
            min={50000}
            max={4000000}
            step={50000}
            value={settings.maxInputTokens}
            onChange={(e) => update({ ...settings, maxInputTokens: Number(e.target.value) })}
          />
          <span className="muted small">Default 1,000,000 (the context window of current Claude models). Larger inputs are reduced to fit.</span>
        </label>
        <label className="field">
          Timeout per analysis (minutes)
          <input className="input" type="number" min={1} max={120} value={settings.timeoutMinutes} onChange={(e) => update({ ...settings, timeoutMinutes: Number(e.target.value) })} />
        </label>
        <label className="field">
          Auto-import interval (seconds)
          <input className="input" type="number" min={5} value={settings.pollSeconds} onChange={(e) => update({ ...settings, pollSeconds: Number(e.target.value) })} />
        </label>
      </div>
      <label className="check" style={{ marginTop: 12 }}>
        <input type="checkbox" checked={settings.redact} onChange={(e) => update({ ...settings, redact: e.target.checked })} />
        Mask secrets (API keys, tokens, passwords, private keys) before sending anything to the model
      </label>
    </div>
  );
}

function RubricCard() {
  const nav = useNav();
  const [items, setItems] = useState<Criterion[] | null>(null);
  const [dirty, setDirty] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    api.listCriteria().then(setItems);
  }, []);
  if (!items) return <Loading />;
  const set = (i: number, patch: Partial<Criterion>) => {
    setItems(items.map((c, j) => (j === i ? { ...c, ...patch } : c)));
    setDirty(true);
  };
  const move = (i: number, d: number) => {
    const j = i + d;
    if (j < 0 || j >= items.length) return;
    const next = [...items];
    [next[i], next[j]] = [next[j], next[i]];
    setItems(next);
    setDirty(true);
  };
  const save = async () => {
    setError(null);
    try {
      setItems(await api.saveCriteria(items.filter((c) => c.name.trim())));
      invalidateCriteriaCache();
      setDirty(false);
      nav.toast("Rubric saved");
    } catch (e) {
      setError(errorMessage(e));
    }
  };
  const reset = async () => {
    if (!confirm("Replace your rubric with the default criteria?")) return;
    setItems(await api.resetCriteria());
    invalidateCriteriaCache();
    setDirty(false);
  };
  return (
    <div className="card">
      <div className="row">
        <h2 className="grow" style={{ margin: 0 }}>
          Scoring rubric
        </h2>
        <button className="btn sm" onClick={reset}>
          Reset to defaults
        </button>
        <button className="btn primary sm" onClick={save} disabled={!dirty}>
          Save rubric
        </button>
      </div>
      <p className="muted small">Each enabled criterion is scored 0–10. The weight is used when the model does not give an overall score.</p>
      {error && <ErrorBox error={error} />}
      {items.map((c, i) => (
        <div className="crit-edit" key={c.id ?? `new${i}`}>
          <input type="checkbox" checked={c.enabled} onChange={(e) => set(i, { enabled: e.target.checked })} title="Enabled" style={{ marginTop: 9 }} />
          <div className="col" style={{ gap: 4 }}>
            <input className="input" value={c.name} placeholder="Criterion name" onChange={(e) => set(i, { name: e.target.value })} />
            <textarea
              className="textarea"
              style={{ minHeight: 54 }}
              value={c.description}
              placeholder="What the reviewer should look for"
              onChange={(e) => set(i, { description: e.target.value })}
            />
          </div>
          <label className="field small">
            Weight
            <input className="input" type="number" min={0} step={0.5} value={c.weight} onChange={(e) => set(i, { weight: Number(e.target.value) })} />
          </label>
          <div className="col" style={{ gap: 2 }}>
            <button className="btn ghost sm" onClick={() => move(i, -1)} title="Move up">
              <Icon name="up" size={14} />
            </button>
            <button className="btn ghost sm" onClick={() => move(i, 1)} title="Move down">
              <Icon name="down" size={14} />
            </button>
            <button
              className="btn ghost sm danger"
              title="Delete"
              onClick={() => {
                setItems(items.filter((_, j) => j !== i));
                setDirty(true);
              }}
            >
              <Icon name="trash" size={14} />
            </button>
          </div>
        </div>
      ))}
      <button
        className="btn sm"
        style={{ marginTop: 10 }}
        onClick={() => {
          setItems([...items, { id: null, key: "", name: "", description: "", weight: 1, enabled: true }]);
          setDirty(true);
        }}
      >
        <Icon name="plus" size={14} /> Add criterion
      </button>
    </div>
  );
}

function UpdatesCard() {
  const nav = useNav();
  const status = useUpdateStatus();
  const [checking, setChecking] = useState(false);
  const check = async () => {
    setChecking(true);
    try {
      const s = await api.checkForUpdates();
      if (s.state === "up-to-date") nav.toast("You have the latest version");
    } catch (e) {
      nav.toast(errorMessage(e));
    } finally {
      setChecking(false);
    }
  };
  const install = async () => {
    try {
      await api.installUpdate();
    } catch (e) {
      nav.toast(errorMessage(e));
    }
  };
  const busy = checking || status?.state === "checking" || status?.state === "downloading";
  return (
    <div className="card">
      <h2>Updates</h2>
      <p className="muted small" style={{ marginTop: 0 }}>
        The app checks for new versions when it starts and every 6 hours, and downloads them in the background. Click “Restart to update”, or the
        update is installed quietly the next time the app starts.
      </p>
      <div className="row wrap">
        <span>
          Installed: <b>{status?.currentVersion ?? "…"}</b>
          {status?.latestVersion && status.latestVersion !== status.currentVersion && (
            <>
              {" "}
              · Available: <b>{status.latestVersion}</b>
            </>
          )}
        </span>
        <span className="spacer" />
        {status?.state === "ready" ? (
          <button className="btn primary" onClick={install}>
            Restart to update
          </button>
        ) : (
          <button className="btn" onClick={check} disabled={busy}>
            {busy ? <span className="spinner" /> : <Icon name="refresh" size={14} />} Check for updates
          </button>
        )}
      </div>
      {status?.state === "downloading" && (
        <div className="progress" style={{ marginTop: 10 }}>
          <div style={{ width: `${Math.round((status.progress ?? 0) * 100)}%` }} />
        </div>
      )}
      <div className="small" style={{ marginTop: 8 }}>
        {status?.state === "up-to-date" && <span style={{ color: "var(--good)" }}>✓ You have the latest version.</span>}
        {status?.state === "error" && <span className="error-text">Update check failed: {status.error}</span>}
        {status?.checkedAt && <span className="muted"> Last checked {new Date(status.checkedAt).toLocaleString()}.</span>}
      </div>
      {status?.notes && status.state === "ready" && <div className="preview-text" style={{ marginTop: 8, maxHeight: 160 }}>{status.notes}</div>}
    </div>
  );
}

function AboutCard() {
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [ret, setRet] = useState<RetentionStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    api.appInfo().then(setInfo);
    api.retentionStatus().then(setRet, (e) => setError(errorMessage(e)));
  }, []);
  return (
    <div className="card small">
      <h2>Data</h2>
      <div className="col" style={{ gap: 6 }}>
        {info && (
          <>
            <div>
              <span className="muted">Version:</span> {info.version}
            </div>
            <div style={{ wordBreak: "break-all" }}>
              <span className="muted">Database:</span> <code>{info.dbPath}</code>
            </div>
          </>
        )}
        {ret && (
          <div className="row wrap">
            <span>
              <span className="muted">Claude Code keeps sessions for:</span>{" "}
              {ret.cleanupPeriodDays != null ? `${ret.cleanupPeriodDays} days` : `${ret.effectiveDays} days (default)`}
            </span>
            {ret.atRisk && (
              <button className="btn sm" onClick={() => api.fixRetention().then(setRet, (e) => setError(errorMessage(e)))}>
                Keep sessions
              </button>
            )}
          </div>
        )}
        {error && <div className="error-text">{error}</div>}
      </div>
    </div>
  );
}

export function SettingsPage() {
  const nav = useNav();
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [dirty, setDirty] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    api.getSettings().then(setSettings, (e) => setError(errorMessage(e)));
  }, []);
  const update = (s: AppSettings) => {
    setSettings(s);
    setDirty(true);
  };
  const save = async () => {
    if (!settings) return;
    try {
      setSettings(await api.saveSettings(settings));
      setDirty(false);
      nav.toast("Settings saved");
    } catch (e) {
      setError(errorMessage(e));
    }
  };
  if (error) return <div className="page"><ErrorBox error={error} /></div>;
  if (!settings) return <Loading />;
  return (
    <div className="page">
      <div className="page-header">
        <div className="titles">
          <h1>Settings</h1>
        </div>
        <button className="btn primary" onClick={save} disabled={!dirty}>
          Save settings
        </button>
      </div>
      {dirty && <div className="banner info small">You have unsaved changes.</div>}
      <SourcesCard settings={settings} update={update} />
      <CliCard settings={settings} update={update} />
      <AnalysisCard settings={settings} update={update} />
      <RubricCard />
      <UpdatesCard />
      <AboutCard />
    </div>
  );
}
