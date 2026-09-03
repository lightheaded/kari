import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import MobileApp from "./mobile/MobileApp";
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

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>{isPhone() ? <MobileApp /> : <App />}</React.StrictMode>,
);
