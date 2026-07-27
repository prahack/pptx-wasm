/**
 * WASM module loading.
 *
 * The module is instantiated once per page and shared. Two loading strategies are
 * supported and the default matters: `import init from '../../../crates/wasm/pkg/pptx.js'`
 * lets a bundler fingerprint and inline the `.wasm` next to the JS glue, which is what
 * makes the package work with no configuration. A consumer who would rather serve the
 * module from a CDN passes a URL to `initWasm` (or the `wasm` prop) instead.
 */

import initModule, * as bindings from '../../../crates/wasm/pkg/pptx.js';
import { PptxError, type WasmSource } from './types.js';

export type WasmModule = typeof bindings;

let ready: Promise<WasmModule> | null = null;
let loadedFrom: WasmSource | undefined;

/**
 * Instantiates the WASM module, or returns the already-instantiated one.
 *
 * Calling this with a different `source` after the module has loaded is a no-op and warns:
 * a second instantiation would give two independent heaps and two independent image
 * caches, which is never what the caller meant.
 */
export async function initWasm(source?: WasmSource): Promise<WasmModule> {
  if (ready) {
    if (source !== undefined && source !== loadedFrom) {
      console.warn(
        '[pptx-viewer] the WASM module is already loaded; the new `wasm` source is ignored',
      );
    }
    return ready;
  }
  loadedFrom = source;
  ready = (async () => {
    try {
      // `initModule` accepts a URL, a Response, a module, or bytes; passing nothing lets
      // the generated glue resolve the .wasm relative to itself.
      await initModule(source === undefined ? undefined : ({ module_or_path: source } as never));
      return bindings;
    } catch (e) {
      // A failed instantiation must not poison the cache: a transient network error
      // should be retryable.
      ready = null;
      loadedFrom = undefined;
      throw new PptxError(
        `could not load the pptx-viewer WASM module: ${e instanceof Error ? e.message : String(e)}`,
        e,
      );
    }
  })();
  return ready;
}

/** True once the module is instantiated. */
export function isWasmReady(): boolean {
  return ready !== null;
}

/** Version of the loaded WASM module. */
export async function wasmVersion(): Promise<string> {
  const m = await initWasm();
  return m.version();
}
