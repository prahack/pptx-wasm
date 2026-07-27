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
  readdirSync,
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

const ENGINES = [
  { id: 'ours', label: 'pptx-viewer (this project)' },
  { id: 'pptx-preview', label: 'pptx-preview 1.0.7' },
];

// --------------------------------------------------------------------- payload

/** Total gzipped size of the files a package actually ships to a browser. */
function payloadOf(dir, exts = ['.js', '.wasm', '.cjs', '.css']) {
  if (!existsSync(dir)) return null;
  let total = 0;
  const walk = (d) => {
    for (const entry of readdirSync(d, { withFileTypes: true })) {
      const full = join(d, entry.name);
      if (entry.isDirectory()) {
        walk(full);
        continue;
      }
      // Source maps and type declarations are not downloaded at runtime.
      if (entry.name.endsWith('.map') || entry.name.endsWith('.d.ts')) continue;
      if (!exts.some((e) => entry.name.endsWith(e))) continue;
      total += gzipSync(readFileSync(full)).length;
    }
  };
  walk(dir);
  return total;
}

/**
 * What each engine ships.
 *
 * For ours that is the built `dist`. For pptx-preview it is the ES bundle plus the
 * runtime dependencies a bundler would pull in with it — counting only its own 134KB
 * file would understate it by an order of magnitude, since echarts alone dwarfs it.
 */
function measurePayloads() {
  const ours = payloadOf(join(ROOT, 'packages/viewer/dist'));

  const modules = join(APP, 'node_modules');
  const previewSelf = existsSync(join(modules, 'pptx-preview/dist/pptx-preview.es.js'))
    ? gzipSync(readFileSync(join(modules, 'pptx-preview/dist/pptx-preview.es.js'))).length
    : null;
  const deps = ['jszip', 'lodash', 'echarts', 'uuid', 'tslib'];
  const depSizes = {};
  let depTotal = 0;
  for (const d of deps) {
    const size = payloadOf(join(modules, d, 'dist')) ?? payloadOf(join(modules, d));
    if (size) {
      depSizes[d] = size;
      depTotal += size;
    }
  }
  return { ours, previewSelf, previewDeps: depSizes, previewTotal: (previewSelf ?? 0) + depTotal };
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

  report(rows, measurePayloads(), haveOracle);
}

function report(rows, payload, haveOracle) {
  console.log('\n' + '='.repeat(78));
  console.log('PAYLOAD (gzipped, what the browser downloads)');
  console.log('='.repeat(78));
  console.log(`  pptx-viewer (this project)   ${kb(payload.ours)}   wasm + js, no runtime deps`);
  console.log(`  pptx-preview                 ${kb(payload.previewTotal)}   bundle ${kb(payload.previewSelf)} + deps:`);
  for (const [name, size] of Object.entries(payload.previewDeps)) {
    console.log(`      ${name.padEnd(24)} ${kb(size)}`);
  }

  console.log('\n' + '='.repeat(78));
  console.log('PER FIXTURE');
  console.log('='.repeat(78));
  const w = Math.max(8, ...rows.map((r) => r.suite.length));
  const head = haveOracle
    ? `  ${'fixture'.padEnd(w)} ${'engine'.padEnd(14)} ${'cold'.padStart(9)} ${'warm'.padStart(9)} ${'canvas'.padStart(9)} ${'content'.padStart(9)}`
    : `  ${'fixture'.padEnd(w)} ${'engine'.padEnd(14)} ${'cold'.padStart(9)} ${'warm'.padStart(9)}`;
  console.log(head);
  if (haveOracle) {
    console.log(
      `  ${''.padEnd(w)} ${''.padEnd(14)} ${''.padStart(9)} ${''.padStart(9)}` +
        `  % of slide  % of content`,
    );
  }

  for (const r of rows) {
    if (r.error) {
      console.log(`  ${r.suite.padEnd(w)} ${r.engine.padEnd(14)} FAILED: ${r.error}`);
      continue;
    }
    const line = `  ${r.suite.padEnd(w)} ${r.engine.padEnd(14)} ${ms(r.cold).padStart(9)} ${ms(r.warm).padStart(9)}`;
    console.log(
      haveOracle
        ? `${line} ${(r.sizeMismatch ? 'size!' : pct(r.accuracy)).padStart(9)} ${pct(r.ink).padStart(9)}`
        : line,
    );
  }

  // Aggregate, counting only fixtures where both engines produced something.
  const bySuite = new Map();
  for (const r of rows) {
    if (r.error) continue;
    if (!bySuite.has(r.suite)) bySuite.set(r.suite, {});
    bySuite.get(r.suite)[r.engine] = r;
  }
  const both = [...bySuite.values()].filter((s) => s.ours && s['pptx-preview']);

  if (both.length > 0) {
    console.log('\n' + '='.repeat(78));
    console.log(`SUMMARY over ${both.length} fixture(s) both engines rendered`);
    console.log('='.repeat(78));
    const avg = (f) => both.reduce((a, s) => a + (f(s) ?? 0), 0) / both.length;
    console.log(`  cold  ours ${ms(avg((s) => s.ours.cold))}  vs  pptx-preview ${ms(avg((s) => s['pptx-preview'].cold))}`);
    console.log(`  warm  ours ${ms(avg((s) => s.ours.warm))}  vs  pptx-preview ${ms(avg((s) => s['pptx-preview'].warm))}`);
    if (haveOracle) {
      const oursAcc = both.filter((s) => s.ours.accuracy != null);
      const prevAcc = both.filter((s) => s['pptx-preview'].accuracy != null);
      if (oursAcc.length && prevAcc.length) {
        const mean = (rows, key, engine) =>
          rows.reduce((a, s) => a + s[engine][key], 0) / rows.length;
        console.log(
          `  vs oracle, % of slide    ours ${pct(mean(oursAcc, 'accuracy', 'ours'))}` +
            `   vs  pptx-preview ${pct(mean(prevAcc, 'accuracy', 'pptx-preview'))}`,
        );
        console.log(
          `  vs oracle, % of content  ours ${pct(mean(oursAcc, 'ink', 'ours'))}` +
            `   vs  pptx-preview ${pct(mean(prevAcc, 'ink', 'pptx-preview'))}`,
        );
        console.log(
          '\n  Lower is closer to LibreOffice. Prefer the content figure: a slide is mostly\n' +
            '  white, so a badly misplaced text block barely moves the canvas figure.',
        );
      }
    }
    const failedPreview = rows.filter((r) => r.engine === 'pptx-preview' && r.error).length;
    if (failedPreview > 0) {
      console.log(`\n  Note: pptx-preview failed on ${failedPreview} fixture(s); those are excluded above.`);
    }
  }

  console.log(`\n  Renders and diffs: ${OUT}`);
}

await main();
