import { createReadStream, existsSync, statSync } from 'node:fs';
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

/**
 * Refuses to start against a stale package build.
 *
 * `npm run wasm` rebuilds `crates/wasm/pkg`, but this app imports `pptx-viewer`, which
 * resolves to `packages/viewer/dist` — a *copy* made by the package build. Change some
 * Rust, rebuild the wasm, reload the page, and you are silently looking at the previous
 * build wondering why your fix did nothing. Ask me how I know.
 */
function requireFreshPackage(): Plugin {
  return {
    name: 'require-fresh-package',
    apply: 'serve',
    configResolved() {
      const root = resolve(__dirname, '../..');
      const built = resolve(root, 'crates/wasm/pkg/pptx_bg.wasm');
      const shipped = resolve(root, 'packages/viewer/dist/pptx_bg.wasm');

      if (!existsSync(shipped)) {
        throw new Error(
          '\n\n  packages/viewer/dist is missing.\n' +
            '  Build it first:  npm run build:pkg   (from the repo root)\n',
        );
      }
      if (existsSync(built) && statSync(built).mtimeMs > statSync(shipped).mtimeMs) {
        throw new Error(
          '\n\n  packages/viewer/dist is older than the built WASM, so this app would\n' +
            '  run the PREVIOUS build and any recent Rust change would appear to do nothing.\n\n' +
            '  Refresh it:  npm run build:pkg   (from the repo root)\n',
        );
      }
    },
  };
}

export default defineConfig({
  plugins: [react(), serveFixtures(), requireFreshPackage()],
  server: { port: 5179, strictPort: true, fs: { allow: [resolve(__dirname, '../..')] } },
  optimizeDeps: { exclude: ['pptx-viewer'] },
});
