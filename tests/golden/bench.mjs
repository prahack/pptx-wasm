/**
 * The performance bench.
 *
 * Reports the numbers M6's definition of done is written against: how long the first
 * slide takes, how long a navigation takes, and how long a zoom takes. The last one is
 * the interesting one — a zoom must not re-run layout, so it should be an order of
 * magnitude cheaper than a navigation. If it is not, the display list has stopped being
 * resolution-independent and something upstream is re-laying-out per frame.
 *
 *   npm run bench
 *   npm run bench -- --fixture=m6-large.pptx --runs=5
 */

import { execFile } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { promisify } from 'node:util';

import { BASE, ROOT, ensureServer } from './server.mjs';

const exec = promisify(execFile);

const args = process.argv.slice(2);
const fixture = args.find((a) => a.startsWith('--fixture='))?.slice('--fixture='.length)
  ?? 'm6-large.pptx';
const runs = Number.parseInt(
  args.find((a) => a.startsWith('--runs='))?.slice('--runs='.length) ?? '3',
  10,
);

/**
 * Targets, in milliseconds. Chosen against the plan's "first slide renders fast,
 * navigation and zoom hold ~60fps": 16.7ms is one frame at 60fps, so a navigation that
 * fits in two frames feels instant, and a zoom has to fit in one.
 */
const TARGETS = {
  open: 400,
  firstSlide: 250,
  navigate: 33,
  zoom: 16.7,
};

async function ensureFixture() {
  const path = join(ROOT, 'fixtures/generated', fixture);
  if (existsSync(path)) return;
  const python = join(ROOT, '.venv/bin/python');
  if (!existsSync(python)) {
    console.error('No .venv found; cannot generate fixtures.');
    process.exit(2);
  }
  // The bench decks are opt-in in the generator, so ask for them by name.
  const suite = fixture.startsWith('bench-') ? ['bench'] : [];
  await exec(python, [join(ROOT, 'fixtures/gen.py'), ...suite], { cwd: ROOT, timeout: 600_000 });
}

function stats(samples) {
  const sorted = [...samples].sort((a, b) => a - b);
  const at = (q) => sorted[Math.min(sorted.length - 1, Math.floor(q * sorted.length))] ?? 0;
  return {
    median: at(0.5),
    p95: at(0.95),
    min: sorted[0] ?? 0,
    max: sorted[sorted.length - 1] ?? 0,
  };
}

/**
 * The measurement itself, run inside the page.
 *
 * `performance.now()` around the awaited render is the honest figure: it includes the
 * wasm call, the canvas work, and anything the browser does synchronously as a result.
 */
async function measure(page, fixtureName) {
  return page.evaluate(async (name) => {
    const api = window.__pptx;
    if (!api) throw new Error('the dev harness did not expose its API layer');

    const canvas = document.createElement('canvas');
    canvas.width = 1280;
    canvas.height = 720;
    document.body.appendChild(canvas);

    const t0 = performance.now();
    const deck = await api.Presentation.open(`/fixtures/generated/${name}`);
    const openMs = performance.now() - t0;

    const opts = { width: 1280, height: 720, dpr: 1 };

    const t1 = performance.now();
    await deck.render(0, canvas, opts);
    const firstSlideMs = performance.now() - t1;

    // Navigation: layout is not yet cached for these, so this is the real cost of moving
    // to an unvisited slide.
    const navigate = [];
    const count = Math.min(deck.slideCount, 30);
    for (let i = 1; i < count; i++) {
      const t = performance.now();
      await deck.render(i, canvas, opts);
      navigate.push(performance.now() - t);
    }

    // Re-rendering an already-laid-out slide, which is what a resize or a zoom does.
    const zoom = [];
    for (let i = 0; i < 30; i++) {
      const t = performance.now();
      await deck.render(0, canvas, { ...opts, zoom: 1 + (i % 10) * 0.1 });
      zoom.push(performance.now() - t);
    }

    // Revisiting a slide whose layout is cached.
    const revisit = [];
    for (let i = 1; i < count; i++) {
      const t = performance.now();
      await deck.render(i, canvas, opts);
      revisit.push(performance.now() - t);
    }

    const measureCalls = deck.info ? undefined : undefined;
    const slideCount = deck.slideCount;
    deck.destroy();
    canvas.remove();

    return { openMs, firstSlideMs, navigate, zoom, revisit, slideCount, measureCalls };
  }, fixtureName);
}

async function main() {
  await ensureFixture();

  let playwright;
  try {
    playwright = await import('playwright');
  } catch {
    console.error('playwright is not installed. Run `npm install`.');
    process.exit(2);
  }

  const wasm = join(ROOT, 'crates/wasm/pkg/pptx_bg.wasm');
  if (!existsSync(wasm)) {
    console.error('No WASM build found. Run `npm run wasm` first.');
    process.exit(2);
  }

  const server = await ensureServer();
  const browser = await playwright.chromium.launch();
  const results = [];
  try {
    for (let run = 0; run < runs; run++) {
      const page = await browser.newPage({ viewport: { width: 1280, height: 720 } });
      page.on('pageerror', (e) => console.error('page error:', String(e)));
      await page.goto(`${BASE}/?headless=1`, { waitUntil: 'load', timeout: 60_000 });
      results.push(await measure(page, fixture));
      await page.close();
    }
  } finally {
    await browser.close();
    server.stop();
  }

  const first = results[0];
  console.log(`\nFixture: ${fixture} (${first.slideCount} slides), ${runs} run(s)\n`);

  const rows = [
    ['open + parse index', stats(results.map((r) => r.openMs)), TARGETS.open],
    ['first slide (layout + draw)', stats(results.map((r) => r.firstSlideMs)), TARGETS.firstSlide],
    ['navigate (uncached slide)', stats(results.flatMap((r) => r.navigate)), TARGETS.navigate],
    ['revisit (cached layout)', stats(results.flatMap((r) => r.revisit)), TARGETS.zoom],
    ['zoom (no re-layout)', stats(results.flatMap((r) => r.zoom)), TARGETS.zoom],
  ];

  const width = Math.max(...rows.map(([label]) => label.length));
  console.log(
    `  ${'metric'.padEnd(width)}   median      p95      max   target`,
  );
  let failed = false;
  for (const [label, s, target] of rows) {
    const ok = s.median <= target;
    if (!ok) failed = true;
    console.log(
      `  ${label.padEnd(width)} ${fmt(s.median)} ${fmt(s.p95)} ${fmt(s.max)} ${fmt(target)}  ${ok ? '✓' : '✗'}`,
    );
  }

  // The structural claim the display list is built on, checked rather than assumed: a
  // zoom must reuse the cached display list instead of re-running layout. Only meaningful
  // when a navigation costs enough to compare against.
  const nav = stats(results.flatMap((r) => r.navigate)).median;
  const zoom = stats(results.flatMap((r) => r.zoom)).median;
  if (nav > 1) {
    console.log(
      `\n  A zoom is ${(nav / Math.max(zoom, 0.001)).toFixed(1)}x cheaper than an uncached navigation.`,
    );
    if (zoom > nav * 0.6) {
      console.log(
        '  ⚠️  Expected a zoom to be much cheaper: it should reuse the cached display list\n' +
          '     rather than re-running layout. Check that the layout cache is being hit.',
      );
      failed = true;
    }
  } else {
    console.log(
      `\n  Navigation is too cheap on this deck (${nav.toFixed(2)}ms) to compare against zoom.`,
    );
  }

  if (failed) {
    console.log('\nSome metrics missed their target.');
    process.exitCode = 1;
  } else {
    console.log('\nAll metrics within target.');
  }
}

function fmt(ms) {
  return `${ms.toFixed(1)}ms`.padStart(8);
}

await main();
