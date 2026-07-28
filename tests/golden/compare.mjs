/**
 * Comparative benchmark against other browser-side pptx renderers.
 *
 * The project's own bench answers "is this fast enough?". This answers "is this better
 * than what already exists?" — which is a different question and needs a different setup.
 *
 * Measures three things per engine, per fixture:
 *
 *  - **Cold** open+render, in a fresh page. Includes whatever the engine must do once:
 *    instantiating a WASM module, or parsing and JIT-warming a JS bundle. This is what a
 *    first-time visitor waits for.
 *  - **Warm** open+render, opening further decks in the same page. This is what someone
 *    browsing several files experiences.
 *  - **Accuracy**, by screenshotting *every* slide and diffing each against the same
 *    LibreOffice render the golden suite uses. Both engines are judged by an
 *    implementation neither of them wrote, which is the only way this number means
 *    anything coming from the author of one of them.
 *
 *    Every slide, not just the first: slide 1 is usually the title slide, the simplest
 *    in the deck, and scoring only that understates how far two renderers diverge. On
 *    the m4 template, for instance, the engines look near-identical on slide 1 and very
 *    different on slide 2.
 *
 * Payload is measured from what each package actually ships.
 *
 * Two of the engines do not install cleanly. pptxviewjs imports `chart.js/auto` and
 * pptx-vanilla-viewer imports `three`, and neither declares the dependency, so a bundler
 * fails to resolve them until you install the missing package yourself. Both are in this
 * example's package.json for that reason, and both count toward their engine's payload.
 *
 *   npm run compare
 *   npm run compare -- --runs=5 --suite=m2
 *   npm run compare -- --file=~/decks/quarterly.pptx     # score your own deck
 */

import { execFile } from 'node:child_process';
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import { homedir } from 'node:os';
import { gzipSync } from 'node:zlib';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';

import pixelmatch from 'pixelmatch';
import { PNG } from 'pngjs';

import { findTools, renderFixture } from './oracle.mjs';
import { ROOT } from './server.mjs';

const exec = promisify(execFile);
const HERE = dirname(fileURLToPath(import.meta.url));
const OUT = join(HERE, 'out', 'compare');
const APP = join(ROOT, 'examples/comparison');
const PORT = 5179;
const BASE = `http://localhost:${PORT}`;
const MARKER = '<title>pptx renderer comparison</title>';

const args = process.argv.slice(2);
const only = args.find((a) => a.startsWith('--suite='))?.slice('--suite='.length);
const customFile = args.find((a) => a.startsWith('--file='))?.slice('--file='.length);
const runs = Number.parseInt(args.find((a) => a.startsWith('--runs='))?.slice('--runs='.length) ?? '3', 10);

const config = JSON.parse(readFileSync(join(HERE, 'suites.json'), 'utf8'));

/**
 * The decks to compare.
 *
 * `--file` swaps the bundled fixtures for one of your own. It is copied next to them so
 * the dev server and the LibreOffice oracle can both reach it by the same path, and it
 * gets the default tolerance since suites.json has no entry for it.
 */
function resolveSuites() {
  if (!customFile) return config.suites.filter((s) => !only || s.id === only);

  const source = customFile.startsWith('~') ? join(homedir(), customFile.slice(1)) : customFile;
  if (!existsSync(source)) {
    console.error(`No such file: ${source}`);
    process.exit(2);
  }
  const name = source.split('/').pop() ?? 'custom.pptx';
  const dest = join(ROOT, 'fixtures/generated', name);
  mkdirSync(join(ROOT, 'fixtures/generated'), { recursive: true });
  if (dest !== source) copyFileSync(source, dest);
  return [{ id: 'custom', fixture: name, custom: true }];
}

const suites = resolveSuites();

/**
 * The engines under test, with what each one ships.
 *
 * `self` is the ES bundle a bundler would pull in; `deps` are the runtime packages it
 * drags with it. Counting only the entry file understates several of these by an order of
 * magnitude — pptx-preview's own bundle is 134KB and the echarts it imports for charts is
 * far larger. A user downloads both, so both are counted.
 *
 * Ours is measured from the built `dist` instead, because the WASM is the payload and it
 * has no runtime dependencies to add.
 */
const ENGINES = [
  {
    id: 'ours',
    label: 'pptx-viewer (this project)',
    entry: 'pptx-viewer',
    wasm: 'packages/viewer/dist/pptx_bg.wasm',
  },
  { id: 'pptx-preview', label: 'pptx-preview 1.0.7', entry: 'pptx-preview' },
  { id: 'pptxviewjs', label: 'pptxviewjs 1.1.9', entry: 'pptxviewjs' },
  { id: 'aiden0z', label: '@aiden0z/pptx-renderer 1.2.4', entry: '@aiden0z/pptx-renderer' },
  { id: 'jvmr', label: '@jvmr/pptx-to-html 1.1.1', entry: '@jvmr/pptx-to-html' },
  { id: 'glimpse', label: 'pptx-glimpse 5.0.0', entry: 'pptx-glimpse' },
  { id: 'vanilla', label: 'pptx-vanilla-viewer 1.6.2', entry: 'pptx-vanilla-viewer' },
];

// --------------------------------------------------------------------- payload

/**
 * What each engine actually makes a browser download, measured by bundling it.
 *
 * The previous version added up the `dist` directories of an engine's declared
 * dependencies. That is easy and wrong in both directions: it counts code the bundler
 * would tree-shake, it counts optional features nobody imported, and — worst — it counts
 * lazily-imported chunks as though they were part of the first load. pptx-vanilla-viewer
 * pulls `three` only inside `await import('./smartart-3d-*.js')`, so 5.5MB of its
 * apparent payload never reaches a browser unless the deck has 3D SmartArt.
 *
 * So each engine is bundled for real with esbuild, and its metafile is walked from the
 * entry through *static* imports only. That set is `initial` — what a first paint costs.
 * Everything else esbuild emitted is reachable solely through a dynamic import and is
 * reported as `deferred`. Sizes are gzipped, since that is what crosses the network.
 */
async function measurePayloads() {
  // Inside the example app: esbuild resolves bare specifiers relative to the importing
  // file, so an entry written anywhere else cannot see its node_modules.
  const tmp = join(APP, '.payload-build');
  rmSync(tmp, { recursive: true, force: true });
  mkdirSync(tmp, { recursive: true });

  let esbuild;
  try {
    esbuild = await import(join(APP, 'node_modules/esbuild/lib/main.js'));
  } catch {
    return null;
  }

  const out = {};
  for (const e of ENGINES) {
    const entry = join(tmp, `${e.id}.js`);
    // Reference the export the adapter actually uses, so nothing is tree-shaken that the
    // real render path needs.
    writeFileSync(entry, `import * as m from ${JSON.stringify(e.entry)};\nglobalThis.__keep = m;\n`);
    try {
      const result = await esbuild.build({
        entryPoints: [entry],
        bundle: true,
        splitting: true,
        format: 'esm',
        outdir: join(tmp, e.id),
        minify: true,
        metafile: true,
        write: true,
        absWorkingDir: APP,
        platform: 'browser',
        conditions: ['browser', 'import', 'default'],
        logLevel: 'silent',
        loader: { '.wasm': 'file', '.png': 'file', '.svg': 'file', '.ttf': 'file' },
      });

      const outputs = result.metafile.outputs;
      const entryFile = Object.keys(outputs).find((f) => outputs[f].entryPoint);
      // Walk static imports transitively; anything unreached is dynamic-only.
      const initial = new Set();
      const walk = (f) => {
        if (!f || initial.has(f) || !outputs[f]) return;
        initial.add(f);
        for (const imp of outputs[f].imports ?? []) {
          if (imp.kind === 'import-statement') walk(imp.path);
        }
      };
      walk(entryFile);

      const gz = (f) => gzipSync(readFileSync(join(APP, f))).length;
      let initialBytes = 0;
      let deferredBytes = 0;
      for (const f of Object.keys(outputs)) {
        const size = existsSync(join(APP, f)) ? gz(f) : 0;
        if (initial.has(f)) initialBytes += size;
        else deferredBytes += size;
      }
      // Our own payload is JS plus the .wasm the module fetches on first open, which is
      // not a JS import and so is not in the graph. A user downloads it every time.
      if (e.wasm) {
        const w = join(ROOT, e.wasm);
        if (existsSync(w)) initialBytes += gzipSync(readFileSync(w)).length;
      }
      out[e.id] = {
        initial: initialBytes,
        deferred: deferredBytes,
        total: initialBytes + deferredBytes,
      };
    } catch (err) {
      out[e.id] = { initial: null, deferred: null, error: String(err.message ?? err).split('\n')[0] };
    }
  }
  return out;
}

// --------------------------------------------------------------------- server

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function isUp() {
  try {
    const res = await fetch(BASE, { signal: AbortSignal.timeout(1500) });
    return res.ok && (await res.text()).includes(MARKER);
  } catch {
    return false;
  }
}

async function ensureComparisonServer() {
  if (await isUp()) return { stop: () => {} };
  if (!existsSync(join(APP, 'node_modules'))) {
    console.error(`Dependencies missing. Run:  cd examples/comparison && npm install`);
    process.exit(2);
  }
  const { spawn } = await import('node:child_process');
  const child = spawn('npx', ['vite', '--port', String(PORT), '--strictPort'], {
    cwd: APP,
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  let stderr = '';
  child.stderr.on('data', (d) => (stderr += String(d)));
  const deadline = Date.now() + 90_000;
  while (Date.now() < deadline) {
    if (await isUp()) return { stop: () => child.kill() };
    if (child.exitCode !== null) throw new Error(`comparison server exited:\n${stderr}`);
    await sleep(250);
  }
  child.kill();
  throw new Error(`comparison server did not start on ${BASE}:\n${stderr}`);
}

// --------------------------------------------------------------------- measure

async function measureEngine(browser, engine, fixture, width, height, slideCount) {
  const page = await browser.newPage({ viewport: { width, height }, deviceScaleFactor: 1 });
  const errors = [];
  page.on('pageerror', (e) => errors.push(String(e).split('\n')[0]));

  try {
    const url = `${BASE}/?headless=1&engine=${engine}&fixture=${encodeURIComponent(fixture)}&slide=0&w=${width}&h=${height}`;
    await page.goto(url, { waitUntil: 'load', timeout: 90_000 });
    await page.waitForFunction(() => window.__cmpReady === true || window.__cmpError, null, {
      timeout: 90_000,
    });

    const failure = await page.evaluate(() => window.__cmpError);
    if (failure) return { error: failure, errors };

    const cold = await page.evaluate(() => window.__cmpTiming);

    // Warm: the module is instantiated and the bundle is hot, so this is the cost of
    // opening the next deck rather than the first.
    const warm = [];
    for (let i = 0; i < runs; i++) {
      const t = await page.evaluate(() => window.__cmpRun?.());
      if (t) warm.push(t.totalMs);
    }

    // One screenshot per slide. The engines may disagree about the slide count; render
    // what this one believes it has, and let the caller notice the disagreement.
    const shots = [];
    const count = Math.min(cold?.slideCount ?? 1, slideCount);
    for (let i = 0; i < count; i++) {
      await page.evaluate((n) => window.__cmpRun?.(n), i);
      shots.push(await page.locator('[data-engine]').screenshot({ type: 'png' }));
    }

    return { cold, warm, shots, slideCount: cold?.slideCount ?? 1, errors };
  } catch (e) {
    return { error: e.message.split('\n')[0], errors };
  } finally {
    await page.close();
  }
}

/** True when a pixel carries content rather than being background. */
function isInked(data, i) {
  return data[i + 3] > 8 && (data[i] < 245 || data[i + 1] < 245 || data[i + 2] < 245);
}

/**
 * How far a render is from the oracle, by two measures.
 *
 * `ratio` is the fraction of the whole canvas that differs — the conventional number, and
 * the one the golden suite's tolerances are written against.
 *
 * `inkRatio` divides the same difference by the number of pixels that carry content in
 * *either* image. That second number exists because the first one lies about text. A
 * slide is mostly white and glyphs are thin strokes, so a body-text block rendered at the
 * wrong size in the wrong place — obviously broken to a human — moves the canvas-relative
 * figure by about one percent and reads as "nearly perfect". Measured against the content
 * rather than the canvas, the same error reads as the large error it is.
 */
function accuracyVsOracle(shot, goldenPath, threshold) {
  if (!existsSync(goldenPath)) return null;
  const actual = PNG.sync.read(shot);
  const expected = PNG.sync.read(readFileSync(goldenPath));
  if (actual.width !== expected.width || actual.height !== expected.height) {
    return { sizeMismatch: `${actual.width}x${actual.height} vs ${expected.width}x${expected.height}` };
  }
  const diff = new PNG({ width: actual.width, height: actual.height });
  const differing = pixelmatch(actual.data, expected.data, diff.data, actual.width, actual.height, {
    threshold,
    includeAA: false,
  });

  let inked = 0;
  for (let i = 0; i < actual.data.length; i += 4) {
    if (isInked(actual.data, i) || isInked(expected.data, i)) inked++;
  }

  const total = actual.width * actual.height;
  return {
    ratio: differing / total,
    // A blank slide that matches has no content to be wrong about.
    inkRatio: inked > 0 ? Math.min(1, differing / inked) : 0,
    diff,
  };
}

const median = (xs) => {
  if (xs.length === 0) return 0;
  const s = [...xs].sort((a, b) => a - b);
  return s[Math.floor(s.length / 2)] ?? 0;
};

const kb = (bytes) => (bytes == null ? '—' : `${(bytes / 1024).toFixed(0)} KB`);
const ms = (v) => (v == null ? '—' : `${v.toFixed(1)} ms`);
const pct = (v) => (v == null ? '—' : `${(v * 100).toFixed(2)}%`);

// --------------------------------------------------------------------- main

async function main() {
  // Regenerating fixtures would be wasted work — and would not produce a custom deck.
  const python = join(ROOT, '.venv/bin/python');
  if (!customFile && existsSync(python)) {
    await exec(python, [join(ROOT, 'fixtures/gen.py')], { cwd: ROOT, timeout: 300_000 });
  }
  if (customFile) {
    console.log(`\nComparing your deck: ${suites[0].fixture}\n`);
  }
  if (!existsSync(join(ROOT, 'packages/viewer/dist/pptx_bg.wasm'))) {
    console.error('Build the package first:  npm run wasm && npm run build --workspace packages/viewer');
    process.exit(2);
  }

  mkdirSync(OUT, { recursive: true });

  const tools = await findTools();
  const haveOracle = tools.missing.length === 0;
  if (!haveOracle) {
    console.log('\n⚠️  No LibreOffice — timings only, no accuracy column.\n');
  }

  const playwright = await import('playwright');
  const browser = await playwright.chromium.launch();
  const server = await ensureComparisonServer();

  const rows = [];
  try {
    for (const suite of suites) {
      const width = suite.width ?? config.defaults.width;
      const height = suite.height ?? config.defaults.height;
      const threshold = suite.threshold ?? config.defaults.threshold;

      let goldens = [];
      if (haveOracle) {
        try {
          goldens = await renderFixture(suite.fixture, { width, height, tools });
        } catch {
          goldens = [];
        }
      }

      for (const engine of ENGINES) {
        process.stdout.write(`  ${suite.id} / ${engine.id} … `);
        const result = await measureEngine(
          browser,
          engine.id,
          suite.fixture,
          width,
          height,
          goldens.length || 1,
        );
        if (result.error) {
          console.log(`failed: ${result.error}`);
          rows.push({ suite: suite.id, engine: engine.id, error: result.error });
          continue;
        }

        const perSlide = [];
        for (let i = 0; i < result.shots.length; i++) {
          writeFileSync(join(OUT, `${suite.id}-${engine.id}-s${i + 1}.png`), result.shots[i]);
          const goldenPath = goldens[i];
          if (!goldenPath) continue;
          const cmp = accuracyVsOracle(result.shots[i], goldenPath, threshold);
          if (cmp?.diff) {
            writeFileSync(
              join(OUT, `${suite.id}-${engine.id}-s${i + 1}-diff.png`),
              PNG.sync.write(cmp.diff),
            );
          }
          if (cmp?.ratio != null) perSlide.push({ ratio: cmp.ratio, ink: cmp.inkRatio });
        }

        // Mean across slides *and* the worst, because an engine that nails the title
        // slide and mangles the content slide averages out to looking fine.
        const avg = (f) => (perSlide.length ? perSlide.reduce((a, b) => a + f(b), 0) / perSlide.length : null);
        const mean = avg((s) => s.ratio);
        const meanInk = avg((s) => s.ink);
        const worstInk = perSlide.length ? Math.max(...perSlide.map((s) => s.ink)) : null;

        rows.push({
          suite: suite.id,
          engine: engine.id,
          cold: result.cold?.totalMs,
          warm: median(result.warm),
          accuracy: mean,
          ink: meanInk,
          worstInk,
          slides: perSlide.length,
          slideCount: result.slideCount,
        });
        console.log(
          `cold ${ms(result.cold?.totalMs)}  warm ${ms(median(result.warm))}` +
            (mean != null
              ? `  canvas ${pct(mean)}  content ${pct(meanInk)} (worst ${pct(worstInk)}) over ${perSlide.length} slide(s)`
              : ''),
        );
      }
    }
  } finally {
    await browser.close();
    server.stop();
  }

  report(rows, await measurePayloads(), haveOracle);
}

function report(rows, payload, haveOracle) {
  const label = (id) => ENGINES.find((e) => e.id === id)?.label ?? id;
  const EW = Math.max(...ENGINES.map((e) => e.id.length)) + 1;

  console.log('\n' + '='.repeat(78));
  console.log('PAYLOAD (gzipped, measured by bundling each engine with esbuild)');
  console.log('='.repeat(78));
  if (!payload) {
    console.log('  esbuild unavailable — payload not measured.');
  } else {
    console.log(
      `  ${'engine'.padEnd(30)} ${'initial'.padStart(10)} ${'deferred'.padStart(10)} ${'total'.padStart(10)}`,
    );
    for (const e of ENGINES) {
      const pay = payload[e.id];
      if (!pay) continue;
      if (pay.error) {
        console.log(`  ${label(e.id).padEnd(30)} could not bundle: ${pay.error}`);
        continue;
      }
      console.log(
        `  ${label(e.id).padEnd(30)} ${kb(pay.initial).padStart(10)} ` +
          `${kb(pay.deferred).padStart(10)} ${kb(pay.total).padStart(10)}`,
      );
    }
    console.log(
      '\n  initial  = entry plus everything reachable through static imports, including the\n' +
        '             .wasm this project fetches on open.\n' +
        '  deferred = chunks reachable only through a dynamic import.\n' +
        '  total    = the whole library.\n\n' +
        '  Read initial and total as a lower and an upper bound on what rendering one slide\n' +
        '  costs, not as "required" and "optional". An engine that code-splits its own\n' +
        '  renderer — pptx-vanilla-viewer ships 53KB initial and 1642KB deferred — fetches\n' +
        '  much of that deferred half before it can draw anything, which is visible in its\n' +
        '  cold time. The summary below uses total, so no engine is credited for deferring\n' +
        '  work it still has to do.',
    );
  }

  console.log('\n' + '='.repeat(78));
  console.log('PER FIXTURE');
  console.log('='.repeat(78));
  const w = Math.max(8, ...rows.map((r) => r.suite.length));
  console.log(
    `  ${'fixture'.padEnd(w)} ${'engine'.padEnd(EW)} ${'cold'.padStart(9)} ${'warm'.padStart(9)}` +
      (haveOracle ? ` ${'canvas'.padStart(9)} ${'content'.padStart(9)}` : ''),
  );

  for (const r of rows) {
    if (r.error) {
      console.log(`  ${r.suite.padEnd(w)} ${r.engine.padEnd(EW)} FAILED: ${r.error}`);
      continue;
    }
    const line = `  ${r.suite.padEnd(w)} ${r.engine.padEnd(EW)} ${ms(r.cold).padStart(9)} ${ms(r.warm).padStart(9)}`;
    console.log(
      haveOracle
        ? `${line} ${(r.sizeMismatch ? 'size!' : pct(r.accuracy)).padStart(9)} ${pct(r.ink).padStart(9)}`
        : line,
    );
  }

  // Aggregate only over fixtures *every* engine rendered, so no engine is flattered by
  // being scored on an easier subset than its competitors.
  const bySuite = new Map();
  for (const r of rows) {
    if (r.error) continue;
    if (!bySuite.has(r.suite)) bySuite.set(r.suite, {});
    bySuite.get(r.suite)[r.engine] = r;
  }
  const ids = ENGINES.map((e) => e.id);
  const common = [...bySuite.values()].filter((s) => ids.every((id) => s[id]));

  console.log('\n' + '='.repeat(78));
  console.log(`SUMMARY over ${common.length} fixture(s) every engine rendered`);
  console.log('='.repeat(78));
  if (common.length === 0) {
    console.log('  No fixture was rendered by every engine, so there is nothing to compare\n' +
      '  on equal terms. The per-fixture table above still stands.');
  } else {
    const mean = (id, key) => common.reduce((a, s) => a + (s[id][key] ?? 0), 0) / common.length;
    const cols = haveOracle
      ? [['cold', 'cold', ms], ['warm', 'warm', ms], ['canvas', 'accuracy', pct], ['content', 'ink', pct]]
      : [['cold', 'cold', ms], ['warm', 'warm', ms]];
    console.log(
      `  ${'engine'.padEnd(30)} ${'payload'.padStart(10)} ` +
        cols.map(([h]) => h.padStart(9)).join(' '),
    );
    for (const id of ids) {
      console.log(
        `  ${label(id).padEnd(30)} ${kb(payload?.[id]?.total).padStart(10)} ` +
          cols.map(([, k, f]) => f(mean(id, k)).padStart(9)).join(' '),
      );
    }
    if (haveOracle) {
      console.log(
        '\n  Lower is closer to LibreOffice. Prefer the content figure: a slide is mostly\n' +
          '  white, so a badly misplaced text block barely moves the canvas figure.\n' +
          '  LibreOffice is an imperfect judge (see CLAUDE.md) — but it is the same\n' +
          '  imperfect judge for every engine, and it is not one that we wrote.',
      );
    }
  }

  // Failures are part of the result: an engine that cannot open a deck has not "tied".
  for (const id of ids) {
    const failed = rows.filter((r) => r.engine === id && r.error);
    if (failed.length > 0) {
      console.log(
        `\n  ${label(id)} failed on ${failed.length} fixture(s): ` +
          failed.map((f) => f.suite).join(', '),
      );
    }
  }

  console.log(`\n  Renders and diffs: ${OUT}`);
}

await main();
