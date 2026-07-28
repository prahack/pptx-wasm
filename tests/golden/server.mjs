/**
 * The dev server the browser-driven tests run against.
 *
 * Shared by the golden runner, the bench and the cross-browser check so that all three
 * behave the same way: reuse a server you already have open, start one if you do not, and
 * shut down only what they started.
 */

import { spawn } from 'node:child_process';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
export const ROOT = resolve(HERE, '../..');

export const PORT = 5178;
export const BASE = `http://localhost:${PORT}`;

/**
 * Identifies *our* dev server.
 *
 * Checking only that something answers is not enough: this port range is routinely
 * occupied by an unrelated project's Vite server, and reusing one silently tests the
 * wrong application. The page title is the cheapest reliable marker.
 */
const MARKER = '<title>pptx-wasm dev</title>';

export async function isUp() {
  try {
    const res = await fetch(BASE, { signal: AbortSignal.timeout(1500) });
    if (!res.ok) return false;
    return (await res.text()).includes(MARKER);
  } catch {
    return false;
  }
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/**
 * Ensures a dev server is running.
 *
 * Returns a `stop()` to call when finished. If a suitable server was already up, `stop()`
 * is a no-op — a developer with `npm run dev` open should not have it killed from under
 * them by a test run.
 */
export async function ensureServer({ quiet = false } = {}) {
  if (await isUp()) {
    if (!quiet) console.log(`Using the dev server already listening on ${BASE}`);
    return { stop: () => {}, started: false };
  }

  if (!quiet) console.log('Starting the dev server…');
  const child = spawn('npm', ['run', 'dev', '--workspace', 'packages/viewer'], {
    cwd: ROOT,
    stdio: ['ignore', 'pipe', 'pipe'],
    env: { ...process.env, BROWSER: 'none' },
  });
  let stderr = '';
  child.stderr.on('data', (d) => {
    stderr += String(d);
  });

  const deadline = Date.now() + 90_000;
  while (Date.now() < deadline) {
    if (await isUp()) {
      return { stop: () => child.kill(), started: true };
    }
    if (child.exitCode !== null) {
      throw new Error(
        `the dev server exited (${child.exitCode}). Is the WASM built? Run \`npm run wasm\`.\n${stderr}`,
      );
    }
    await sleep(250);
  }
  child.kill();
  throw new Error(`the dev server did not come up on ${BASE} within 90s:\n${stderr}`);
}
