#!/usr/bin/env node
// npx xtraos-install (ADR 0071): download the XtraOS release zip from GitHub, verify its SHA-256
// against the release's SHA256SUMS.txt, extract it and run install.ps1 (per user, no admin).
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { REPO, parseArgs, parseSums, pickAssets, verify } from './lib.mjs';

const HELP = `npx xtraos-install [--version X.Y.Z] [--from <zip> --sums <SHA256SUMS.txt>] [--dry-run]
  Installs XtraOS Agent for this Windows user from github.com/${REPO} releases.
  --version   a specific release (default: the latest)
  --from      install a local zip instead (its SHA256SUMS.txt must sit next to it)
  --dry-run   download and verify, but do not install`;

async function download(url, to) {
  const r = await fetch(url, { redirect: 'follow', headers: { 'user-agent': 'xtraos-install' } });
  if (!r.ok) throw new Error(`download failed (${r.status}): ${url}`);
  fs.writeFileSync(to, Buffer.from(await r.arrayBuffer()));
}

async function main() {
  if (process.platform !== 'win32') throw new Error('XtraOS Agent runs on Windows only');
  const o = parseArgs(process.argv.slice(2));
  if (o.help) return console.log(HELP);
  const work = fs.mkdtempSync(path.join(os.tmpdir(), 'xtraos-install-'));
  let zipPath, zipName, sumsText;
  if (o.from) {
    zipPath = path.resolve(o.from);
    zipName = path.basename(zipPath);
    const sumsPath = path.join(path.dirname(zipPath), 'SHA256SUMS.txt');
    if (!fs.existsSync(sumsPath)) throw new Error(`no SHA256SUMS.txt next to ${zipPath}`);
    sumsText = fs.readFileSync(sumsPath, 'utf8');
  } else {
    const api = `https://api.github.com/repos/${REPO}/releases/${o.version ? `tags/v${o.version}` : 'latest'}`;
    const r = await fetch(api, { headers: { 'user-agent': 'xtraos-install', accept: 'application/vnd.github+json' } });
    if (!r.ok) throw new Error(`no release found (${r.status}) at ${api}`);
    const a = pickAssets(await r.json());
    zipName = a.zip.name;
    zipPath = path.join(work, zipName);
    console.log(`downloading ${a.zip.url}`);
    await download(a.zip.url, zipPath);
    const sumsPath = path.join(work, 'SHA256SUMS.txt');
    await download(a.sums.url, sumsPath);
    sumsText = fs.readFileSync(sumsPath, 'utf8');
  }
  const hash = verify(zipPath, zipName, parseSums(sumsText));
  console.log(`verified ${zipName} (sha256 ${hash})`);
  if (o.dryRun) return console.log('dry run: not installed');
  const dest = path.join(work, 'x');
  execFileSync('powershell', ['-NoProfile', '-Command', `Expand-Archive -LiteralPath '${zipPath.replace(/'/g, "''")}' -DestinationPath '${dest.replace(/'/g, "''")}' -Force`], { stdio: 'inherit' });
  const dir = fs.readdirSync(dest).map((d) => path.join(dest, d)).find((d) => fs.existsSync(path.join(d, 'install.ps1')));
  if (!dir) throw new Error('install.ps1 not found in the zip');
  execFileSync('powershell', ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', path.join(dir, 'install.ps1')], { stdio: 'inherit' });
}

main().catch((e) => {
  console.error(`xtraos-install: ${e.message}`);
  // Set the code and let Node finish: process.exit() while a fetch connection is still closing
  // trips a libuv assertion on Windows (seen: "UV_HANDLE_CLOSING", exit code 9).
  process.exitCode = 1;
});
