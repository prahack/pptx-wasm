/**
 * pptx-viewer — a browser-native, read-only `.pptx` renderer.
 *
 * Two entry points: this one, which is framework-agnostic, and `pptx-viewer/react` for
 * the `<PresentationViewer/>` component.
 *
 * ```ts
 * const deck = await Presentation.open('/deck.pptx');
 * await deck.render(0, canvas, { fit: 'contain' });
 * ```
 */

export { Presentation, type OpenOptions } from './presentation.js';
export { initWasm, isWasmReady, wasmVersion } from './wasm.js';
export {
  PptxError,
  type EmbeddedFont,
  type EmbeddedFontVariant,
  type Fit,
  type PresentationInfo,
  type PresentationViewerProps,
  type RenderOptions,
  type SlideInfo,
  type WasmSource,
} from './types.js';
