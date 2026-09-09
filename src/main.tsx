import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import MobileApp from "./mobile/MobileApp";
import { ConversationWindow, conversationTarget } from "./components/Conversation";
import "@fontsource-variable/bricolage-grotesque/opsz.css";
import "@fontsource/ibm-plex-sans/400.css";
import "@fontsource/ibm-plex-sans/500.css";
import "@fontsource/ibm-plex-sans/600.css";
import "@fontsource/ibm-plex-mono/400.css";
import "@fontsource/ibm-plex-mono/500.css";
import "./styles.css";

/** The phone layout: on a phone, or in a browser with `?mobile=1` for a preview. */
function isPhone(): boolean {
  if (new URLSearchParams(window.location.search).has("mobile")) return true;
  if (/Android|iPhone|iPad/i.test(navigator.userAgent)) return true;
  return window.matchMedia("(max-width: 720px)").matches;
}

/** A window that holds one conversation is the same page, opened on a hash. */
const popout = conversationTarget();

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    {popout ? <ConversationWindow node={popout.node} card={popout.card} /> : isPhone() ? <MobileApp /> : <App />}
  </React.StrictMode>,
);
