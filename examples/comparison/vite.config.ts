import { createReadStream, existsSync } from 'node:fs';
import { resolve } from 'node:path';

import react from '@vitejs/plugin-react';
import { defineConfig, type Plugin } from 'vite';

/** Serves the shared fixtures so both engines are measured on identical input. */
function serveFixtures(): Plugin {
  return {
    name: 'serve-fixtures',
    apply: 'serve',
    configureServer(server) {
      const root = resolve(__dirname, '../..');
      server.middlewares.use('/fixtures', (req, res, next) => {
        const url = (req.url ?? '').split('?')[0] ?? '';
        const file = resolve(root, 'fixtures', `.${decodeURIComponent(url)}`);
        if (!file.startsWith(resolve(root, 'fixtures')) || !existsSync(file)) return next();
        res.setHeader('Content-Type', 'application/octet-stream');
        createReadStream(file).pipe(res);
      });
    },
  };
}

export default defineConfig({
  plugins: [react(), serveFixtures()],
  server: { port: 5179, strictPort: true, fs: { allow: [resolve(__dirname, '../..')] } },
  optimizeDeps: { exclude: ['pptx-viewer'] },
});
