// The URL-state layer. The query string is parsed into a route object, a
// route serializes back to a minimal URL, and useRouter owns navigation,
// popstate wiring and the <head> rewrite that describes every route to
// crawlers and share cards. Views receive `route` and `navigate` from App and
// link around with Link.

import { html, useState, useEffect } from "htm/preact";

import { SITE_ORIGIN, SYSTEMS, VIEWS } from "./config.js";

// Whether a navigation adds a history entry (clicks) or amends the current
// one (typing), so Back walks pages rather than keystrokes.
export const Nav = { PUSH: "push", REPLACE: "replace" };

/* ---------- URL state ----------
 *
 * The query string is the single source of truth for what the page shows, so
 * every view is a shareable link:
 *   ?cmd=python3                  one executable's versions
 *   ?pkg=texliveSmall&ver=2023    the commands one build of a package ships
 *   ?cmd=python3&sys=aarch64-linux   the same, on the other system
 *   ?q=ffmpeg                     a search
 *   ?view=packages                the widest packages
 *   ?view=stats                   the charts
 */
const ROUTE_PARAMS = ["q", "cmd", "pkg", "ver", "sys"];

function readRoute() {
  const p = new URLSearchParams(location.search);
  const view = VIEWS.includes(p.get("view")) ? p.get("view") : "commands";
  const route = { view };
  for (const k of ROUTE_PARAMS) {
    route[k] = p.get(k) || "";
  }
  // An unknown system in a hand-edited URL would fetch files that do not
  // exist, so it falls back rather than 404ing every request on the page.
  if (!SYSTEMS.includes(route.sys)) route.sys = "";
  return route;
}

export function routeUrl(route) {
  const p = new URLSearchParams();
  if (route.view && route.view !== "commands") p.set("view", route.view);
  for (const k of ROUTE_PARAMS) {
    if (route[k]) p.set(k, route[k]);
  }
  const query = p.toString();
  return query ? `?${query}` : location.pathname;
}

/* ---------- what each route tells a crawler ---------- */

function describe(route) {
  if (route.cmd) {
    return {
      title: `${route.cmd} · every version · omnibin`,
      description:
        `Every version of the \`${route.cmd}\` command nixpkgs ever shipped, ` +
        `with the store path and download size of each.`,
    };
  }
  if (route.pkg) {
    return {
      title: `${route.pkg} · omnibin`,
      description: `Every command the ${route.pkg} package ships.`,
    };
  }
  if (route.q) {
    return {
      title: `${route.q} · omnibin`,
      description: `Executables matching ${route.q} across every version nixpkgs ever shipped.`,
    };
  }
  if (route.view === "packages") {
    return {
      title: "Widest packages · omnibin",
      description: "The nixpkgs packages that ship the most executables.",
    };
  }
  if (route.view === "stats") {
    return {
      title: "Stats · omnibin",
      description:
        "How many executables nixpkgs has shipped, what they cost to fetch, " +
        "and how packages changed shape over thirteen years.",
    };
  }
  return {
    title: "omnibin · every binary nixpkgs ever shipped",
    description:
      "Search every executable nixpkgs ever shipped, at every version, with " +
      "the store path and the download each one costs.",
  };
}

// The head is rewritten per route so a shared link previews as the thing it
// points at rather than as the site's front page.
function applyHead(route) {
  const { title, description } = describe(route);
  document.title = title;

  const set = (selector, attr, value) => {
    const el = document.querySelector(selector);
    if (el) el.setAttribute(attr, value);
  };
  set('meta[name="description"]', "content", description);
  set('meta[property="og:title"]', "content", title);
  set('meta[property="og:description"]', "content", description);
  set('link[rel="canonical"]', "href", SITE_ORIGIN + routeUrl(route));
  set('meta[property="og:url"]', "content", SITE_ORIGIN + routeUrl(route));
}

export function useRouter() {
  const [route, setRoute] = useState(readRoute);

  useEffect(() => {
    const onPop = () => setRoute(readRoute());
    addEventListener("popstate", onPop);
    return () => removeEventListener("popstate", onPop);
  }, []);

  useEffect(() => applyHead(route), [route]);

  const navigate = (next, how = Nav.PUSH) => {
    const url = routeUrl(next);
    if (how === Nav.REPLACE) history.replaceState(null, "", url);
    else history.pushState(null, "", url);
    setRoute(next);
  };

  return [route, navigate];
}

/** An anchor that navigates in place. A real href, so middle click works. */
export function Link({ to, navigate, children, ...rest }) {
  const onClick = (e) => {
    // Let the browser handle anything that is not a plain left click.
    if (e.metaKey || e.ctrlKey || e.shiftKey || e.button !== 0) return;
    e.preventDefault();
    navigate(to);
  };
  return html`<a href=${routeUrl(to)} onClick=${onClick} ...${rest}>${children}</a>`;
}
