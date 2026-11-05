// Serves build/web and forwards POST /api/kiosk, POST /api/auth/{login,refresh,logout} and GET /health/ready to a running XCHANGE server,
// so the kiosk and its API share one origin the way they do behind nginx.
//   node tool/dev_server.mjs [port=8094] [api=http://127.0.0.1:8093]
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const port = Number(process.argv[2] || 8094);
const api = new URL(process.argv[3] || 'http://127.0.0.1:8093');
const root = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', 'build', 'web');
const types = { '.html': 'text/html; charset=utf-8', '.js': 'text/javascript', '.mjs': 'text/javascript', '.json': 'application/json', '.wasm': 'application/wasm', '.png': 'image/png', '.ico': 'image/x-icon', '.otf': 'font/otf', '.ttf': 'font/ttf', '.bin': 'application/octet-stream', '.symbols': 'text/plain' };

http.createServer((req, res) => {
  const url = new URL(req.url, 'http://x');
  if ((req.method === 'POST' && (url.pathname === '/api/kiosk' || url.pathname === '/api/auth/login' || url.pathname === '/api/auth/refresh' || url.pathname === '/api/auth/logout')) || (req.method === 'GET' && url.pathname === '/health/ready')) {
    // Host is passed through unchanged: the server compares Origin with it.
    const upstream = http.request({ host: api.hostname, port: api.port, path: url.pathname, method: req.method, headers: req.headers }, (r) => {
      res.writeHead(r.statusCode, r.headers);
      r.pipe(res);
    });
    upstream.on('error', () => { res.writeHead(502); res.end('{"error":""}'); });
    req.pipe(upstream);
    return;
  }
  let file = path.join(root, decodeURIComponent(url.pathname));
  if (!file.startsWith(root)) { res.writeHead(403); return res.end(); }
  if (!fs.existsSync(file) || fs.statSync(file).isDirectory()) file = path.join(root, 'index.html');
  res.writeHead(200, { 'Content-Type': types[path.extname(file)] || 'application/octet-stream', 'Cache-Control': 'no-cache' });
  fs.createReadStream(file).pipe(res);
}).listen(port, '127.0.0.1', () => console.log(`kiosk dev server on http://localhost:${port} → ${api.origin}`));
