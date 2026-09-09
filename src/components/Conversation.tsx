import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api, onBoardChanged } from "../api";
import type { Conversation, HubCard } from "../types";
import { STATE_LABEL } from "../types";
import { clock, noAutoFill, proseField } from "../util";
import { useAutoGrow } from "../hooks";
import { Markdown } from "./Markdown";

/** How many turns the conversation view asks for first. "Load everything" asks for the rest. */
export const TURNS_FIRST = 300;
/** As good as no limit: a transcript with more turns than this is not read in a side panel. */
const EVERYTHING = 100000;

/** One turn of the conversation. A reply renders as markdown, a prompt as it was typed. */
export function Turn({ role, text, at, hit }: { role: string; text: string; at: string | null; hit?: boolean }) {
  const who = role === "assistant" ? "Claude" : role === "peer" ? "sent in" : "you";
  return (
    <li className={`turn ${role} ${hit ? "hit" : ""}`}>
      <div className="who">
        <span>{who}</span>
        {at && <span className="when">{clock(at)}</span>}
      </div>
      {role === "assistant" ? <Markdown className="quote md" text={text} /> : <div className="quote">{text}</div>}
    </li>
  );
}

/** Read the conversation of one card, and read it again when told to. */
export function useConversation(node: string, cardId: string, enabled: boolean, changedAt: string | null | undefined) {
  const [conv, setConv] = useState<Conversation | null>(null);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  /** Whether the user asked for the whole transcript, so a reload keeps it. */
  const all = useRef(false);

  const load = useCallback(
    (everything?: boolean) => {
      if (everything) all.current = true;
      setBusy(true);
      setErr(null);
      api
        .conversation(node, cardId, all.current ? EVERYTHING : TURNS_FIRST)
        .then((cv) => {
          setConv(cv);
          setBusy(false);
        })
        .catch((e) => {
          setErr(String(e));
          setBusy(false);
        });
    },
    [node, cardId],
  );

  useEffect(() => {
    setConv(null);
    all.current = false;
  }, [node, cardId]);

  // The open conversation follows the session: a new turn arrives, the list grows.
  useEffect(() => {
    if (enabled) load();
  }, [enabled, changedAt, load]);

  return { conv, busy, err, load };
}

/** The search box, the count line and the list of turns. */
export function ConversationList({
  conv,
  busy,
  err,
  onLoadAll,
  autoFocus,
}: {
  conv: Conversation | null;
  busy: boolean;
  err: string | null;
  onLoadAll: () => void;
  autoFocus?: boolean;
}) {
  const [query, setQuery] = useState("");
  /** The turns that match the search, oldest first. */
  const turns = useMemo(() => {
    const all = conv?.messages ?? [];
    const needle = query.trim().toLowerCase();
    if (!needle) return all.map((m) => ({ ...m, hit: false }));
    return all.filter((m) => m.text.toLowerCase().includes(needle)).map((m) => ({ ...m, hit: true }));
  }, [conv, query]);
  return (
    <>
      <input
        {...noAutoFill}
        className="convsearch"
        type="search"
        value={query}
        autoFocus={autoFocus}
        onChange={(e) => setQuery(e.target.value)}
        placeholder="Search the conversation"
        aria-label="Search the conversation"
      />
      {err && <div className="nodeerr">{err}</div>}
      {conv && (
        <div className="hint">
          {query.trim() ? `${turns.length} of ${conv.messages.length} turns match` : `${conv.messages.length} of ${conv.total} turns shown`}
          {conv.total > conv.messages.length && (
            <>
              {" · "}
              <button className="linkbtn" disabled={busy} onClick={onLoadAll}>
                load everything
              </button>
            </>
          )}
          {busy ? " · loading…" : ""}
        </div>
      )}
      {!conv && busy && <div className="hint">Reading the transcript…</div>}
      <ul className="turns">
        {turns.map((m, i) => (
          <Turn key={`${m.at ?? ""}-${i}`} role={m.role} text={m.text} at={m.at} hit={m.hit} />
        ))}
      </ul>
    </>
  );
}

/** The address of the pop-out window for one card. The hash keeps it inside
 *  the one page the app serves. */
export function conversationUrl(node: string, cardId: string): string {
  const q = new URLSearchParams({ node, card: cardId });
  return `index.html#/conversation?${q}`;
}

/** Open the conversation of a card in a window of its own, or focus the one
 *  that is open already. In a browser preview it is a new tab. */
export async function popOutConversation(node: string, cardId: string, title: string) {
  const url = conversationUrl(node, cardId);
  if (!("__TAURI_INTERNALS__" in window)) {
    window.open(url.replace(/^index\.html/, window.location.pathname), "_blank");
    return;
  }
  const { WebviewWindow } = await import("@tauri-apps/api/webviewWindow");
  // One window per card. A label is a name, so the card id is made safe for it.
  const label = `conversation-${cardId.replace(/[^A-Za-z0-9_-]/g, "_")}`;
  const open = await WebviewWindow.getByLabel(label);
  if (open) {
    await open.setFocus();
    return;
  }
  new WebviewWindow(label, { url, title, width: 720, height: 860, minWidth: 420, minHeight: 400 });
}

/** The card and node named in the hash of a pop-out window, or null on the main page. */
export function conversationTarget(): { node: string; card: string } | null {
  const h = window.location.hash;
  if (!h.startsWith("#/conversation")) return null;
  const q = new URLSearchParams(h.slice(h.indexOf("?") + 1));
  const node = q.get("node");
  const card = q.get("card");
  return node && card ? { node, card } : null;
}

/** A window that holds one conversation: the turns, a search, and a prompt box.
 *  It reads the board for the card's title and state and follows it. */
export function ConversationWindow({ node, card }: { node: string; card: string }) {
  const [view, setView] = useState<HubCard | null>(null);
  const [lost, setLost] = useState(false);
  const [draft, setDraft] = useState("");
  const [note, setNote] = useState<{ text: string; err?: boolean } | null>(null);
  const grow = useAutoGrow("popout.compose", draft, 34, 300);

  const loadBoard = useCallback(() => {
    api
      .board()
      .then((b) => {
        const v = b.cards.find((c) => c.node_id === node && c.card.id === card) ?? null;
        setView(v);
        setLost(!v);
      })
      .catch((e) => setNote({ text: String(e), err: true }));
  }, [node, card]);

  useEffect(() => {
    loadBoard();
    const un = onBoardChanged(loadBoard);
    const t = window.setInterval(loadBoard, 30000);
    return () => {
      un();
      window.clearInterval(t);
    };
  }, [loadBoard]);

  useEffect(() => {
    if (view) document.title = `${view.title} · kari`;
  }, [view]);

  const { conv, busy, err, load } = useConversation(node, card, true, view?.last_activity_at);
  const running = !!view?.live?.alive;
  const canSend = !!view && draft.trim() !== "" && (running || !!(view.card.project_cwd ?? view.session?.cwd));

  const send = () => {
    const text = draft.trim();
    if (!canSend) return;
    setDraft("");
    setNote({ text: "Sending…" });
    api
      .sendPrompt(node, card, text)
      .then((r) => {
        setNote({ text: r });
        load();
      })
      .catch((e) => {
        setNote({ text: String(e), err: true });
        setDraft(text);
      });
  };

  return (
    <div className="popout">
      <header>
        <h2>{view?.title ?? (lost ? "This card is gone from the board." : "…")}</h2>
        {view && (
          <div className="hint">
            {view.node_name} · {STATE_LABEL[view.state]} · {view.reason}
          </div>
        )}
      </header>
      <div className="body">
        <ConversationList conv={conv} busy={busy} err={err} onLoadAll={() => load(true)} autoFocus />
      </div>
      <footer className="composer">
        <textarea
          {...proseField}
          {...grow}
          value={draft}
          disabled={!view}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
              e.preventDefault();
              send();
            }
          }}
          placeholder={running ? "Say something to the session… (⌘↵ sends)" : "The next step for a background run… (⌘↵ sends)"}
          aria-label="Prompt"
        />
        <div className="composerrow">
          <span className={`hint ${note?.err ? "err" : ""}`}>
            {note?.text ??
              (running
                ? `Goes into the running session (pid ${view!.live!.pid}).`
                : view
                  ? "The session is not running. A background job resumes it with this prompt."
                  : "")}
          </span>
          <button className="btn primary sm" disabled={!canSend} onClick={send}>
            {running ? "Send" : "Continue in bg"}
          </button>
        </div>
      </footer>
    </div>
  );
}
