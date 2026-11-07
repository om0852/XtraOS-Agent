// Spike S6 client (stands in for xtraos-mcp): connect to the core's pipe, pair, measure latency.
// Usage: node client.mjs <pipeName> <token|-> <pings>
import net from 'node:net';

const [, , name, tokenArg, pingsArg] = process.argv;
const token = tokenArg === '-' ? undefined : tokenArg;
const pings = Number(pingsArg || 200);
const path = `\\\\.\\pipe\\${name}`;

const sock = net.connect(path);
let buf = '';
const waiters = [];
sock.on('data', (d) => {
  buf += d;
  let i;
  while ((i = buf.indexOf('\n')) >= 0) {
    const line = buf.slice(0, i); buf = buf.slice(i + 1);
    const w = waiters.shift(); if (w) w(JSON.parse(line));
  }
});
sock.on('error', (e) => { console.log(JSON.stringify({ clientError: e.code || e.message })); process.exit(0); });
sock.on('close', () => { for (const w of waiters.splice(0)) w({ closed: true }); });

const call = (id, extra = {}) => new Promise((res) => {
  waiters.push(res);
  sock.write(JSON.stringify({ jsonrpc: '2.0', id, method: 'ping', ...extra }) + '\n');
});

sock.once('connect', async () => {
  const first = await call(1, token ? { token } : {});
  if (first.error || first.closed) {
    console.log(JSON.stringify({ paired: false, reply: first }));
    sock.destroy(); return;
  }
  const times = [];
  for (let i = 0; i < pings; i++) {
    const t0 = process.hrtime.bigint();
    await call(i + 2);
    times.push(Number(process.hrtime.bigint() - t0) / 1e6);
  }
  times.sort((a, b) => a - b);
  const pct = (p) => times[Math.min(times.length - 1, Math.floor(p * times.length))].toFixed(3);
  console.log(JSON.stringify({ paired: true, pings, p50_ms: pct(0.5), p95_ms: pct(0.95), max_ms: times[times.length - 1].toFixed(3) }));
  sock.end();
});
