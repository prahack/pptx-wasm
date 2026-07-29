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

/**
 * Signals a spawned server and everything it started.
 *
 * Negating the pid addresses the process group, which is why the server is spawned
 * detached. SIGKILL follows SIGTERM after a moment because vite occasionally declines to
 * take the hint, and a test run must not depend on its manners.
 */
export function killTree(child) {
  if (!child?.pid) return;
  const signal = (sig) => {
    try {
      process.kill(-child.pid, sig);
    } catch {
      // Already gone, or never became a group leader; the direct kill is the fallback.
      try {
        child.kill(sig);
      } catch {
        /* nothing left to signal */
      }
    }
  };
  signal('SIGTERM');
  const hard = setTimeout(() => signal('SIGKILL'), 3000);
  // Do not let the timer itself hold the event loop open — that would be the same bug.
  hard.unref?.();
}

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
  // `detached` puts the server in its own process group so the whole tree can be
  // signalled at once. Without it `child.kill()` reaches only the `npm` wrapper: npm
  // spawns vite, vite spawns esbuild, and those two outlive it holding the port open.
  // Locally that goes unnoticed because the shell lives on anyway. On a CI runner the
  // step never exits — the golden suite reported 20/20 and then sat there for six hours
  // until the job was cancelled.
  const child = spawn('npm', ['run', 'dev', '--workspace', 'packages/viewer'], {
    cwd: ROOT,
    stdio: ['ignore', 'pipe', 'pipe'],
    env: { ...process.env, BROWSER: 'none' },
    detached: true,
  });
  let stderr = '';
  child.stderr.on('data', (d) => {
    stderr += String(d);
  });

  const deadline = Date.now() + 90_000;
  while (Date.now() < deadline) {
    if (await isUp()) {
      return { stop: () => killTree(child), started: true };
    }
    if (child.exitCode !== null) {
      throw new Error(
        `the dev server exited (${child.exitCode}). Is the WASM built? Run \`npm run wasm\`.\n${stderr}`,
      );
    }
    await sleep(250);
  }
  killTree(child);
  throw new Error(`the dev server did not come up on ${BASE} within 90s:\n${stderr}`);
}
