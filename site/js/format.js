// Formatting shared by the views. Nothing here fetches or renders.

// A DOM id safe for any executable name. Names carry +, ., @ and worse.
export const domId = (s) => "b-" + s.replace(/[^a-zA-Z0-9_-]/g, "_");

export function fmtBytes(n) {
  if (n === null || n === undefined) return "?";
  if (n < 1000) return `${n} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = n / 1000;
  let i = 0;
  while (value >= 1000 && i < units.length - 1) {
    value /= 1000;
    i += 1;
  }
  return `${value < 10 ? value.toFixed(1) : Math.round(value)} ${units[i]}`;
}

export const compact = (n) =>
  n === null || n === undefined ? "?" : n.toLocaleString();

// Sort version strings the way a person reads them: numeric runs compare as
// numbers, so 3.10 comes after 3.9 rather than before it.
export function compareVersions(a, b) {
  const pa = String(a).split(/(\d+)/);
  const pb = String(b).split(/(\d+)/);
  for (let i = 0; i < Math.max(pa.length, pb.length); i += 1) {
    const x = pa[i] ?? "";
    const y = pb[i] ?? "";
    if (x === y) continue;
    const nx = Number(x);
    const ny = Number(y);
    if (!Number.isNaN(nx) && !Number.isNaN(ny) && x !== "" && y !== "") {
      return nx - ny;
    }
    return x < y ? -1 : 1;
  }
  return 0;
}
