// The site is a Preact app written with htm tagged templates, so there is no
// build step and no JSX. "htm/preact" resolves through the import map in
// index.html to a pinned, integrity-checked single-file CDN bundle. The app
// is plain ES modules under js/: this entry module composes the router and
// the views and boots the render.

import { html, render, useMemo } from "htm/preact";

import { SHARD_ERROR, SYSTEMS, VIEWS } from "./config.js";
import { useStats } from "./data.js";
import { compact } from "./format.js";
import { Link, useRouter } from "./router.js";
import { Commands, systemOf } from "./views/commands.js";
import { Stats } from "./views/stats.js";

/** The system picker. A store path belongs to one system, so this is not a
 *  filter over one table but a choice of which table to read. */
function Systems({ route, navigate }) {
  return html`
    <label class="syspick">
      <span class="syspick-label">store paths for</span>
      <select
        value=${systemOf(route)}
        onChange=${(e) =>
          navigate({
            ...route,
            sys: e.target.value === SYSTEMS[0] ? "" : e.target.value,
          })}
      >
        ${SYSTEMS.map((s) => html`<option key=${s} value=${s}>${s}</option>`)}
      </select>
    </label>
  `;
}

function App() {
  const [route, navigate] = useRouter();
  const statsFile = useStats();
  const stats = statsFile === SHARD_ERROR ? null : statsFile;

  const summary = useMemo(() => {
    if (!stats) return null;
    const s =
      stats.systems.find((x) => x.system === systemOf(route)) ?? stats.systems[0];
    return (
      `${compact(s.names)} executables across ${compact(s.bins)} versions of ` +
      `${compact(s.attrs)} packages · ${s.namedFrom} → ${s.lastDate}`
    );
  }, [stats, route.sys]);

  return html`
    <p class="muted" id="stats">
      ${statsFile === SHARD_ERROR
        ? "Failed to load the index data."
        : (summary ?? "Loading index…")}
    </p>

    <nav>
      ${VIEWS.map(
        (v) => html`
          <${Link}
            key=${v}
            class=${route.view === v ? "active" : ""}
            to=${{ ...route, view: v }}
            navigate=${navigate}
          >
            ${v[0].toUpperCase() + v.slice(1)}
          <//>
        `,
      )}
      <${Systems} route=${route} navigate=${navigate} />
    </nav>

    <section hidden=${route.view !== "commands"}>
      <${Commands} route=${route} navigate=${navigate} />
    </section>

    <section hidden=${route.view !== "stats"}>
      <${Stats} route=${route} navigate=${navigate} stats=${statsFile} />
    </section>
  `;
}

// The container ships a static "Loading index…" placeholder for the moment
// before this module executes; Preact does not clear pre-existing children,
// so drop the placeholder before mounting.
const root = document.getElementById("app");
root.textContent = "";
render(html`<${App} />`, root);

// The site build substitutes the derivation's own $out into STORE_PATH, so
// the footer names the store path serving the page. A local checkout still
// carries the placeholder, and the line stays hidden.
const STORE_PATH = "__STORE_PATH__";
if (!STORE_PATH.startsWith("__")) {
  document.getElementById("store-path").textContent = STORE_PATH;
  document.getElementById("store").hidden = false;
}
