import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { api, onBoardChanged } from "../api";
import type { Conversation, HubCard } from "../types";
import { STATE_LABEL } from "../types";
import { clock, noAutoFill, proseField } from "../util";
import { useAutoGrow } from "../hooks";
import { Markdown } from "./Markdown";

/** How many turns the conversation view asks for first. A long transcript is
 *  read from its end, so the newest turns are the ones worth waiting for. */
export const TURNS_FIRST = 200;
/** How many more turns one press of "Earlier turns" adds. */
const PAGE = 400;
/** As good as no limit: no transcript holds more turns than this. */
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

/** Read the conversation of one card, and read it again when told to.
 *
 *  The view holds the newest `TURNS_FIRST` turns and asks for older ones from
 *  there, one page at a time. It keeps the count it reached, so a new turn in
 *  a live session never takes back the older turns the reader asked for. */
export function useConversation(node: string, cardId: string, enabled: boolean, changedAt: string | null | undefined) {
  const [conv, setConv] = useState<Conversation | null>(null);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  /** How many turns the view asks the node for. */
  const limit = useRef(TURNS_FIRST);

  const load = useCallback(
    (want?: number) => {
      if (want) limit.current = Math.max(limit.current, want);
      setBusy(true);
      setErr(null);
      api
        .conversation(node, cardId, limit.current)
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

  /** One page more of the older turns. */
  const more = useCallback(() => load(limit.current + PAGE), [load]);
  /** Every turn there is, however long the transcript is. */
  const all = useCallback(() => load(EVERYTHING), [load]);

  useEffect(() => {
    setConv(null);
    limit.current = TURNS_FIRST;
  }, [node, cardId]);

  // The open conversation follows the session: a new turn arrives, the list grows.
  useEffect(() => {
    if (enabled) load();
  }, [enabled, changedAt, load]);

  return { conv, busy, err, load, more, all };
}

/** The element that scrolls this list: the body of the drawer, of the sheet or
 *  of the window. The list sits inside it, so it is found by walking up from
 *  the list itself. This keeps the list usable in all three without a prop
 *  that names its own container. */
function scrollerOf(el: HTMLElement | null): HTMLElement | null {
  for (let p = el?.parentElement ?? null; p; p = p.parentElement) {
    const oy = getComputedStyle(p).overflowY;
    if (oy === "auto" || oy === "scroll") return p;
  }
  return null;
}

/** How near the end of the list counts as reading the newest turn, in pixels. */
const AT_END = 80;

/** The search box, the count line and the list of turns.
 *
 *  A transcript can hold thousands of turns, and every reply is markdown, so
 *  the list never renders the whole of one by itself. It shows the newest
 *  turns and loads the older ones when the reader asks. */
export function ConversationList({
  conv,
  busy,
  err,
  onMore,
  onLoadAll,
  autoFocus,
  tail,
}: {
  conv: Conversation | null;
  busy: boolean;
  err: string | null;
  /** Load one page more of the older turns. */
  onMore: () => void;
  /** Load every turn there is. */
  onLoadAll: () => void;
  autoFocus?: boolean;
  /** Open on the newest turn, and follow it while the reader sits there. For
   *  a view that holds the conversation and nothing else. */
  tail?: boolean;
}) {
  const [query, setQuery] = useState("");
  const listRef = useRef<HTMLUListElement | null>(null);
  /** Where the reader was when a page of older turns was asked for. */
  const older = useRef<{ el: HTMLElement; height: number } | null>(null);
  /** Whether the reader sits at the newest turn. */
  const atEnd = useRef(true);
  /** The turns that match the search, oldest first.
   *
   *  Each one carries its place in the whole transcript. That number is the
   *  identity of the turn: it does not move when older turns load above it or
   *  a new one arrives below. A key made from the position in the list would
   *  change for every turn as soon as a page loaded, React would build the
   *  list again, and the browser would drop the reading position with it. */
  const turns = useMemo(() => {
    const all = conv?.messages ?? [];
    const first = conv ? conv.total - all.length : 0;
    const needle = query.trim().toLowerCase();
    const numbered = all.map((m, i) => ({ ...m, n: first + i }));
    if (!needle) return numbered.map((m) => ({ ...m, hit: false }));
    return numbered.filter((m) => m.text.toLowerCase().includes(needle)).map((m) => ({ ...m, hit: true }));
  }, [conv, query]);

  /** Ask for older turns, and remember the reading position first. */
  const earlier = (ask: () => void) => {
    const el = scrollerOf(listRef.current);
    older.current = el ? { el, height: el.scrollHeight } : null;
    ask();
  };

  // A page of older turns lands above the reader and would push the turn they
  // read off the screen. The list grew by a known amount, so the scroll moves
  // by the same amount and the turn stays where it was.
  useLayoutEffect(() => {
    const from = older.current;
    older.current = null;
    const el = from?.el ?? scrollerOf(listRef.current);
    if (!el) return;
    if (from) el.scrollTop += el.scrollHeight - from.height;
    else if (tail && atEnd.current) el.scrollTop = el.scrollHeight;
  }, [conv, tail]);

  // A new turn follows the reader only while they are at the end of the list.
  // A reader who scrolled up is reading, and must not be dragged away.
  useEffect(() => {
    if (!tail) return;
    const el = scrollerOf(listRef.current);
    if (!el) return;
    const onScroll = () => {
      atEnd.current = el.scrollHeight - el.scrollTop - el.clientHeight < AT_END;
    };
    onScroll();
    el.addEventListener("scroll", onScroll, { passive: true });
    return () => el.removeEventListener("scroll", onScroll);
  }, [tail, conv]);

  const rest = conv ? conv.total - conv.messages.length : 0;
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
          {query.trim() && rest > 0 ? ` · ${rest} earlier turns are not loaded` : ""}
          {busy ? " · loading…" : ""}
        </div>
      )}
      {!conv && busy && <div className="hint">Reading the transcript…</div>}
      {conv && rest > 0 && (
        <div className="convmore">
          <button className="btn ghost sm" disabled={busy} onClick={() => earlier(onMore)}>
            ↑ {Math.min(rest, PAGE)} earlier turns
          </button>
          <button className="linkbtn" disabled={busy} onClick={() => earlier(onLoadAll)}>
            all {conv.total}
          </button>
        </div>
      )}
      <ul className="turns" ref={listRef}>
        {turns.map((m) => (
          <Turn key={m.n} role={m.role} text={m.text} at={m.at} hit={m.hit} />
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

  const { conv, busy, err, load, more, all } = useConversation(node, card, true, view?.last_activity_at);
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
        <ConversationList conv={conv} busy={busy} err={err} onMore={more} onLoadAll={all} tail autoFocus />
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
