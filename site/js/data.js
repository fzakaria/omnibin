// Everything the site fetches out of its own data files: the search index,
// the per-name shards, and the stats. All of it is cached at module scope, so
// a re-mounted component never refetches.

import { useState, useEffect } from "htm/preact";

import { HTTP_NOT_FOUND, SHARD_ERROR } from "./config.js";

export const fetchJson = (file) =>
  fetch(file).then((r) => {
    if (!r.ok) throw new Error(`${file}: HTTP ${r.status}`);
    return r.json();
  });

/* ---------- per-name shards ----------
 *
 * Every version of every executable is 882,000 rows and tens of megabytes,
 * and a page about `python3` wants one name out of it. The build splits the
 * table by the first two characters of the name, so a command page costs one
 * shard of a few kilobytes. Median shard is 22 KB.
 */
const shardOf = (name) =>
  [...name.slice(0, 2).toLowerCase()]
    .map((c) => (/[a-z0-9]/.test(c) ? c : "_"))
    .join("") || "_";

const shardCache = new Map();

function loadShard(system, name) {
  const path = `bins-${system}/${shardOf(name)}.json`;
  if (!shardCache.has(path)) {
    shardCache.set(
      path,
      fetch(path).then((r) => {
        // A missing shard is not a failure: no file for "zz" means no name
        // starts with those two characters, which is the same answer as a
        // shard that loads and does not hold the name.
        if (r.status === HTTP_NOT_FOUND) return {};
        if (!r.ok) throw new Error(`${path}: HTTP ${r.status}`);
        return r.json();
      }),
    );
  }
  return shardCache.get(path);
}

// A shard row is positional to keep the files small.
const ATTR = 0;
const VERSION = 1;
const DIGEST = 2;
const STORE_NAME = 3;
const NAR_SIZE = 4;
const LAST_SEEN = 5;

export const asVersion = (row) => ({
  attr: row[ATTR],
  version: row[VERSION],
  digest: row[DIGEST],
  storeName: row[STORE_NAME],
  narSize: row[NAR_SIZE],
  lastSeen: row[LAST_SEEN],
});

/** Every version of one executable, or SHARD_ERROR, or null while loading. */
export function useVersions(system, name) {
  const [state, setState] = useState(null);

  useEffect(() => {
    if (!name) {
      setState(null);
      return;
    }

    let live = true;
    setState(null);
    loadShard(system, name)
      .then((shard) => live && setState((shard[name] ?? []).map(asVersion)))
      .catch(() => live && setState(SHARD_ERROR));
    return () => {
      live = false;
    };
  }, [system, name]);

  return state;
}

/* ---------- whole files ---------- */

const fileCache = new Map();

function useFile(path) {
  const [state, setState] = useState(() => fileCache.get(path) ?? null);

  useEffect(() => {
    if (fileCache.has(path)) {
      setState(fileCache.get(path));
      return;
    }

    let live = true;
    fetchJson(path)
      .then((data) => {
        fileCache.set(path, data);
        if (live) setState(data);
      })
      .catch(() => live && setState(SHARD_ERROR));
    return () => {
      live = false;
    };
  }, [path]);

  return state;
}

export const useStats = () => useFile("stats.json");

/** The search index for a system: [name, versionCount] pairs, name-sorted. */
export const useNames = (system) => useFile(`names-${system}.json`);

/* ---------- file listings ----------
 *
 * A store path's complete contents, fetched from cache.nixos.org by the
 * browser. This site publishes none of it. The cache serves every `.ls` with
 * `access-control-allow-origin: *` and the usual Content-Encoding, so a
 * listing is one cross-origin fetch and a JSON parse, and it costs nothing
 * until somebody opens a row.
 *
 * Paths published before roughly 2017 have no listing, and the cache answers
 * 404. That is an answer rather than an error, and it renders as one.
 */
export const CACHE_URL = "https://cache.nixos.org";

const listingCache = new Map();

export function useListing(digest) {
  const [state, setState] = useState(null);

  useEffect(() => {
    if (!digest) {
      setState(null);
      return;
    }

    if (!listingCache.has(digest)) {
      listingCache.set(
        digest,
        fetch(`${CACHE_URL}/${digest}.ls`).then((r) => {
          if (r.status === HTTP_NOT_FOUND) return null;
          if (!r.ok) throw new Error(`${digest}.ls: HTTP ${r.status}`);
          return r.json();
        }),
      );
    }

    let live = true;
    setState(null);
    listingCache
      .get(digest)
      .then((doc) => live && setState(doc ? doc.root : "none"))
      .catch(() => live && setState(SHARD_ERROR));
    return () => {
      live = false;
    };
  }, [digest]);

  return state;
}
