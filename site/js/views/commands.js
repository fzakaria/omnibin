// The view people come for: find a command, see every version of it, and get
// the exact thing to type.

import { html, useMemo, useState, useEffect } from "htm/preact";

import {
  MAX_RESULTS,
  MIN_QUERY,
  MULTIVERSE_URL,
  SEENIX_URL,
  SHARD_ERROR,
  STORE_DIR,
  SYSTEMS,
  TRYNIX_URL,
} from "../config.js";
import { useListing, useNames, usePackageCommands, useVersions } from "../data.js";
import { compareVersions, compact, domId, fmtBytes } from "../format.js";

// How many providing packages to name before giving up and saying "others".
const ATTRS_SHOWN = 4;

// How many directory entries to draw before asking. A store path can hold
// tens of thousands of files, and nobody reads past the first screen.
const ENTRIES_SHOWN = 40;

// How many of a build's other commands to name before saying "and more".
const SIBLINGS_SHOWN = 8;
import { Link, Nav } from "../router.js";
import { Cmd, Row } from "../ui.js";
import { niceTicks, PLOT, PLOT_H, useWidth } from "../charts.js";

/** The system a route is showing, which is the first one until asked. */
export const systemOf = (route) => route.sys || SYSTEMS[0];

function Search({ route, navigate, names }) {
  // Typing replaces the history entry so Back leaves the page rather than
  // walking one keystroke at a time.
  const onInput = (e) =>
    navigate({ ...route, q: e.target.value, cmd: "" }, Nav.REPLACE);

  const hits = useMemo(() => {
    if (!names || names === SHARD_ERROR) return [];
    const q = route.q.trim().toLowerCase();
    if (q.length < MIN_QUERY) return [];

    // Exact first, then prefix, then anywhere. Somebody typing "gcc" wants
    // gcc before aarch64-unknown-linux-gnu-gcc.
    const exact = [];
    const prefix = [];
    const rest = [];
    for (const entry of names) {
      const name = entry[0];
      const at = name.toLowerCase().indexOf(q);
      if (at < 0) continue;
      if (name.toLowerCase() === q) exact.push(entry);
      else if (at === 0) prefix.push(entry);
      else rest.push(entry);
      if (exact.length + prefix.length + rest.length > MAX_RESULTS * 4) break;
    }
    return [...exact, ...prefix, ...rest].slice(0, MAX_RESULTS);
  }, [names, route.q]);

  return html`
    <input
      type="search"
      placeholder="Search 51,000 executables, by the name you would type"
      value=${route.q}
      onInput=${onInput}
      autofocus
    />

    ${route.q.trim().length >= MIN_QUERY &&
    html`
      <div id="status" class="muted">
        ${hits.length === 0
          ? "No executable by that name."
          : `${hits.length.toLocaleString()}${hits.length === MAX_RESULTS ? "+" : ""} matching`}
      </div>
      <div id="results">
        ${hits.map(
          ([name, versions]) => html`
            <${Link}
              class="pkg"
              to=${{ ...route, cmd: name, q: "" }}
              navigate=${navigate}
              key=${name}
            >
              ${name}
              <span class="muted">
                ${` · ${compact(versions)} version${versions === 1 ? "" : "s"}`}
              </span>
            <//>
          `,
        )}
      </div>
    `}
  `;
}

/** Every command one build of a package ships, from this index. */
function PackageCommands({ route, navigate, system }) {
  const commands = usePackageCommands(system, route.pkg, route.ver);

  if (commands === null) return html`<p class="muted">Loading…</p>`;
  if (commands === SHARD_ERROR)
    return html`<p class="muted">Could not load that package.</p>`;
  if (commands.length === 0)
    return html`<p class="muted">
      ${`Nothing recorded for ${route.pkg}@${route.ver} on ${system}.`}
    </p>`;

  return html`
    <h2>${`${route.pkg}@${route.ver}`}</h2>
    <p class="muted">
      ${`${compact(commands.length)} command${commands.length === 1 ? "" : "s"} on `}
      <code>${system}</code>${`. Each one is every version of itself.`}
    </p>
    <div id="results">
      ${commands.map(
        (c) => html`
          <${Link}
            class="pkg"
            key=${c}
            to=${{ ...route, view: "commands", cmd: c, pkg: "", ver: "" }}
            navigate=${navigate}
          >
            ${c}
          <//>
        `,
      )}
    </div>
  `;
}

/** One directory of a listing, with its children expandable underneath. */
function Dir({ node, path, depth }) {
  const [open, setOpen] = useState(depth === 0 ? ["bin"] : []);
  const [shown, setShown] = useState(ENTRIES_SHOWN);

  const entries = Object.entries(node.entries ?? {}).sort(([a], [b]) =>
    a.localeCompare(b),
  );
  const visible = entries.slice(0, shown);

  return html`
    <ul class="tree">
      ${visible.map(([name, child]) => {
        const isDir = child.type === "directory";
        const isOpen = open.includes(name);
        const toggle = () =>
          setOpen(isOpen ? open.filter((n) => n !== name) : [...open, name]);

        return html`
          <li key=${name}>
            ${isDir
              ? html`<button class="twist" onClick=${toggle}>
                  ${isOpen ? "▾" : "▸"} ${name}/
                </button>`
              : html`<span class=${child.executable ? "exe" : ""}>${name}</span>`}
            ${child.type === "symlink" &&
            html`<span class="muted">${` → ${child.target}`}</span>`}
            ${child.type === "regular" &&
            html`<span class="muted">${` ${fmtBytes(child.size)}`}</span>`}
            ${isDir && isOpen &&
            html`<${Dir} node=${child} path=${`${path}/${name}`} depth=${depth + 1} />`}
          </li>
        `;
      })}
      ${entries.length > shown &&
      html`<li>
        <button class="twist" onClick=${() => setShown(shown + ENTRIES_SHOWN * 5)}>
          ${`… ${compact(entries.length - shown)} more`}
        </button>
      </li>`}
    </ul>
  `;
}

/** What is actually inside a store path, fetched from the cache on demand. */
function Listing({ digest }) {
  const root = useListing(digest);

  if (root === null) return html`<p class="muted">Reading the listing…</p>`;
  if (root === SHARD_ERROR)
    return html`<p class="muted">The cache did not answer for this path.</p>`;
  if (root === "none")
    return html`<p class="muted">
      The cache publishes no file listing for this path, which is normal for
      anything built before about 2017. The path still fetches and still runs.
    </p>`;

  return html`
    <p class="muted">
      Fetched from cache.nixos.org as you opened this, rather than served from
      here: the full tree of every path would be gigabytes. The counts above
      come from this index.
    </p>
    <${Dir} node=${root} path="" depth=${0} />
  `;
}

/** One version row, expanded: what to type, what is in it, where to go. */
function VersionDetail({ name, row, system }) {
  const path = `${STORE_DIR}/${row.digest}-${row.storeName}`;
  const siblings = usePackageCommands(system, row.attr, row.version);
  const others = Array.isArray(siblings) ? siblings.filter((c) => c !== name) : [];

  return html`
    <div class="detail">
      <${Cmd} text=${`${name}@${row.version}`} caption="on PATH inside omnibin" />
      <${Cmd}
        text=${`nix run github:fzakaria/omnibin -- ${name}@${row.version} --version`}
        caption="without installing anything"
      />
      <p class="muted">
        <code class="path">${path}</code>
      </p>

      ${row.files !== null &&
      row.files !== undefined &&
      html`<p class="muted">
        ${`${compact(row.files)} files, ${compact(row.dirs)} directories and ${compact(row.links)} symlinks`}
        ${row.topDirs.length > 0 &&
        html`${", in "}${row.topDirs.map(
          (d, i) => html`${i ? ", " : ""}<code>${`${d}/`}</code>`,
        )}`}
      </p>`}

      ${others.length > 0 &&
      html`<p class="muted">
        ${`This build also ships ${compact(others.length)} other command${others.length === 1 ? "" : "s"}: `}
        ${others.slice(0, SIBLINGS_SHOWN).map(
          (c, i) => html`${i ? ", " : ""}<code>${c}</code>`,
        )}${others.length > SIBLINGS_SHOWN ? ", and more" : ""}
      </p>`}

      <p class="muted">
        <a href=${`${TRYNIX_URL}?path=${encodeURIComponent(path)}&boot=1`}>
          boot it in your browser
        </a>
        ${" · "}
        <a href=${`${SEENIX_URL}?path=${encodeURIComponent(path)}`}>
          map its closure
        </a>
        ${" · "}
        <a href=${`${MULTIVERSE_URL}?pkg=${encodeURIComponent(row.attr)}&sys=${system}`}>
          ${`${row.attr} in the index`}
        </a>
      </p>

      <details class="table">
        <summary>Every file</summary>
        <${Listing} digest=${row.digest} />
      </details>
    </div>
  `;
}

function Versions({ route, navigate, name, system }) {
  const rows = useVersions(system, name);
  const [open, setOpen] = useState(null);

  // A new command closes whatever was open under the old one.
  useEffect(() => setOpen(null), [name, system]);

  if (rows === null) return html`<p class="muted">Loading ${name}…</p>`;
  if (rows === SHARD_ERROR)
    return html`<p class="muted">Could not load the versions of ${name}.</p>`;
  if (rows.length === 0)
    return html`<p class="muted">
      Nothing on <code>${system}</code> ships a <code>${name}</code>.
    </p>`;

  const sorted = [...rows].sort(
    (a, b) => compareVersions(a.version, b.version) || a.attr.localeCompare(b.attr),
  );
  const byDate = [...rows]
    .filter((r) => r.lastSeen)
    .sort((a, b) => a.lastSeen.localeCompare(b.lastSeen));
  const newest = byDate[byDate.length - 1] ?? sorted[sorted.length - 1];
  const attrs = new Set(sorted.map((r) => r.attr));

  return html`
    <h2>${name}</h2>
    <p class="muted">
      ${`${compact(sorted.length)} builds ship a `}<code>${name}</code>${` on `}
      <code>${system}</code>${`, from ${compact(attrs.size)} different packages,
      newest `}<code>${`${newest.attr}@${newest.version}`}</code>${`.`}
    </p>
    ${attrs.size > 1 &&
    html`<p class="muted">
      ${`More than one package provides this command: `}
      ${[...attrs].slice(0, ATTRS_SHOWN).map(
        (a, i) => html`${i ? ", " : ""}<code>${a}</code>`,
      )}${attrs.size > ATTRS_SHOWN ? ", and others" : ""}${`. The package column
      says which one a row came from.`}
    </p>`}

    ${byDate.length > 1 && html`<${History} rows=${byDate} />`}

    <div class="head cols-cmd">
      <span></span><span>version</span><span>package</span><span>shipped</span
      ><span class="rowsize">download</span>
    </div>
    ${sorted.map((row) => {
      const id = domId(`${name}-${row.attr}-${row.version}`);
      const isOpen = open === id;
      return html`
        <${Row}
          key=${id}
          cols="cols-cmd"
          id=${id}
          open=${isOpen}
          toggle=${() => setOpen(isOpen ? null : id)}
          label=${`${name}@${row.version}`}
          body=${html`<${VersionDetail} name=${name} row=${row} system=${system} />`}
        >
          <span>${row.version}</span>
          <span class="muted">${row.attr}</span>
          <span class="muted">${row.lastSeen ?? ""}</span>
          <span class="rowsize">${fmtBytes(row.narSize)}</span>
        <//>
      `;
    })}
  `;
}

/** Download size across every build of one command, oldest to newest. */
function History({ rows }) {
  const [ref, width] = useWidth();
  const pts = rows.map((r) => r.narSize || 0);
  const ticks = niceTicks(Math.max(...pts, 1));
  const top = ticks[ticks.length - 1];
  const inner = width - PLOT.left - PLOT.right;
  const X = (n) => PLOT.left + (rows.length < 2 ? inner / 2 : (n / (rows.length - 1)) * inner);
  const Y = (v) => PLOT.top + (1 - v / top) * PLOT_H;
  const line = pts.map((v, n) => `${n ? "L" : "M"}${X(n)},${Y(v)}`).join("");

  return html`
    <div class="chart">
      <h3>How big it has been</h3>
      <figure ref=${ref}>
        <svg height=${PLOT_H + PLOT.top + PLOT.bottom}>
          <g class="grid">
            ${ticks.map(
              (t) => html`<line
                x1=${PLOT.left}
                x2=${width - PLOT.right}
                y1=${Y(t)}
                y2=${Y(t)}
              />`,
            )}
          </g>
          ${ticks.map(
            (t) => html`<text x=${PLOT.left - 6} y=${Y(t) + 4} text-anchor="end">
              ${fmtBytes(t)}
            </text>`,
          )}
          <path
            class="area"
            d=${`${line}L${X(rows.length - 1)},${Y(0)}L${X(0)},${Y(0)}Z`}
          />
          <path class="series" d=${line} fill="none" />
        </svg>
      </figure>
    </div>
  `;
}

export function Commands({ route, navigate }) {
  const system = systemOf(route);
  const names = useNames(system);

  return html`
    ${route.pkg
      ? html`<${PackageCommands} route=${route} navigate=${navigate} system=${system} />`
      : route.cmd
      ? html`<${Versions}
          route=${route}
          navigate=${navigate}
          name=${route.cmd}
          system=${system}
        />`
      : html`<${Search} route=${route} navigate=${navigate} names=${names} />`}
  `;
}
