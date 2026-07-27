/**
 * Cross-browser smoke test.
 *
 * Renders every fixture in Chromium, Firefox and WebKit and checks three things:
 * the deck opens, the canvas is not blank, and the extracted text is identical
 * everywhere.
 *
 * The third check is the one that matters. Spike A chose Canvas2D `measureText` as the
 * metrics source, accepting that the three engines do not return byte-identical advances
 * — so wrap points can in principle differ between browsers. This test is where that
 * theory meets evidence: it reports how far the line breaks actually diverge, so the
 * decision recorded in CLAUDE.md can be revisited on data rather than on worry.
 *
 *   npm run test:browsers
 */

import { execFile } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';

import { PNG } from 'pngjs';

import { BASE, ROOT, ensureServer } from './server.mjs';

const exec = promisify(execFile);
const HERE = dirname(fileURLToPath(import.meta.url));

const config = JSON.parse(readFileSync(join(HERE, 'suites.json'), 'utf8'));
const only = process.argv
  .find((a) => a.startsWith('--suite='))
  ?.slice('--suite='.length);
const suites = config.suites.filter((s) => !only || s.id === only);

async function ensureFixtures() {
  const python = join(ROOT, '.venv/bin/python');
  if (!existsSync(python)) {
    console.error('No .venv found; cannot generate fixtures.');
    process.exit(2);
  }
  await exec(python, [join(ROOT, 'fixtures/gen.py')], { cwd: ROOT, timeout: 300_000 });
}

/** Fraction of pixels that are not the background colour. */
function inkRatio(buffer) {
  const png = PNG.sync.read(buffer);
  let ink = 0;
  for (let i = 0; i < png.data.length; i += 4) {
    const r = png.data[i];
    const g = png.data[i + 1];
    const b = png.data[i + 2];
    const a = png.data[i + 3];
    if (a > 8 && (r < 245 || g < 245 || b < 245)) ink++;
  }
  return ink / (png.width * png.height);
}

async function renderIn(browser, fixture, slide, width, height) {
  const page = await browser.newPage({ viewport: { width, height }, deviceScaleFactor: 1 });
  const errors = [];
  page.on('pageerror', (e) => errors.push(String(e)));
  try {
    await page.goto(
      `${BASE}/?headless=1&fixture=${encodeURIComponent(fixture)}&slide=${slide}&w=${width}&h=${height}`,
      { waitUntil: 'load', timeout: 90_000 },
    );
    await page.waitForFunction(
      () => window.__pptxReady === true || window.__pptxError,
      null,
      { timeout: 90_000 },
    );
    const error = await page.evaluate(() => window.__pptxError);
    if (error) throw new Error(error);
    const buffer = await page.locator('canvas').screenshot({ type: 'png' });
    // The accessibility text layer is what the viewer believes it drew, which is what we
    // compare across engines.
    const text = await page.evaluate(() => {
      const el = document.querySelector('[role="region"] div:last-child');
      return el?.textContent ?? '';
    });
    return { buffer, text, errors };
  } finally {
    await page.close();
  }
}

async function main() {
  await ensureFixtures();

  if (!existsSync(join(ROOT, 'crates/wasm/pkg/pptx_bg.wasm'))) {
    console.error('No WASM build found. Run `npm run wasm` first.');
    process.exit(2);
  }
  const playwright = await import('playwright');
  const server = await ensureServer();
  const engines = [
    ['chromium', playwright.chromium, 'Chrome / Edge'],
    ['firefox', playwright.firefox, 'Firefox'],
    ['webkit', playwright.webkit, 'Safari'],
  ];

  const results = [];
  let failed = false;

  try {
  for (const [id, type, label] of engines) {
    let browser;
    try {
      browser = await type.launch();
    } catch (e) {
      console.error(`  ! ${label}: could not launch (${e.message.split('\n')[0]})`);
      console.error(`    Run \`npx playwright install ${id}\`.`);
      failed = true;
      continue;
    }
    try {
      for (const suite of suites) {
        const width = suite.width ?? config.defaults.width;
        const height = suite.height ?? config.defaults.height;
        try {
          const out = await renderIn(browser, suite.fixture, 0, width, height);
          results.push({
            engine: id,
            label,
            suite: suite.id,
            ink: inkRatio(out.buffer),
            text: out.text,
            errors: out.errors,
          });
        } catch (e) {
          results.push({ engine: id, label, suite: suite.id, error: e.message });
          failed = true;
        }
      }
    } finally {
      await browser.close();
    }
  }
  } finally {
    server.stop();
  }

  // --- report -----------------------------------------------------------------

  console.log('\nCross-browser smoke test\n');
  for (const suite of suites) {
    const forSuite = results.filter((r) => r.suite === suite.id);
    if (forSuite.length === 0) continue;

    const problems = [];
    for (const r of forSuite) {
      if (r.error) {
        problems.push(`${r.label}: ${r.error}`);
        continue;
      }
      // A blank canvas means the deck opened but nothing was drawn — the failure mode a
      // pixel diff against a per-engine reference would not catch.
      if (suite.id !== 'm0' && r.ink < 0.001) {
        problems.push(`${r.label}: canvas is blank (ink ${(r.ink * 100).toFixed(3)}%)`);
        failed = true;
      }
      if (r.errors?.length) {
        problems.push(`${r.label}: ${r.errors[0]}`);
        failed = true;
      }
    }

    // Text must match across engines: it comes from layout, which must not depend on the
    // browser beyond metrics.
    const texts = forSuite.filter((r) => !r.error).map((r) => r.text ?? '');
    const distinct = new Set(texts);
    if (distinct.size > 1) {
      problems.push(`extracted text differs between engines (${distinct.size} variants)`);
      failed = true;
    }

    const inks = forSuite
      .filter((r) => !r.error)
      .map((r) => `${r.engine}=${((r.ink ?? 0) * 100).toFixed(1)}%`)
      .join(' ');
    console.log(`  ${problems.length === 0 ? '✓' : '✗'} ${suite.id.padEnd(5)} ink ${inks}`);
    for (const p of problems) console.log(`      ${p}`);
  }

  console.log(
    failed
      ? '\nSome engines failed. See above.'
      : '\nAll fixtures render with content and identical text in Chromium, Firefox and WebKit.',
  );
  if (failed) process.exitCode = 1;
}

await main();
