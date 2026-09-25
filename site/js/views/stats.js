// What the index looks like from above: how much of nixpkgs it reaches, what
// running something out of it costs, and how packages changed shape.

import { html } from "htm/preact";

import { MULTIVERSE_URL, SHARD_ERROR } from "../config.js";
import { compact, fmtBytes } from "../format.js";
import { niceTicks, PLOT, PLOT_H, useWidth } from "../charts.js";
import { Link } from "../router.js";

// Space for a bar and its gap. Both distributions have five buckets, so this
// is generous and the chart never needs a scroll.
const BAR_GAP = 0.25;

/** A labelled horizontal axis of bars, with the same numbers in a table. */
function Bars({ title, sub, rows, unit, counted, format = compact }) {
  const [ref, width] = useWidth();
  const max = Math.max(...rows.map((r) => r.count), 1);
  const ticks = niceTicks(max);
  const top = ticks[ticks.length - 1];
  const inner = width - PLOT.left - PLOT.right;
  const step = inner / rows.length;
  const barW = step * (1 - BAR_GAP);
  const Y = (v) => PLOT.top + (1 - v / top) * PLOT_H;

  return html`
    <div class="chart">
      <h3>${title}</h3>
      <p class="sub">${sub}</p>
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
              ${compact(t)}
            </text>`,
          )}
          ${rows.map(
            (r, i) => html`
              <rect
                class="series-fill"
                key=${r.label}
                x=${PLOT.left + i * step + (step - barW) / 2}
                y=${Y(r.count)}
                width=${barW}
                height=${Math.max(PLOT_H + PLOT.top - Y(r.count), 0)}
              />
              <text
                x=${PLOT.left + i * step + step / 2}
                y=${PLOT_H + PLOT.top + 15}
                text-anchor="middle"
              >
                ${r.label}
              </text>
            `,
          )}
        </svg>
      </figure>
      <details class="table">
        <summary>Table</summary>
        <table>
          <thead>
            <tr>
              <th>${unit}</th>
              <th class="num">${counted}</th>
            </tr>
          </thead>
          <tbody>
            ${rows.map(
              (r) => html`
                <tr key=${r.label}>
                  <td>${r.label}</td>
                  <td class="num">${format(r.count)}</td>
                </tr>
              `,
            )}
          </tbody>
        </table>
      </details>
    </div>
  `;
}

/** One value per year, drawn as a line, with the table under it. */
function Years({ title, sub, rows, value, counted, format = compact }) {
  const [ref, width] = useWidth();
  const pts = rows.map(value);
  const ticks = niceTicks(Math.max(...pts, 1));
  const top = ticks[ticks.length - 1];
  const inner = width - PLOT.left - PLOT.right;
  const X = (n) => PLOT.left + (rows.length < 2 ? inner / 2 : (n / (rows.length - 1)) * inner);
  const Y = (v) => PLOT.top + (1 - v / top) * PLOT_H;
  const line = pts.map((v, n) => `${n ? "L" : "M"}${X(n)},${Y(v)}`).join("");

  return html`
    <div class="chart">
      <h3>${title}</h3>
      <p class="sub">${sub}</p>
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
              ${t}
            </text>`,
          )}
          <path class="area" d=${`${line}L${X(rows.length - 1)},${Y(0)}L${X(0)},${Y(0)}Z`} />
          <path class="series" d=${line} fill="none" />
          ${rows.map(
            (r, n) => html`
              <circle key=${r.year} cx=${X(n)} cy=${Y(pts[n])} r="2.5" />
              <text x=${X(n)} y=${PLOT_H + PLOT.top + 15} text-anchor="middle">
                ${r.year.slice(2)}
              </text>
            `,
          )}
        </svg>
      </figure>
      <details class="table">
        <summary>Table</summary>
        <table>
          <thead>
            <tr>
              <th>year</th>
              <th class="num">${counted}</th>
            </tr>
          </thead>
          <tbody>
            ${rows.map(
              (r, n) => html`
                <tr key=${r.year}>
                  <td>${r.year}</td>
                  <td class="num">${format(pts[n])}</td>
                </tr>
              `,
            )}
          </tbody>
        </table>
      </details>
    </div>
  `;
}

/** The two leaderboards. They were their own tab, but a leaderboard is a
 *  statistic, and a tab that holds one thing is a tab nobody clicks. */
function Leaderboards({ route, navigate, entry, system }) {
  return html`
    <h2>Widest packages</h2>
    <p class="muted">
      One package, many commands. TeX and Kaldi at the top is what a healthy
      index looks like.
    </p>
    <table class="plain">
      <thead>
        <tr>
          <th>package</th>
          <th class="num">executables</th>
        </tr>
      </thead>
      <tbody>
        ${entry.widest.map(
          (row) => html`
            <tr key=${`${row.attr}@${row.version}`}>
              <td>
                <a
                  href=${`${MULTIVERSE_URL}?pkg=${encodeURIComponent(row.attr)}&ver=${encodeURIComponent(row.version)}&sys=${system}`}
                >
                  ${`${row.attr}@${row.version}`}
                </a>
              </td>
              <td class="num">${compact(row.bins)}</td>
            </tr>
          `,
        )}
      </tbody>
    </table>

    <h2>Most versioned commands</h2>
    <p class="muted">
      The commands nixpkgs has shipped the most builds of. Each one links to
      all of them.
    </p>
    <table class="plain">
      <thead>
        <tr>
          <th>command</th>
          <th class="num">versions</th>
        </tr>
      </thead>
      <tbody>
        ${entry.mostVersioned.map(
          (row) => html`
            <tr key=${row.name}>
              <td>
                <${Link}
                  to=${{ ...route, view: "commands", cmd: row.name, q: "" }}
                  navigate=${navigate}
                >
                  ${row.name}
                <//>
              </td>
              <td class="num">${compact(row.versions)}</td>
            </tr>
          `,
        )}
      </tbody>
    </table>
  `;
}

export function Stats({ route, navigate, stats }) {
  if (stats === null) return html`<p class="muted">Loading…</p>`;
  if (stats === SHARD_ERROR)
    return html`<p class="muted">Could not load the stats.</p>`;

  const primary = stats.systems.find((s) => s.system === "x86_64-linux") ?? stats.systems[0];

  // The years before listings were published carry packages and no commands,
  // so they would draw a flat zero and say nothing.
  const named = primary.years.filter((y) => y.bins > 0);

  return html`
    <h2>What is in here</h2>
    <table class="plain">
      <thead>
        <tr>
          <th>system</th>
          <th class="num">executables</th>
          <th class="num">versions</th>
          <th class="num">store paths</th>
          <th class="num">unpacked</th>
        </tr>
      </thead>
      <tbody>
        ${stats.systems.map(
          (s) => html`
            <tr key=${s.system}>
              <td><code>${s.system}</code></td>
              <td class="num">${compact(s.names)}</td>
              <td class="num">${compact(s.bins)}</td>
              <td class="num">${compact(s.paths)}</td>
              <td class="num">${fmtBytes(s.bytes)}</td>
            </tr>
          `,
        )}
      </tbody>
    </table>

    <p class="muted">
      Built from nixpkgs-multiverse <code>${stats.multiverseTag}</code>. Store
      paths reach back to ${primary.firstDate}, but the commands inside them
      can only be named from ${primary.namedFrom}: cache.nixos.org publishes a
      file listing beside every narinfo, and it did not always. Earlier paths
      still fetch and still run, they just cannot be searched by command name.
      ${` ${compact(primary.pkgsNamed)} of ${compact(primary.pkgs)} package versions are named.`}
    </p>

    <${Years}
      title="Executables per package"
      sub="Mean count of commands in a package's bin/, by the year its newest build shipped. It falls, which is nixpkgs splitting packages up rather than fattening them."
      rows=${named}
      value=${(r) => r.binsPerPackage}
      counted="mean commands per package"
      format=${(v) => v.toFixed(2)}
    />

    <${Years}
      title="Distinct commands"
      sub="How many differently named executables were shipped by packages whose newest build is from that year."
      rows=${named}
      value=${(r) => r.names}
      counted="distinct commands"
    />

    <${Bars}
      title="Commands per package"
      sub=${`How many executables one package ships, on ${primary.system}. Most ship exactly one.`}
      rows=${primary.binsPerPackage}
      unit="commands in the package"
      counted="package builds"
    />

    <h2>How big a command is</h2>
    <p class="muted">
      Unpacked size of the newest build of each command, which is what the
      first run of it downloads before any of its dependencies.
    </p>
    <table class="plain">
      <thead>
        <tr>
          <th>unpacked size</th>
          <th class="num">commands</th>
        </tr>
      </thead>
      <tbody>
        ${primary.sizes.map(
          (r) => html`
            <tr key=${r.label}>
              <td>${r.label}</td>
              <td class="num">${compact(r.count)}</td>
            </tr>
          `,
        )}
      </tbody>
    </table>

    <${Leaderboards}
      route=${route}
      navigate=${navigate}
      entry=${primary}
      system=${primary.system}
    />
  `;
}
