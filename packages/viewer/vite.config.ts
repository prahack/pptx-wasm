import { copyFileSync, createReadStream, existsSync, mkdirSync } from 'node:fs';
import { resolve } from 'node:path';

import react from '@vitejs/plugin-react';
import { defineConfig, type Plugin } from 'vite';

const WASM_PKG = resolve(__dirname, '../../crates/wasm/pkg');
const WASM_FILES = ['pptx.js', 'pptx_bg.wasm', 'pptx.d.ts', 'pptx_bg.wasm.d.ts'];

/**
 * Ships the WASM module as real files next to the bundle instead of inlining it.
 *
 * Vite's library mode inlines every asset unconditionally — `assetsInlineLimit` does not
 * apply — so leaving the generated glue in the graph base64s a 592KB binary into
 * `index.js`. That inflates it by a third, defeats HTTP caching of the largest artefact
 * in the package, and forces the whole thing through the JS parser before the browser can
 * start compiling it.
 *
 * So the glue is marked external and rewritten to a sibling import. `index.js` imports
 * `./pptx.js`, which fetches `./pptx_bg.wasm` relative to itself — which works unchanged
 * whether the consumer bundles the package, serves `dist/` directly, or points
 * `initWasm()` at a CDN copy.
 */
function externalWasm(): Plugin {
  return {
    name: 'pptx-external-wasm',
    apply: 'build',
    enforce: 'pre',
    resolveId(source, importer) {
      if (!importer) return null;
      if (source.endsWith('crates/wasm/pkg/pptx.js') || source.endsWith('/pkg/pptx.js')) {
        return { id: './pptx.js', external: true };
      }
      return null;
    },
    writeBundle(options) {
      const outDir = options.dir ?? resolve(__dirname, 'dist');
      if (!existsSync(WASM_PKG)) {
        this.warn(
          `no WASM build at ${WASM_PKG}. Run \`npm run wasm\` before building the package.`,
        );
        return;
      }
      mkdirSync(outDir, { recursive: true });
      for (const file of WASM_FILES) {
        const from = resolve(WASM_PKG, file);
        if (existsSync(from)) copyFileSync(from, resolve(outDir, file));
      }
    },
  };
}

/**
 * Serves `fixtures/generated/` from the repo root during development.
 *
 * The dev app and the golden harness both load fixtures by URL, but they live outside
 * this package, so Vite will not serve them without being told.
 */
function serveFixtures(): Plugin {
  return {
    name: 'pptx-serve-fixtures',
    apply: 'serve',
    configureServer(server) {
      const root = resolve(__dirname, '../..');
      server.middlewares.use('/fixtures', (req, res, next) => {
        const url = (req.url ?? '').split('?')[0] ?? '';
        // Reject anything that could climb out of the fixtures directory.
        const file = resolve(root, 'fixtures', `.${decodeURIComponent(url)}`);
        if (!file.startsWith(resolve(root, 'fixtures')) || !existsSync(file)) {
          next();
          return;
        }
        res.setHeader(
          'Content-Type',
          file.endsWith('.pptx')
            ? 'application/vnd.openxmlformats-officedocument.presentationml.presentation'
            : 'application/octet-stream',
        );
        createReadStream(file).pipe(res);
      });
    },
  };
}

/**
 * Two modes from one config: `vite` serves the dev app in `src/dev`, `vite build`
 * produces the library. React is external in the library build so a consumer's copy is
 * used — bundling it would give the app two Reacts and break hooks.
 */
export default defineConfig(({ command }) => ({
  plugins: [react(), externalWasm(), serveFixtures()],
  // The .wasm and its glue live outside this package's root, so Vite has to be told it
  // may serve them in dev.
  server: {
    fs: { allow: [resolve(__dirname, '../..')] },
    // The golden suite drives this server headlessly, so the port is fixed. 5173 is
    // Vite's default and therefore frequently occupied by an unrelated project;
    // `strictPort` makes a clash fail loudly instead of silently moving.
    port: 5178,
    strictPort: true,
  },
  // wasm-pack's glue uses top-level await for instantiation.
  esbuild: { target: 'es2022' },
  optimizeDeps: {
    // Pre-bundling relocates the glue, which breaks its relative resolution of the .wasm.
    exclude: ['pptx'],
  },
  build:
    command === 'build'
      ? {
          target: 'es2022',
          lib: {
            entry: {
              index: resolve(__dirname, 'src/index.ts'),
              react: resolve(__dirname, 'src/react.tsx'),
            },
            formats: ['es', 'cjs'],
            fileName: (format, name) => `${name}.${format === 'es' ? 'js' : 'cjs'}`,
          },
          rollupOptions: {
            external: ['react', 'react-dom', 'react/jsx-runtime'],
            output: { globals: { react: 'React', 'react-dom': 'ReactDOM' } },
          },
          sourcemap: true,
          emptyOutDir: true,
        }
      : undefined,
}));
