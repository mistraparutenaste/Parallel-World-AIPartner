import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { extname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const port = Number.parseInt(process.env.PORT ?? '4179', 10);
const root = fileURLToPath(new URL('.', import.meta.url));
const contentTypes = {
  '.css': 'text/css; charset=utf-8',
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
};

createServer(async (request, response) => {
  const pathname = new URL(request.url ?? '/', 'http://127.0.0.1').pathname;
  const relativePath = pathname === '/' ? 'index.html' : pathname.slice(1);

  if (!['index.html', 'styles.css', 'mock.js'].includes(relativePath)) {
    response.writeHead(404).end('Not found');
    return;
  }

  try {
    const file = await readFile(join(root, relativePath));
    response.writeHead(200, {
      'Cache-Control': 'no-store',
      'Content-Type': contentTypes[extname(relativePath)] ?? 'application/octet-stream',
    });
    response.end(file);
  } catch {
    response.writeHead(500).end('Unable to read mock asset');
  }
}).listen(port, '127.0.0.1', () => {
  console.log(`UI motion mock: http://127.0.0.1:${port}/`);
});
