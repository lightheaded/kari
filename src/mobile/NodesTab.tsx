import { useState } from "react";
import { api } from "../api";
import type { HubBoard, NodeStatus, Settings } from "../types";
import { nodeDot, relTime } from "../util";

interface Props {
  board: HubBoard;
  settings: Settings | null;
  onChanged: () => void;
  onSettingsChanged: () => void;
  onAction: (fn: () => Promise<unknown>, ok?: string) => Promise<void>;
}

/** One node in a pairing code. The desktop writes it; the phone reads it. */
interface PairEntry {
  name: string;
  /** Every address the node answers on. The hub keeps the one that works. */
  addresses: string[];
  token: string;
}

/** Read a code of version 1 (one address) or version 2 (a list). */
function parseCode(text: string): PairEntry[] {
  const v = JSON.parse(text) as { kari?: number; nodes?: { name?: string; address?: string | null; addresses?: string[]; token?: string }[] };
  if (!v.kari || v.kari > 2 || !Array.isArray(v.nodes)) throw new Error("not a kari pairing code");
  return v.nodes.map((n) => {
    const list = [...(n.address ? [String(n.address)] : []), ...(Array.isArray(n.addresses) ? n.addresses.map(String) : [])];
    return { name: String(n.name ?? ""), addresses: list.filter((a, i) => a.trim() && list.indexOf(a) === i), token: String(n.token ?? "") };
  });
}

/** Status per node, the primary control, pairing, and this device's name. */
export function NodesTab({ board, settings, onChanged, onSettingsChanged, onAction }: Props) {
  const [code, setCode] = useState("");
  const [entries, setEntries] = useState<PairEntry[] | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [name, setName] = useState<string | null>(null);
  const [confirm, setConfirm] = useState<string | null>(null);
  const holder = board.nodes.find((n) => n.lease && !n.primary)?.lease?.hub_name;

  const read = () => {
    try {
      setEntries(parseCode(code.trim()));
      setErr(null);
    } catch (e) {
      setErr(String(e));
    }
  };

  const ready = (e: PairEntry) => e.addresses.length > 0 && !!e.token;

  const addAll = () =>
    onAction(async () => {
      let n = 0;
      for (const e of entries ?? []) {
        if (!ready(e)) continue;
        await api.addNode({ name: e.name, ssh_host: null, address: e.addresses[0], addresses: e.addresses, remote_port: 47311, token: e.token });
        n++;
      }
      setEntries(null);
      setCode("");
      return `${n} node(s) added`;
    }, "Nodes added").then(onChanged);

  const deviceName = name ?? settings?.node_name ?? "";

  return (
    <div className="mnodes">
      <header className="mhead">
        <span className="wordmark">Nodes</span>
        <span className="mhead-meta">{board.primary ? "this device pushes the columns" : holder ? `${holder} pushes the columns` : "no primary yet"}</span>
      </header>

      {!board.primary && (
        <div className="msection">
          <button className="btn primary" onClick={() => onAction(() => api.claimPrimary(), "This device is primary")}>
            Make this device primary
          </button>
          <div className="hint">The columns you edit here then reach every node. The other hub follows.</div>
        </div>
      )}

      <div className="mlist">
        {board.nodes.map((n: NodeStatus) => (
          <div key={n.id} className="mnode">
            <div className="mnode-head">
              <span className={nodeDot(n)} />
              <span className="mnode-name">{n.name}</span>
              <span className="hint">{n.enabled ? (n.online ? "online" : "offline") : "off"}</span>
            </div>
            <div className="hint">
              {n.address ?? n.ssh_host ?? "no address"}
              {n.version ? ` · ${n.version}` : ""}
              {n.last_seen ? ` · seen ${relTime(n.last_seen)} ago` : ""}
              {n.primary ? " · columns: this device" : n.lease ? ` · columns: ${n.lease.hub_name}` : ""}
            </div>
            {(n.addresses ?? []).length > 1 && <div className="hint">also tries {n.addresses.filter((a) => a !== n.address).join(", ")}</div>}
            {n.error && <div className="nodeerr">{n.error}</div>}
            {n.online && (
              <label className="field inline">
                <input type="checkbox" checked={n.away_mode} onChange={(e) => onAction(() => api.setAwayMode(n.id, e.target.checked), e.target.checked ? "Away mode on" : "Away mode off").then(onChanged)} />
                <span>Away mode: hold permission prompts for this phone</span>
              </label>
            )}
            {n.kind === "remote" && (
              <div className="macts">
                <button className="btn ghost sm" onClick={() => onAction(() => api.updateNode(n.id, { enabled: !n.enabled }), n.enabled ? "Disabled" : "Enabled").then(onChanged)}>
                  {n.enabled ? "Disable" : "Enable"}
                </button>
                {confirm === n.id ? (
                  <>
                    <button className="btn danger sm" onClick={() => onAction(() => api.removeNode(n.id), "Removed").then(onChanged)}>
                      Remove it
                    </button>
                    <button className="btn ghost sm" onClick={() => setConfirm(null)}>
                      Keep
                    </button>
                  </>
                ) : (
                  <button className="btn ghost sm" onClick={() => setConfirm(n.id)}>
                    Remove
                  </button>
                )}
              </div>
            )}
          </div>
        ))}
        {board.nodes.length === 0 && <div className="empty">No nodes yet. Paste a pairing code from the desktop.</div>}
      </div>

      <div className="msection">
        <h5>Pair with a code</h5>
        <div className="hint">On the desktop: Settings → Nodes → Show pairing code. Paste it here. The code holds the node tokens, so paste it at home and nowhere else.</div>
        <textarea value={code} onChange={(e) => setCode(e.target.value)} rows={4} placeholder='{"kari":2,"nodes":[…]}' />
        <div className="macts">
          <button className="btn sm" disabled={!code.trim()} onClick={read}>
            Read the code
          </button>
        </div>
        {err && <div className="nodeerr">{err}</div>}
        {entries && (
          <div className="mlist">
            {entries.map((e, i) => (
              <div key={i} className="mnode">
                <div className="mnode-head">
                  <span className="mnode-name">{e.name || "node"}</span>
                  <span className="hint">{e.token ? "token ok" : "no token"}</span>
                </div>
                {e.addresses.length > 0 ? (
                  <div className="hint">{e.addresses.join(", ")}</div>
                ) : (
                  <div className="field">
                    <label>Address</label>
                    <input
                      placeholder="ip:47311, the node's address on the VPN"
                      onChange={(ev) => setEntries((all) => (all ?? []).map((x, j) => (j === i ? { ...x, addresses: ev.target.value.trim() ? [ev.target.value.trim()] : [] } : x)))}
                    />
                    <div className="hint">This node carried no address. Turn on "Let a phone reach this machine" on the desktop, then show the code again.</div>
                  </div>
                )}
              </div>
            ))}
            <button className="btn primary" disabled={!entries.some(ready)} onClick={addAll}>
              Add {entries.filter(ready).length} node(s)
            </button>
          </div>
        )}
      </div>

      <div className="msection">
        <h5>This device</h5>
        <div className="field">
          <label>Name</label>
          <input value={deviceName} onChange={(e) => setName(e.target.value)} placeholder="phone" />
          <div className="hint">The nodes show this name as the holder of the columns. Takes effect after a restart.</div>
        </div>
        <div className="macts">
          <button
            className="btn sm"
            disabled={name === null || !settings || name === settings.node_name}
            onClick={() => settings && onAction(() => api.setSettings({ ...settings, node_name: name ?? "" }), "Saved").then(onSettingsChanged)}
          >
            Save
          </button>
        </div>
        <div className="hint">
          hub {board.hub_name} · {board.hub_id.slice(0, 8)}
        </div>
      </div>
    </div>
  );
}
