/**
 * The oracle: renders fixture decks with headless LibreOffice.
 *
 * LibreOffice converts a .pptx to PNG one slide at a time only, so the route is
 * pptx → PDF → per-page PNG via poppler's `pdftoppm`. That also gives control over the
 * raster resolution, which matters: the golden and the viewer output must be the same
 * size before they can be diffed.
 *
 * The oracle is not PowerPoint. It is *consistent*, which is what a regression detector
 * needs — but some diffs are its fault, not the viewer's. See suites.json.
 */

import { execFile } from 'node:child_process';
import { existsSync, mkdirSync, readdirSync, renameSync, rmSync, statSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';

const exec = promisify(execFile);

const HERE = dirname(fileURLToPath(import.meta.url));
export const ROOT = resolve(HERE, '../..');
export const FIXTURES = join(ROOT, 'fixtures/generated');
export const GOLDENS = join(ROOT, 'fixtures/goldens');
const CACHE = join(HERE, '.oracle-cache');

/** Candidate locations for `soffice`, most specific first. */
const SOFFICE_CANDIDATES = [
  process.env.SOFFICE,
  '/Applications/LibreOffice.app/Contents/MacOS/soffice',
  '/usr/bin/soffice',
  '/usr/local/bin/soffice',
  'soffice',
].filter(Boolean);

async function which(candidates, versionArg = '--version') {
  for (const candidate of candidates) {
    try {
      await exec(candidate, [versionArg], { timeout: 60_000 });
      return candidate;
    } catch {
      // Not this one.
    }
  }
  return null;
}

/** Locates the tools the oracle needs, or explains what is missing. */
export async function findTools() {
  const soffice = await which(SOFFICE_CANDIDATES);
  const pdftoppm = await which([process.env.PDFTOPPM, 'pdftoppm', '/opt/homebrew/bin/pdftoppm'].filter(Boolean), '-v');
  const missing = [];
  if (!soffice) {
    missing.push(
      'LibreOffice (`soffice`). Install it (macOS: `brew install --cask libreoffice`) or set SOFFICE.',
    );
  }
  if (!pdftoppm) {
    missing.push('poppler (`pdftoppm`). Install it (macOS: `brew install poppler`).');
  }
  return { soffice, pdftoppm, missing };
}

/**
 * Renders every slide of a fixture to a PNG, returning the paths in slide order.
 *
 * Results are cached against the fixture's mtime: LibreOffice takes several seconds per
 * deck and the fixtures rarely change, so re-rendering them on every test run would
 * dominate the suite's runtime.
 */
export async function renderFixture(fixture, { width, height, tools, force = false } = {}) {
  const { soffice, pdftoppm } = tools ?? (await findTools());
  if (!soffice || !pdftoppm) {
    throw new Error('oracle tools are not available');
  }

  const source = join(FIXTURES, fixture);
  if (!existsSync(source)) {
    throw new Error(`fixture ${fixture} does not exist; run \`npm run fixtures\``);
  }
  const stamp = String(statSync(source).mtimeMs);
  const name = fixture.replace(/\.pptx$/i, '');
  const outDir = join(GOLDENS, name);
  const stampFile = join(outDir, '.stamp');

  if (!force && existsSync(stampFile)) {
    const { readFileSync } = await import('node:fs');
    if (readFileSync(stampFile, 'utf8') === stamp) {
      const existing = listPngs(outDir);
      if (existing.length > 0) return existing;
    }
  }

  rmSync(outDir, { recursive: true, force: true });
  mkdirSync(outDir, { recursive: true });
  mkdirSync(CACHE, { recursive: true });

  // A dedicated profile directory keeps concurrent runs from fighting over the shared
  // one, which is the usual cause of "soffice produced no output" flakes.
  const profile = join(CACHE, `profile-${name}`);
  await exec(
    soffice,
    [
      '--headless',
      '--norestore',
      '--invisible',
      `-env:UserInstallation=file://${profile}`,
      '--convert-to',
      'pdf',
      '--outdir',
      CACHE,
      source,
    ],
    { timeout: 180_000 },
  );

  const pdf = join(CACHE, `${name}.pdf`);
  if (!existsSync(pdf)) {
    throw new Error(`LibreOffice produced no PDF for ${fixture}`);
  }

  // `-scale-to-x/-y` pins the output to exactly the size the viewer renders at, so the
  // diff never has to resample — resampling would blur real differences away.
  await exec(
    pdftoppm,
    ['-png', '-scale-to-x', String(width), '-scale-to-y', String(height), pdf, join(outDir, 'slide')],
    { timeout: 180_000 },
  );

  const { writeFileSync } = await import('node:fs');
  writeFileSync(stampFile, stamp);

  // pdftoppm names pages `slide-1.png`, `slide-01.png` or `slide-001.png` depending on
  // the page count. Normalise to a zero-padded, sortable form.
  const produced = listPngs(outDir);
  const normalised = [];
  produced.forEach((file, i) => {
    const target = join(outDir, `slide-${String(i + 1).padStart(3, '0')}.png`);
    if (file !== target) renameSync(file, target);
    normalised.push(target);
  });
  return normalised;
}

function listPngs(dir) {
  if (!existsSync(dir)) return [];
  return readdirSync(dir)
    .filter((f) => f.endsWith('.png'))
    .sort((a, b) => {
      const na = Number.parseInt(a.replace(/\D+/g, ''), 10) || 0;
      const nb = Number.parseInt(b.replace(/\D+/g, ''), 10) || 0;
      return na - nb;
    })
    .map((f) => join(dir, f));
}

/** Renders every suite's fixture. Used by `npm run goldens`. */
async function main() {
  const { readFileSync } = await import('node:fs');
  const config = JSON.parse(readFileSync(join(HERE, 'suites.json'), 'utf8'));
  const tools = await findTools();
  if (tools.missing.length > 0) {
    console.error('Cannot render goldens. Missing:');
    for (const m of tools.missing) console.error(`  - ${m}`);
    process.exitCode = 1;
    return;
  }
  const force = process.argv.includes('--force');
  for (const suite of config.suites) {
    const width = suite.width ?? config.defaults.width;
    const height = suite.height ?? config.defaults.height;
    process.stdout.write(`${suite.id}: rendering ${suite.fixture} … `);
    try {
      const pages = await renderFixture(suite.fixture, { width, height, tools, force });
      console.log(`${pages.length} slide(s)`);
    } catch (e) {
      console.log(`failed: ${e.message}`);
      process.exitCode = 1;
    }
  }
}

if (process.argv[1] && resolve(process.argv[1]) === resolve(fileURLToPath(import.meta.url))) {
  await main();
}
