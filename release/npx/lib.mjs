// Pure helpers of the XtraOS installer (ADR 0071), unit-tested.
import crypto from 'node:crypto';
import fs from 'node:fs';

/** The only place releases come from. */
export const REPO = 'om0852/XtraOS-Agent';

/** `SHA256SUMS.txt` lines "<hex>  <file>" → Map(file → hex, lower case). */
export function parseSums(text) {
  const m = new Map();
  for (const line of text.split(/\r?\n/)) {
    const r = /^([0-9a-fA-F]{64})\s+\*?(\S.*)$/.exec(line.trim());
    if (r) m.set(r[2].trim(), r[1].toLowerCase());
  }
  return m;
}

export function sha256File(path) {
  return crypto.createHash('sha256').update(fs.readFileSync(path)).digest('hex');
}

/** Throws unless `path` has the hash `SHA256SUMS.txt` lists for `name`. */
export function verify(path, name, sums) {
  const want = sums.get(name);
  if (!want) throw new Error(`${name} is not listed in SHA256SUMS.txt`);
  const got = sha256File(path);
  if (got !== want) throw new Error(`checksum mismatch for ${name}: expected ${want}, got ${got} — not installing`);
  return got;
}

/** From a GitHub release JSON: the zip asset and SHA256SUMS.txt URLs. */
export function pickAssets(release) {
  const assets = release?.assets ?? [];
  const zip = assets.find((a) => /^xtraos-\d+\.\d+\.\d+-win-x64\.zip$/.test(a.name));
  const sums = assets.find((a) => a.name === 'SHA256SUMS.txt');
  if (!zip || !sums) throw new Error(`release ${release?.tag_name ?? '?'} has no xtraos zip or SHA256SUMS.txt`);
  const ok = (u) => u.startsWith(`https://github.com/${REPO}/releases/download/`);
  if (!ok(zip.browser_download_url) || !ok(sums.browser_download_url)) throw new Error('unexpected download location');
  return { zip: { name: zip.name, url: zip.browser_download_url }, sums: { url: sums.browser_download_url } };
}

/** Command-line options (pure). */
export function parseArgs(argv) {
  const o = { version: null, from: null, dryRun: false, help: false };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === '--dry-run') o.dryRun = true;
    else if (a === '--help' || a === '-h') o.help = true;
    else if (a === '--version' && argv[i + 1]) o.version = argv[++i].replace(/^v/, '');
    else if (a === '--from' && argv[i + 1]) o.from = argv[++i];
    else throw new Error(`unknown option '${a}' (see --help)`);
  }
  if (o.version && !/^\d+\.\d+\.\d+$/.test(o.version)) throw new Error(`bad version '${o.version}'`);
  return o;
}
