/**
 * The golden-file runner.
 *
 * For each suite: generate the fixture, render it with the oracle, render it with the
 * viewer in headless Chromium, and pixel-diff the two. Failures write the actual render
 * and a diff image so the difference can be looked at rather than guessed at.
 *
 *   npm run test:golden
 *   npm run test:golden -- --suite=m2
 *   npm run test:golden -- --update      # accept current output as the reference
 *
 * On a machine without the oracle tools, the suite degrades to *self-comparison*: it
 * still renders every slide and diffs against the last accepted output in
 * `tests/golden/out/reference/`. That catches regressions, which is most of the value,
 * without pretending it has verified fidelity — and it says so.
 *
 * A suite can also set `"oracle": false` in suites.json, for features LibreOffice renders
 * *wrongly* — there, diffing against it would score a correct render worse than a broken
 * one. Those compare against a reviewed, committed reference in `tests/golden/reference/`.
 */

import { execFile, spawn } from 'node:child_process';
import {
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  writeFileSync,
} from 'node:fs';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';

import pixelmatch from 'pixelmatch';
import { PNG } from 'pngjs';

import { findTools, renderFixture, ROOT } from './oracle.mjs';
import { BASE, ensureServer } from './server.mjs';

const exec = promisify(execFile);
const HERE = dirname(fileURLToPath(import.meta.url));
const OUT = join(HERE, 'out');
// Two different kinds of "reference", deliberately in different places.
//   SELF_REF  scratch, under the gitignored out/ — used when LibreOffice is missing, so
//             a run still detects regressions on a machine without the full toolchain.
//   REVIEWED  committed — the reference for suites that set `oracle: false` because the
//             oracle renders the feature wrongly. This one has to be in the repository:
//             if it were gitignored, a fresh checkout would re-record whatever the code
//             currently does and the suite would assert nothing at all.
const SELF_REF = join(OUT, 'reference');
const REVIEWED = join(HERE, 'reference');
const args = process.argv.slice(2);
const only = args.find((a) => a.startsWith('--suite='))?.slice('--suite='.length);
const update = args.includes('--update');
const keepServer = args.includes('--keep-server');

const config = JSON.parse(readFileSync(join(HERE, 'suites.json'), 'utf8'));
const suites = config.suites.filter((s) => !only || s.id === only);

if (suites.length === 0) {
  console.error(`no suite matches --suite=${only}. Available: ${config.suites.map((s) => s.id).join(', ')}`);
  process.exit(2);
}

// --------------------------------------------------------------------- fixtures

async function ensureFixtures() {
  const python = join(ROOT, '.venv/bin/python');
  if (!existsSync(python)) {
    console.error(
      'No .venv found. Create it and install python-pptx:\n' +
        '  python3 -m venv .venv && ./.venv/bin/pip install python-pptx Pillow',
    );
    process.exit(2);
  }
  await exec(python, [join(ROOT, 'fixtures/gen.py')], { cwd: ROOT, timeout: 300_000 });
}

// --------------------------------------------------------------------- rendering

async function renderWithViewer(browser, fixture, slide, width, height) {
  const page = await browser.newPage({
    viewport: { width, height },
    deviceScaleFactor: 1,
  });
  const consoleErrors = [];
  page.on('console', (m) => {
    if (m.type() === 'error') consoleErrors.push(m.text());
  });
  page.on('pageerror', (e) => consoleErrors.push(String(e)));

  try {
    const url = `${BASE}/?headless=1&fixture=${encodeURIComponent(fixture)}&slide=${slide}&w=${width}&h=${height}`;
    await page.goto(url, { waitUntil: 'load', timeout: 60_000 });
    await page.waitForFunction(
      () => window.__pptxReady === true || window.__pptxError,
      null,
      { timeout: 60_000 },
    );

    const error = await page.evaluate(() => window.__pptxError);
    if (error) throw new Error(`the viewer reported: ${error}`);

    const trace = await page.evaluate(() => window.__pptxTrace ?? '');
    const canvas = page.locator('canvas');
    const buffer = await canvas.screenshot({ type: 'png' });
    return { buffer, trace, consoleErrors };
  } finally {
    await page.close();
  }
}

// --------------------------------------------------------------------- diffing

function readPng(buffer) {
  return PNG.sync.read(buffer);
}

/**
 * Compares two PNGs, resizing neither.
 *
 * A size mismatch is reported as a failure rather than papered over by scaling: it means
 * the viewer and the oracle disagree about the slide's aspect ratio, which is a real bug
 * that resampling would hide.
 */
function compare(actualBuffer, expectedBuffer, threshold) {
  const actual = readPng(actualBuffer);
  const expected = readPng(expectedBuffer);
  if (actual.width !== expected.width || actual.height !== expected.height) {
    return {
      ok: false,
      sizeMismatch: `${actual.width}x${actual.height} vs ${expected.width}x${expected.height}`,
      ratio: 1,
      diff: null,
    };
  }
  const diff = new PNG({ width: actual.width, height: actual.height });
  const differing = pixelmatch(
    actual.data,
    expected.data,
    diff.data,
    actual.width,
    actual.height,
    { threshold, includeAA: false },
  );
  const ratio = differing / (actual.width * actual.height);
  return { ok: true, ratio, diff, differing };
}

// --------------------------------------------------------------------- main

/**
 * Warns when the built WASM is older than the Rust it was built from.
 *
 * Worth its length: a stale module makes the suite test the *previous* build, and the
 * failure looks exactly like a code bug. This turns half an hour of confusion into a
 * line of output.
 */
function checkWasmFreshness() {
  const wasm = join(ROOT, 'crates/wasm/pkg/pptx_bg.wasm');
  if (!existsSync(wasm)) {
    console.error('No WASM build found. Run `npm run wasm` first.');
    process.exit(2);
  }
  const built = statSync(wasm).mtimeMs;
  let newest = 0;
  let newestFile = '';
  const walk = (dir) => {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      if (entry.name === 'target' || entry.name === 'pkg') continue;
      const full = join(dir, entry.name);
      if (entry.isDirectory()) {
        walk(full);
      } else if (entry.name.endsWith('.rs') || entry.name === 'Cargo.toml') {
        const m = statSync(full).mtimeMs;
        if (m > newest) {
          newest = m;
          newestFile = full;
        }
      }
    }
  };
  walk(join(ROOT, 'crates'));
  if (newest > built) {
    console.warn(
      `\n⚠️  ${newestFile.replace(ROOT + '/', '')} is newer than the built WASM.\n` +
        '   Run `npm run wasm` — otherwise this suite tests the previous build.\n',
    );
  }
}

async function main() {
  checkWasmFreshness();
  console.log('Generating fixtures…');
  await ensureFixtures();

  const tools = await findTools();
  const haveOracle = tools.missing.length === 0;
  if (!haveOracle) {
    console.log('\n⚠️  The LibreOffice oracle is unavailable:');
    for (const m of tools.missing) console.log(`   - ${m}`);
    console.log(
      '   Falling back to self-comparison against tests/golden/out/reference/.\n' +
        '   This detects regressions but does NOT verify fidelity.\n',
    );
  }

  let playwright;
  try {
    playwright = await import('playwright');
  } catch {
    console.error('playwright is not installed. Run `npm install`.');
    process.exit(2);
  }

  let browser;
  try {
    browser = await playwright.chromium.launch();
  } catch (e) {
    console.error(
      'Could not launch Chromium. Run `npx playwright install chromium`.\n' + String(e.message),
    );
    process.exit(2);
  }

  const server = await ensureServer();
  rmSync(join(OUT, 'actual'), { recursive: true, force: true });
  rmSync(join(OUT, 'diff'), { recursive: true, force: true });
  mkdirSync(join(OUT, 'actual'), { recursive: true });
  mkdirSync(join(OUT, 'diff'), { recursive: true });
  mkdirSync(join(OUT, 'trace'), { recursive: true });
  mkdirSync(SELF_REF, { recursive: true });
  mkdirSync(REVIEWED, { recursive: true });

  const results = [];

  try {
    for (const suite of suites) {
      const width = suite.width ?? config.defaults.width;
      const height = suite.height ?? config.defaults.height;
      const threshold = suite.threshold ?? config.defaults.threshold;
      const limit = suite.maxDiffRatio ?? config.defaults.maxDiffRatio;

      // A suite can opt out of the oracle when LibreOffice renders the feature wrongly,
      // in which case a pixel diff against it is worse than no test at all — it passes
      // for the wrong reason and would keep passing through a regression. Such a suite
      // compares against a reviewed reference instead: that catches regressions, which
      // is the part the oracle was providing anyway. `reason` in suites.json says why.
      const suiteHasOracle = haveOracle && suite.oracle !== false;

      let goldens = [];
      if (suiteHasOracle) {
        try {
          goldens = await renderFixture(suite.fixture, { width, height, tools });
        } catch (e) {
          results.push({ suite: suite.id, slide: '-', status: 'error', detail: e.message });
          continue;
        }
      }

      // Without the oracle we do not know the slide count up front, so ask the viewer.
      const slideCount = suiteHasOracle
        ? goldens.length
        : await countSlides(browser, suite.fixture);

      for (let slide = 0; slide < slideCount; slide++) {
        const label = `${suite.id}/slide${slide + 1}`;
        let rendered;
        try {
          rendered = await renderWithViewer(browser, suite.fixture, slide, width, height);
        } catch (e) {
          results.push({ suite: suite.id, slide: slide + 1, status: 'error', detail: e.message });
          continue;
        }

        const actualPath = join(OUT, 'actual', `${suite.id}-${slide + 1}.png`);
        writeFileSync(actualPath, rendered.buffer);
        writeFileSync(join(OUT, 'trace', `${suite.id}-${slide + 1}.txt`), rendered.trace);

        if (rendered.consoleErrors.length > 0) {
          results.push({
            suite: suite.id,
            slide: slide + 1,
            status: 'error',
            detail: `console: ${rendered.consoleErrors[0]}`,
          });
          continue;
        }

        const referencePath = suiteHasOracle
          ? goldens[slide]
          : join(suite.oracle === false ? REVIEWED : SELF_REF, `${suite.id}-${slide + 1}.png`);

        if (update || !existsSync(referencePath)) {
          if (!suiteHasOracle) {
            writeFileSync(referencePath, rendered.buffer);
            results.push({
              suite: suite.id,
              slide: slide + 1,
              status: 'recorded',
              detail:
                suite.oracle === false
                  ? `written to ${relative(ROOT, referencePath)} — review it by eye and commit it; ` +
                    'until then this suite asserts nothing'
                  : 'reference written',
            });
            continue;
          }
        }
        if (!existsSync(referencePath)) {
          results.push({
            suite: suite.id,
            slide: slide + 1,
            status: 'error',
            detail: 'no reference image',
          });
          continue;
        }

        const cmp = compare(rendered.buffer, readFileSync(referencePath), threshold);
        if (!cmp.ok) {
          results.push({
            suite: suite.id,
            slide: slide + 1,
            status: 'fail',
            detail: `size mismatch: ${cmp.sizeMismatch}`,
          });
          continue;
        }
        if (cmp.diff) {
          writeFileSync(join(OUT, 'diff', `${suite.id}-${slide + 1}.png`), PNG.sync.write(cmp.diff));
        }
        const pass = cmp.ratio <= limit;
        results.push({
          suite: suite.id,
          slide: slide + 1,
          status: pass ? 'pass' : 'fail',
          ratio: cmp.ratio,
          limit,
          detail:
            `${(cmp.ratio * 100).toFixed(3)}% differ (limit ${(limit * 100).toFixed(2)}%)` +
            // Say so on every line, not once in a footnote: a number compared against our
            // own past output means something weaker than one compared against the oracle.
            (suiteHasOracle ? '' : '  [vs recorded reference, not the oracle]'),
        });
        process.stdout.write(pass ? '.' : 'F');
        void label;
      }
    }
  } finally {
    await browser.close();
    if (!keepServer) server.stop();
  }

  report(results, haveOracle);
}

/** Opens a fixture just to count its slides, for the no-oracle path. */
async function countSlides(browser, fixture) {
  const page = await browser.newPage();
  try {
    await page.goto(`${BASE}/?headless=1&fixture=${encodeURIComponent(fixture)}&slide=0`, {
      waitUntil: 'load',
      timeout: 60_000,
    });
    await page.waitForFunction(
      () => window.__pptxReady === true || window.__pptxError,
      null,
      { timeout: 60_000 },
    );
    return await page.evaluate(() => {
      const label = document.querySelector('[aria-label^="Slide "]')?.getAttribute('aria-label');
      const m = label?.match(/of (\d+)/);
      return m ? Number.parseInt(m[1], 10) : 1;
    });
  } catch {
    return 1;
  } finally {
    await page.close();
  }
}

function report(results, haveOracle) {
  console.log('\n');
  const width = Math.max(...results.map((r) => `${r.suite}/slide${r.slide}`.length), 12);
  for (const r of results) {
    const name = `${r.suite}/slide${r.slide}`.padEnd(width);
    const mark = { pass: '✓', fail: '✗', error: '!', recorded: '+' }[r.status];
    console.log(`  ${mark} ${name}  ${r.detail ?? ''}`);
  }

  const failed = results.filter((r) => r.status === 'fail' || r.status === 'error');
  const passed = results.filter((r) => r.status === 'pass').length;
  console.log(
    `\n${passed} passed, ${failed.length} failed, ${results.length} total` +
      (haveOracle ? '' : '  (self-comparison; fidelity NOT verified)'),
  );

  if (failed.length > 0) {
    console.log(`\nArtefacts:\n  actual: ${join(OUT, 'actual')}\n  diff:   ${join(OUT, 'diff')}`);
    console.log(
      `  trace:  ${join(OUT, 'trace')}   ← check this first; it says whether layout moved` +
        ' something or the rasteriser drew it differently',
    );
    process.exitCode = 1;
  }
}

await main();
