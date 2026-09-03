import { useEffect, useState } from "react";
import { api } from "../api";
import type { Column, DerivedState, NewTask, NodeStatus, Settings } from "../types";
import { ALL_STATES, RUN_MODELS, STATE_LABEL } from "../types";
import { nodeDot, relTime } from "../util";

function Modal({ title, children, footer, onClose }: { title: string; children: React.ReactNode; footer?: React.ReactNode; onClose: () => void }) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);
  return (
    <div className="backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="modal" role="dialog" aria-label={title}>
        <header>
          <h3>{title}</h3>
          <div className="spacer" />
          <button className="btn ghost sm" onClick={onClose} aria-label="Close">
            ✕
          </button>
        </header>
        <div className="body">{children}</div>
        {footer && <footer>{footer}</footer>}
      </div>
    </div>
  );
}

interface AddTaskProps {
  nodes: NodeStatus[];
  /** Node the picker starts on. The node filter chooses it, else the local node. */
  defaultNode: string;
  /** Projects taken from the cards of each node. Used until the node answers. */
  projectsByNode: Record<string, [string, string][]>;
  onClose: () => void;
  onSubmit: (nodeId: string, t: NewTask) => void;
}

export function AddTaskModal({ nodes, defaultNode, projectsByNode, onClose, onSubmit }: AddTaskProps) {
  const [node, setNode] = useState(defaultNode);
  const [loaded, setLoaded] = useState<Record<string, [string, string][]>>({});
  const [title, setTitle] = useState("");
  const [cwd, setCwd] = useState("");
  const [custom, setCustom] = useState("");
  const [prompt, setPrompt] = useState("");
  const [autoRun, setAutoRun] = useState(false);
  const [priority, setPriority] = useState(0);
  const [notes, setNotes] = useState("");
  const [model, setModel] = useState("");
  // The node answers with its projects. Until then the cards of that node name them.
  const projects = loaded[node] ?? projectsByNode[node] ?? [];
  const chosen = cwd === "__custom" || projects.some(([c]) => c === cwd) ? cwd : projects[0]?.[0] ?? "";
  const dir = chosen === "__custom" ? custom : chosen;

  useEffect(() => {
    let live = true;
    api
      .projects(node)
      .then((list) => {
        if (live && list.length > 0) setLoaded((m) => ({ ...m, [node]: list }));
      })
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [node]);

  return (
    <Modal
      title="New task"
      onClose={onClose}
      footer={
        <>
          <button className="btn" onClick={onClose}>
            Cancel
          </button>
          <button
            className="btn primary"
            disabled={!title.trim()}
            onClick={() =>
              onSubmit(node, {
                title: title.trim(),
                project_cwd: dir || null,
                run_prompt: prompt.trim() || null,
                auto_run: autoRun,
                priority,
                notes: notes.trim() || null,
                model: model || null,
              })
            }
          >
            Add
          </button>
        </>
      }
    >
      <div className="field">
        <label>Title</label>
        <input autoFocus value={title} onChange={(e) => setTitle(e.target.value)} placeholder="What needs to happen" />
      </div>
      {nodes.length > 1 && (
        <div className="field">
          <label>Node</label>
          <select value={node} onChange={(e) => setNode(e.target.value)}>
            {nodes.map((n) => (
              <option key={n.id} value={n.id}>
                {n.name}
                {n.online ? "" : " (offline)"}
              </option>
            ))}
          </select>
        </div>
      )}
      <div className="field">
        <label>Project directory</label>
        <select value={chosen} onChange={(e) => setCwd(e.target.value)}>
          {projects.map(([c, n]) => (
            <option key={c} value={c}>
              {n} — {c}
            </option>
          ))}
          <option value="__custom">Other path…</option>
        </select>
        {chosen === "__custom" && <input value={custom} onChange={(e) => setCustom(e.target.value)} placeholder="/absolute/path" />}
      </div>
      <div className="field">
        <label>Run prompt (what Claude gets when the task starts)</label>
        <textarea value={prompt} onChange={(e) => setPrompt(e.target.value)} placeholder="Leave empty to use the title" />
      </div>
      <div className="grid2">
        <div className="field">
          <label>Priority</label>
          <input type="number" value={priority} onChange={(e) => setPriority(Number(e.target.value))} />
        </div>
        <div className="field">
          <label>Model (optional)</label>
          <select value={model} onChange={(e) => setModel(e.target.value)}>
            {RUN_MODELS.map((m) => (
              <option key={m.value} value={m.value}>
                {m.label}
              </option>
            ))}
          </select>
        </div>
      </div>
      <label className="field inline">
        <input type="checkbox" checked={autoRun} onChange={(e) => setAutoRun(e.target.checked)} />
        <span>May run unattended</span>
      </label>
      <div className="field">
        <label>Notes</label>
        <textarea value={notes} onChange={(e) => setNotes(e.target.value)} />
      </div>
    </Modal>
  );
}

const COLORS = ["neutral", "green", "amber", "rust", "slate"];

export function ColumnsModal({ columns, onClose, onSave, onReset }: { columns: Column[]; onClose: () => void; onSave: (c: Column[]) => void; onReset: () => void }) {
  const [cols, setCols] = useState<Column[]>(() => [...columns].sort((a, b) => a.order - b.order).map((c) => ({ ...c })));
  const update = (i: number, patch: Partial<Column>) => setCols((cs) => cs.map((c, j) => (j === i ? { ...c, ...patch } : c)));
  const move = (i: number, d: number) =>
    setCols((cs) => {
      const n = [...cs];
      const j = i + d;
      if (j < 0 || j >= n.length) return cs;
      [n[i], n[j]] = [n[j], n[i]];
      return n.map((c, k) => ({ ...c, order: k }));
    });
  const toggleState = (i: number, s: DerivedState) =>
    update(i, { accepts: cols[i].accepts.includes(s) ? cols[i].accepts.filter((x) => x !== s) : [...cols[i].accepts, s] });
  const unassigned = ALL_STATES.filter((s) => s !== "stale" && !cols.some((c) => !c.hidden && c.accepts.includes(s)));
  return (
    <Modal
      title="Columns"
      onClose={onClose}
      footer={
        <>
          <button className="btn ghost" onClick={onReset}>
            Reset to defaults
          </button>
          <div className="spacer" />
          <button className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn primary" onClick={() => onSave(cols.map((c, k) => ({ ...c, order: k })))}>
            Save
          </button>
        </>
      }
    >
      <div className="hint">
        Each column accepts a set of derived states. A state that no visible column accepts falls back to the column that accepts Unknown. Stale cards are hidden unless a column
        accepts Stale.
      </div>
      {unassigned.length > 0 && <div className="hint" style={{ color: "var(--amber)" }}>Not shown anywhere: {unassigned.map((s) => STATE_LABEL[s]).join(", ")}</div>}
      {cols.map((c, i) => (
        <div key={c.id} className="colrow">
          <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
            <button className="btn ghost sm" onClick={() => move(i, -1)} aria-label="Up">
              ↑
            </button>
            <button className="btn ghost sm" onClick={() => move(i, 1)} aria-label="Down">
              ↓
            </button>
          </div>
          <input type="text" value={c.name} onChange={(e) => update(i, { name: e.target.value })} />
          <input type="number" placeholder="WIP" value={c.wip_limit ?? ""} onChange={(e) => update(i, { wip_limit: e.target.value === "" ? null : Number(e.target.value) })} title="WIP limit" />
          <select value={c.color ?? "neutral"} onChange={(e) => update(i, { color: e.target.value })}>
            {COLORS.map((k) => (
              <option key={k} value={k}>
                {k}
              </option>
            ))}
          </select>
          <div style={{ display: "flex", gap: 4 }}>
            <button className={`toggle ${c.hidden ? "" : "on"}`} onClick={() => update(i, { hidden: !c.hidden })}>
              {c.hidden ? "hidden" : "shown"}
            </button>
            <button className="btn ghost sm" onClick={() => setCols((cs) => cs.filter((_, j) => j !== i))} aria-label="Delete column">
              ✕
            </button>
          </div>
          <div className="accepts">
            {ALL_STATES.map((s) => (
              <button key={s} className={`toggle ${c.accepts.includes(s) ? "on" : ""}`} onClick={() => toggleState(i, s)}>
                {STATE_LABEL[s]}
              </button>
            ))}
          </div>
        </div>
      ))}
      <div>
        <button
          className="btn sm"
          onClick={() => setCols((cs) => [...cs, { id: `col_${Date.now().toString(36)}`, name: "New column", order: cs.length, accepts: [], wip_limit: null, color: "neutral", hidden: false }])}
        >
          + Column
        </button>
      </div>
    </Modal>
  );
}

function NodeRow({
  node,
  busy,
  localName,
  onLocalName,
  onRename,
  onToggle,
  onAway,
  onPair,
  onRemove,
}: {
  node: NodeStatus;
  busy: boolean;
  localName: string;
  onLocalName: (name: string) => void;
  onRename: (name: string) => void;
  onToggle: () => void;
  onAway: () => void;
  onPair: () => void;
  onRemove: () => void;
}) {
  const [confirm, setConfirm] = useState(false);
  const local = node.kind === "local";
  const rename = (e: { currentTarget: HTMLInputElement }) => {
    const name = e.currentTarget.value.trim();
    if (name && name !== node.name) onRename(name);
  };
  return (
    <div className="noderow">
      <span className={`pill ${node.online && node.enabled ? "on" : ""}`} title={node.enabled ? (node.online ? "online" : "offline") : "disabled"}>
        <span className={nodeDot(node)} />
        {local ? "this machine" : node.enabled ? (node.online ? "online" : "offline") : "off"}
      </span>
      {local ? (
        <input
          className="nodename"
          value={localName}
          placeholder="the host name"
          aria-label="Name of this machine"
          onChange={(e) => onLocalName(e.target.value)}
        />
      ) : (
        <input
          className="nodename"
          defaultValue={node.name}
          aria-label={`Name of ${node.name}`}
          onBlur={rename}
          onKeyDown={(e) => e.key === "Enter" && rename(e)}
        />
      )}
      <span className="hint">
        {local ? "local" : node.ssh_host ? `${node.ssh_host} · port ${node.remote_port}` : node.address ? node.address : `127.0.0.1:${node.remote_port}`}
        {node.version ? ` · ${node.version}` : ""}
        {node.last_seen ? ` · seen ${relTime(node.last_seen)} ago` : ""}
        {!local && !node.paired ? " · not paired" : ""}
        {node.primary ? " · columns: this device" : node.lease ? ` · columns: ${node.lease.hub_name}` : ""}
        {node.away_mode ? " · away mode" : ""}
      </span>
      {node.addresses?.length > 1 && <span className="hint">also at {node.addresses.slice(1).join(", ")}</span>}
      <div className="nodeacts">
        {node.online && (
          <button className="btn ghost sm" disabled={busy} onClick={onAway} title="Hold permission prompts for a remote answer, such as from a phone">
            {node.away_mode ? "Away mode off" : "Away mode on"}
          </button>
        )}
      </div>
      {!local && (
        <div className="nodeacts">
          <button className="btn ghost sm" disabled={busy} onClick={onToggle}>
            {node.enabled ? "Disable" : "Enable"}
          </button>
          <button className="btn ghost sm" disabled={busy} onClick={onPair}>
            Pair again
          </button>
          {confirm ? (
            <>
              <button
                className="btn danger sm"
                disabled={busy}
                onClick={() => {
                  setConfirm(false);
                  onRemove();
                }}
              >
                Remove it
              </button>
              <button className="btn ghost sm" onClick={() => setConfirm(false)}>
                Keep
              </button>
            </>
          ) : (
            <button className="btn ghost sm" disabled={busy} onClick={() => setConfirm(true)}>
              Remove
            </button>
          )}
        </div>
      )}
      {node.error && <div className="nodeerr">{node.error}</div>}
    </div>
  );
}

function NodesSection({
  nodes,
  primary,
  onNodesChanged,
  localName,
  onLocalName,
  listenPrivate,
  onListenPrivate,
}: {
  nodes: NodeStatus[];
  primary: boolean;
  onNodesChanged: () => void;
  localName: string;
  onLocalName: (name: string) => void;
  listenPrivate: boolean;
  onListenPrivate: (on: boolean) => void;
}) {
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [host, setHost] = useState("");
  const [address, setAddress] = useState("");
  const [token, setToken] = useState("");
  const [name, setName] = useState("");
  const [port, setPort] = useState(47311);
  const [msg, setMsg] = useState<string | null>(null);
  const [code, setCode] = useState<string | null>(null);
  const holder = nodes.find((n) => n.lease && !n.primary)?.lease?.hub_name;

  /** Every change reloads the board, and the fresh node list comes back with it. */
  const change = async (fn: () => Promise<unknown>) => {
    setBusy(true);
    setErr(null);
    try {
      await fn();
      onNodesChanged();
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  };

  const add = () =>
    change(async () => {
      await api.addNode({
        name: name.trim() || host.trim() || address.trim(),
        ssh_host: host.trim() || null,
        address: address.trim() || null,
        remote_port: port,
        token: token.trim() || null,
      });
      setHost("");
      setAddress("");
      setToken("");
      setName("");
    });

  const reach = nodes.find((n) => n.kind === "local")?.addresses ?? [];

  return (
    <div className="section">
      <h5>Nodes</h5>
      <div className="hint">
        kari connects over an SSH port forward and reads the node's token once. The node must run <code>kari-node serve</code>. A node on a private network takes an address and its token instead. A name is how the other kari
        instances see the machine. Empty means the host name.
      </div>
      <div className="primaryrow">
        <span className={`pill ${primary ? "on" : ""}`}>{primary ? "this device pushes the columns" : holder ? `${holder} pushes the columns` : "no primary yet"}</span>
        {!primary && (
          <button
            className="btn sm"
            disabled={busy}
            onClick={() =>
              change(async () => {
                setMsg(await api.claimPrimary());
              })
            }
          >
            Make this device primary
          </button>
        )}
        {msg && <span className="hint">{msg}</span>}
        <button
          className="btn ghost sm"
          disabled={busy}
          onClick={() =>
            change(async () => {
              setCode(code ? null : await api.pairingCode());
            })
          }
        >
          {code ? "Hide the pairing code" : "Show pairing code"}
        </button>
      </div>
      <div className="phonereach">
        <label className="checkrow">
          <input type="checkbox" checked={listenPrivate} onChange={(e) => onListenPrivate(e.target.checked)} />
          <span>Let a phone reach this machine</span>
        </label>
        <div className="hint">
          kari then answers on the private addresses of this machine as well, such as a VPN address. It never answers on a public address. A hub on a phone needs this, because a phone cannot open an SSH
          forward. The list is checked again every 20 seconds, so a VPN that comes up later needs no restart.
        </div>
        {listenPrivate && (
          <div className="hint">{reach.length ? `reachable at ${reach.join(", ")}` : "no private address yet. Bring the VPN up, then look again in 20 seconds."}</div>
        )}
      </div>
      {code && (
        <div className="paircode">
          <div className="hint">Paste this in the phone's Nodes tab. The code carries the addresses of every node, so the phone types none. It holds the node tokens: show it at home only, and hide it when done.</div>
          <textarea readOnly value={code} rows={3} onFocus={(e) => e.currentTarget.select()} />
        </div>
      )}
      <div className="nodelist">
        {nodes.map((n) => (
          <NodeRow
            key={n.id}
            node={n}
            busy={busy}
            localName={localName}
            onLocalName={onLocalName}
            onRename={(newName) => change(() => api.updateNode(n.id, { name: newName }))}
            onToggle={() => change(() => api.updateNode(n.id, { enabled: !n.enabled }))}
            onAway={() => change(() => api.setAwayMode(n.id, !n.away_mode))}
            onPair={() => change(() => api.pairNode(n.id))}
            onRemove={() => change(() => api.removeNode(n.id))}
          />
        ))}
      </div>
      <div className="nodeadd">
        <div className="field">
          <label>SSH host</label>
          <input value={host} onChange={(e) => setHost(e.target.value)} placeholder="ssh-host" />
          <div className="hint">an alias from ~/.ssh/config</div>
        </div>
        <div className="field">
          <label>Name</label>
          <input value={name} onChange={(e) => setName(e.target.value)} placeholder={host || "same as the SSH host"} />
        </div>
        <div className="field">
          <label>Port</label>
          <input type="number" value={port} onChange={(e) => setPort(Number(e.target.value))} />
        </div>
        <button className="btn sm" disabled={busy || !(host.trim() || address.trim())} onClick={add}>
          Add node
        </button>
        <div className="field">
          <label>Address (no SSH)</label>
          <input value={address} onChange={(e) => setAddress(e.target.value)} placeholder="ip:47311" />
          <div className="hint">on a private network, when there is no SSH forward</div>
        </div>
        <div className="field">
          <label>Token</label>
          <input value={token} onChange={(e) => setToken(e.target.value)} placeholder="the node's hook-token" type="password" />
          <div className="hint">needed with an address; read from the node's config directory</div>
        </div>
      </div>
      {err && <div className="nodeerr">{err}</div>}
    </div>
  );
}

export function SettingsModal({
  settings,
  hooksInstalled,
  hooksPort,
  nodes,
  primary,
  onNodesChanged,
  onClose,
  onSave,
  onSaveNow,
  onStopAll,
  onHooks,
}: {
  settings: Settings;
  hooksInstalled: boolean;
  hooksPort: number;
  nodes: NodeStatus[];
  primary: boolean;
  onNodesChanged: () => void;
  onClose: () => void;
  onSave: (s: Settings) => void;
  /** Write one setting now, without closing the modal. */
  onSaveNow: (s: Settings) => void;
  onStopAll: () => void;
  onHooks: (install: boolean) => void;
}) {
  const [s, setS] = useState<Settings>({ ...settings });
  const [paths, setPaths] = useState<Record<string, string> | null>(null);
  useEffect(() => {
    api.paths().then(setPaths).catch(() => {});
  }, []);
  const num = (k: keyof Settings) => (e: React.ChangeEvent<HTMLInputElement>) => setS({ ...s, [k]: Number(e.target.value) });
  return (
    <Modal
      title="Settings"
      onClose={onClose}
      footer={
        <>
          <button className="btn danger" onClick={onStopAll} title="Stop every background job kari started">
            Stop all kari jobs
          </button>
          <div className="spacer" />
          <button className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn primary" onClick={() => onSave(s)}>
            Save
          </button>
        </>
      }
    >
      <div className="grid2">
        <div className="field">
          <label>History window (days)</label>
          <input type="number" value={s.history_days} onChange={num("history_days")} />
        </div>
        <div className="field">
          <label>Done after inactivity (days)</label>
          <input type="number" value={s.done_after_days} onChange={num("done_after_days")} />
        </div>
        <div className="field">
          <label>Stale after inactivity (days)</label>
          <input type="number" value={s.stale_after_days} onChange={num("stale_after_days")} />
        </div>
        <div className="field">
          <label>Max parallel background jobs</label>
          <input type="number" value={s.max_parallel_bg} onChange={num("max_parallel_bg")} />
        </div>
        <div className="field">
          <label>Terminal for Jump in</label>
          <select value={s.terminal_app} onChange={(e) => setS({ ...s, terminal_app: e.target.value })}>
            <option value="iTerm">iTerm2</option>
            <option value="Terminal">Terminal.app</option>
            <option value="Ghostty">Ghostty</option>
          </select>
        </div>
        <div className="field">
          <label>Default model for runs</label>
          <select value={s.default_run_model} onChange={(e) => setS({ ...s, default_run_model: e.target.value })}>
            {RUN_MODELS.map((m) => (
              <option key={m.value} value={m.value}>
                {m.label}
              </option>
            ))}
          </select>
        </div>
        <div className="field">
          <label>Default permission mode for unattended runs</label>
          <select value={s.default_permission_mode} onChange={(e) => setS({ ...s, default_permission_mode: e.target.value })}>
            {["auto", "acceptEdits", "bypassPermissions", "plan", "default"].map((m) => (
              <option key={m} value={m}>
                {m}
              </option>
            ))}
          </select>
        </div>
      </div>
      <NodesSection
        nodes={nodes}
        primary={primary}
        onNodesChanged={onNodesChanged}
        localName={s.node_name ?? ""}
        onLocalName={(name) => setS({ ...s, node_name: name })}
        listenPrivate={s.listen_private}
        onListenPrivate={(on) => {
          setS({ ...s, listen_private: on });
          // Saved at once: the pairing code needs the addresses now, not after
          // the Save button.
          onSaveNow({ ...settings, listen_private: on });
        }}
      />
      <div className="section">
        <h5>Live hooks</h5>
        <div className="hint">
          Claude Code can tell kari the moment a session starts, stops, or waits for a permission. kari adds a small relay script to <code>~/.claude/settings.json</code> and keeps a backup. The relay
          never blocks a session when kari is closed.
        </div>
        <div style={{ display: "flex", gap: 8, alignItems: "center", marginTop: 8 }}>
          <span className={`pill ${hooksInstalled ? "on" : ""}`}>
            <span className="dot" />
            {hooksInstalled ? `hooks installed · port ${hooksPort}` : "hooks not installed"}
          </span>
          {hooksInstalled ? (
            <button className="btn sm" onClick={() => onHooks(false)}>
              Remove hooks
            </button>
          ) : (
            <button className="btn primary sm" onClick={() => onHooks(true)}>
              Install hooks
            </button>
          )}
        </div>
      </div>
      <div className="section">
        <h5>Summaries</h5>
        <div className="hint">kari asks Haiku for a two-sentence narrative after a turn ends. Calls are capped per hour and skip sessions older than the recent window.</div>
        <div className="grid2" style={{ marginTop: 8 }}>
          <label className="field inline">
            <input type="checkbox" checked={s.summaries_enabled} onChange={(e) => setS({ ...s, summaries_enabled: e.target.checked })} />
            <span>Summaries on</span>
          </label>
          <div className="field">
            <label>Model</label>
            <input value={s.summary_model} onChange={(e) => setS({ ...s, summary_model: e.target.value })} />
          </div>
          <div className="field">
            <label>Max calls per hour</label>
            <input type="number" value={s.summaries_per_hour} onChange={num("summaries_per_hour")} />
          </div>
          <div className="field">
            <label>Only sessions active in the last (hours)</label>
            <input type="number" value={s.summary_recent_hours} onChange={num("summary_recent_hours")} />
          </div>
        </div>
      </div>
      <div className="section">
        <h5>Scheduling</h5>
        <div className="hint">
          kari offers a plan when quota would expire unused. It never starts a card that is not marked "May run unattended". The planner keeps a reserve free during
          working hours and never fills a window past the ceiling.
        </div>
        <label className="field inline" style={{ marginTop: 8 }}>
          <input type="checkbox" checked={s.proposals_enabled} onChange={(e) => setS({ ...s, proposals_enabled: e.target.checked })} />
          <span>Offer plans</span>
        </label>
        <div className="grid2" style={{ marginTop: 8 }}>
          <div className="field">
            <label>Weekly window unused above (percent)</label>
            <input type="number" value={s.weekly_unused_pct} onChange={num("weekly_unused_pct")} />
          </div>
          <div className="field">
            <label>and resets within (hours)</label>
            <input type="number" value={s.weekly_hours_before_reset} onChange={num("weekly_hours_before_reset")} />
          </div>
          <div className="field">
            <label>5-hour window below (percent)</label>
            <input type="number" value={s.five_hour_idle_pct} onChange={num("five_hour_idle_pct")} />
          </div>
          <div className="field">
            <label>and nobody worked for (minutes)</label>
            <input type="number" value={s.idle_minutes} onChange={num("idle_minutes")} />
          </div>
          <div className="field">
            <label>Working hours start</label>
            <input type="number" min={0} max={23} value={s.working_hours_start} onChange={num("working_hours_start")} />
          </div>
          <div className="field">
            <label>Working hours end</label>
            <input type="number" min={0} max={23} value={s.working_hours_end} onChange={num("working_hours_end")} />
          </div>
          <div className="field">
            <label>Keep free in working hours (percent)</label>
            <input type="number" value={s.working_hours_reserve_pct} onChange={num("working_hours_reserve_pct")} />
          </div>
          <div className="field">
            <label>Never fill past (percent)</label>
            <input type="number" value={s.fill_ceiling_pct} onChange={num("fill_ceiling_pct")} />
          </div>
        </div>
      </div>
      <div className="section">
        <h5>Autopilot</h5>
        <div className="hint">
          With autopilot on, a weekly-reset plan starts by itself. kari still sends a notice and the plan panel keeps a Stop button. Only cards marked "May run
          unattended" are eligible, and the parallel cap holds.
        </div>
        <div className="grid2" style={{ marginTop: 8 }}>
          <label className="field inline">
            <input type="checkbox" checked={s.autopilot} onChange={(e) => setS({ ...s, autopilot: e.target.checked })} />
            <span>Start weekly-reset plans without asking</span>
          </label>
          <div className="field">
            <label>Jobs autopilot may start at once</label>
            <input type="number" min={1} value={s.autopilot_max_jobs} onChange={num("autopilot_max_jobs")} />
          </div>
          <label className="field inline">
            <input type="checkbox" checked={s.prefer_herdr} onChange={(e) => setS({ ...s, prefer_herdr: e.target.checked })} />
            <span>Open new sessions in a herdr pane</span>
          </label>
          <div className="field">
            <label>Warn when weekly unused is above (percent)</label>
            <input type="number" value={s.weekly_warn_unused_pct} onChange={num("weekly_warn_unused_pct")} />
          </div>
        </div>
      </div>
      <div className="section">
        <h5>Quota tracking</h5>
        <div className="hint">
          kari reads the 5-hour and 7-day windows from the Claude Code status line. Run the installer once. It wraps your current status line command and keeps a backup.
        </div>
        <pre className="code">scripts/install-statusline.sh</pre>
        <label className="field inline" style={{ marginTop: 8 }}>
          <input type="checkbox" checked={s.usage_endpoint_enabled} onChange={(e) => setS({ ...s, usage_endpoint_enabled: e.target.checked })} />
          <span>Ask the usage endpoint when no session refreshed the status line for 5 minutes</span>
        </label>
        <div className="hint">
          The endpoint is undocumented. kari reads the Claude Code login token from the keychain and asks at most once every 3 minutes. macOS shows a keychain prompt the first time.
        </div>
        {paths && (
          <div className="hint">
            Samples land in <code>{paths.rate_limits}</code>. Database: <code>{paths.db}</code>. kari {paths.version}.
          </div>
        )}
      </div>
    </Modal>
  );
}
