/**
 * The framework-agnostic API layer.
 *
 * Everything asynchronous lives here — module instantiation, fetching, image decoding,
 * font loading — so the Rust side can stay synchronous. A synchronous wasm call cannot
 * await an `ImageBitmap`, so the shape of the contract is: Rust says what it needs, this
 * layer gets it, hands it back, and only then asks for a frame.
 */

import { initWasm, type WasmModule } from './wasm.js';
import {
  PptxError,
  type EmbeddedFont,
  type PresentationInfo,
  type RenderOptions,
  type SlideInfo,
  type WasmSource,
} from './types.js';

export interface OpenOptions {
  /** Where to load the WASM module from. */
  wasm?: WasmSource;
  /** Install the deck's embedded fonts as `FontFace`s. Default true. */
  useEmbeddedFonts?: boolean;
  signal?: AbortSignal;
}

/** An open presentation. Call {@link Presentation.destroy} when finished with it. */
export class Presentation {
  #inner: InstanceType<WasmModule['Presentation']>;
  #info: PresentationInfo;
  #destroyed = false;
  /** Image ids currently being decoded, so a re-render does not start a second decode. */
  #decoding = new Set<number>();
  /** Fonts already handed to `document.fonts.load()`. */
  #fontsLoaded = new Set<string>();
  #installedFonts: FontFace[] = [];
  #assetListeners = new Set<() => void>();

  private constructor(inner: InstanceType<WasmModule['Presentation']>, info: PresentationInfo) {
    this.#inner = inner;
    this.#info = info;
  }

  /** Opens a presentation from a URL, `File`/`Blob`, or raw bytes. */
  static async open(
    src: string | File | Blob | ArrayBuffer | Uint8Array,
    options: OpenOptions = {},
  ): Promise<Presentation> {
    const wasm = await initWasm(options.wasm);
    const bytes = await toBytes(src, options.signal);
    let inner: InstanceType<WasmModule['Presentation']>;
    try {
      inner = new wasm.Presentation(bytes);
    } catch (e) {
      throw new PptxError(`could not open the presentation: ${errorText(e)}`, e);
    }

    const info: PresentationInfo = {
      slideCount: inner.slideCount(),
      width: inner.slideWidth(),
      height: inner.slideHeight(),
      slides: parseJson<SlideInfo[]>(inner.slideIndex(), []),
      embeddedFonts: parseJson<EmbeddedFont[]>(inner.embeddedFonts(), []),
    };

    const presentation = new Presentation(inner, info);
    if (options.useEmbeddedFonts !== false && info.embeddedFonts.length > 0) {
      // Best-effort: a deck whose embedded fonts fail to install still renders, with
      // substituted faces. Failing the open would be worse.
      await presentation.#installEmbeddedFonts().catch((e) => {
        console.warn('[pptx-viewer] embedded fonts could not be installed', e);
      });
    }
    return presentation;
  }

  /**
   * Subscribes to "the assets a frame was missing have arrived".
   *
   * Images decode asynchronously, so the first frame of a slide with pictures draws
   * without them. Rather than have callers guess a delay and redraw on a timer, this
   * fires once the outstanding decodes for a slide have finished. Returns an
   * unsubscribe function.
   */
  onAssetsReady(listener: () => void): () => void {
    this.#assetListeners.add(listener);
    return () => this.#assetListeners.delete(listener);
  }

  /**
   * How many images are still decoding.
   *
   * Zero does not mean a slide is fully drawn — it means nothing is outstanding. Test
   * harnesses use it to know when a frame is final; application code should prefer
   * {@link Presentation.onAssetsReady}.
   */
  pendingAssetCount(): number {
    return this.#decoding.size;
  }

  #notifyAssetsReady(): void {
    for (const listener of this.#assetListeners) {
      try {
        listener();
      } catch (e) {
        console.warn('[pptx-viewer] an onAssetsReady listener threw', e);
      }
    }
  }

  get info(): PresentationInfo {
    return this.#info;
  }

  get slideCount(): number {
    return this.#info.slideCount;
  }

  /** Slide size in points. */
  get slideSize(): { width: number; height: number } {
    return { width: this.#info.width, height: this.#info.height };
  }

  /**
   * Lays a slide out without drawing it.
   *
   * Worth calling on the next and previous slides after a navigation: layout is the
   * expensive half, and doing it ahead of time is what makes arrow-key navigation feel
   * instant rather than merely fast.
   */
  prepare(index: number): void {
    if (this.#destroyed || !this.#valid(index)) return;
    try {
      this.#inner.prepare(index);
    } catch {
      // Out of range or a slide we cannot parse; render() reports it properly.
    }
  }

  /**
   * Draws a slide into a canvas.
   *
   * Resolves once the slide is drawn *with everything that was ready*. Images and fonts
   * that were not decoded yet are fetched, and the caller is told to draw again via the
   * returned `complete` flag — a viewer typically renders once for the layout and once
   * more when the assets arrive, rather than blocking on them.
   */
  async render(
    index: number,
    canvas: HTMLCanvasElement,
    options: RenderOptions = {},
  ): Promise<{ complete: boolean }> {
    if (this.#destroyed) throw new PptxError('presentation has been destroyed');
    if (!this.#valid(index)) {
      throw new PptxError(`slide ${index} is out of range (0..${this.slideCount - 1})`);
    }

    const width = options.width ?? canvas.clientWidth ?? canvas.width;
    const height = options.height ?? canvas.clientHeight ?? canvas.height;
    const dpr = options.dpr ?? (typeof window !== 'undefined' ? window.devicePixelRatio : 1) ?? 1;

    // Size the backing store, but only when it actually changed: assigning to
    // canvas.width clears it, so an unconditional assignment flashes on every frame.
    const pw = Math.max(1, Math.round(width * dpr));
    const ph = Math.max(1, Math.round(height * dpr));
    if (canvas.width !== pw) canvas.width = pw;
    if (canvas.height !== ph) canvas.height = ph;

    const ctx = canvas.getContext('2d');
    if (!ctx) throw new PptxError('could not get a 2D context from the canvas');

    await this.#ensureFonts(index);
    const pending = this.#startDecoding(index);

    try {
      this.#inner.renderSlide(
        index,
        ctx,
        width,
        height,
        dpr,
        options.fit ?? 'contain',
        options.zoom ?? 1,
        options.panX ?? 0,
        options.panY ?? 0,
      );
    } catch (e) {
      throw new PptxError(`could not render slide ${index}: ${errorText(e)}`, e);
    }
    return { complete: pending.length === 0 };
  }

  /** A slide's text, in draw order. For search, selection and accessibility. */
  text(index: number): string {
    if (this.#destroyed || !this.#valid(index)) return '';
    return this.#inner.slideText(index);
  }

  /** Speaker notes for a slide. */
  notes(index: number): string {
    if (this.#destroyed || !this.#valid(index)) return '';
    return this.#inner.slideNotes(index);
  }

  /**
   * A textual dump of a slide's draw commands.
   *
   * Diagnostic, not API: it is how you tell a layout bug from a rasterisation bug when a
   * slide looks wrong. The format is not stable.
   */
  debugTrace(index: number, width = 960, height = 540): string {
    if (this.#destroyed || !this.#valid(index)) return '';
    return this.#inner.debugTrace(index, width, height);
  }

  /** What a WebGPU backend would need for this slide. Diagnostic. */
  gpuRequirements(index: number): string {
    if (this.#destroyed || !this.#valid(index)) return '';
    return this.#inner.gpuRequirements(index);
  }

  /** Drops cached layouts. Rebuilt on demand. */
  evict(): void {
    if (!this.#destroyed) this.#inner.evictLayouts();
  }

  /** Frees the WASM-side presentation and uninstalls any embedded fonts. */
  destroy(): void {
    if (this.#destroyed) return;
    this.#destroyed = true;
    for (const face of this.#installedFonts) {
      try {
        document.fonts.delete(face);
      } catch {
        // A font that was never added, or a document that has gone away.
      }
    }
    this.#installedFonts = [];
    this.#assetListeners.clear();
    this.#inner.free();
  }

  #valid(index: number): boolean {
    return Number.isInteger(index) && index >= 0 && index < this.slideCount;
  }

  /**
   * Waits for the faces a slide draws with.
   *
   * `fillText` against a face the browser has not loaded silently substitutes it, and the
   * result looks exactly like a layout bug — so this is awaited before the first frame
   * rather than treated as an optimisation.
   */
  async #ensureFonts(index: number): Promise<void> {
    if (typeof document === 'undefined' || !document.fonts) return;
    const fonts = parseJson<string[]>(this.#inner.fontsNeeded(index), []);
    const toLoad = fonts.filter((f) => !this.#fontsLoaded.has(f));
    if (toLoad.length === 0) return;
    for (const f of toLoad) this.#fontsLoaded.add(f);
    await Promise.all(
      toLoad.map((f) =>
        // A face that is genuinely unavailable rejects; the fallback chain in the font
        // spec already covers that case, so it is not an error.
        document.fonts.load(f).catch(() => undefined),
      ),
    );
  }

  /**
   * Kicks off decoding for a slide's images.
   *
   * Returns the ids that were not ready. Decoding is deliberately not awaited: a slide
   * that is mostly text should appear immediately rather than after its photographs.
   */
  #startDecoding(index: number): number[] {
    const raw = this.#inner.pendingImages(index);
    if (!raw) return [];
    const ids = raw
      .split(',')
      .map((s) => Number.parseInt(s, 10))
      .filter((n) => Number.isInteger(n) && !this.#decoding.has(n));

    for (const id of ids) {
      this.#decoding.add(id);
      void this.#decodeImage(id);
    }
    return ids;
  }

  async #decodeImage(id: number): Promise<void> {
    try {
      const bytes = this.#inner.imageBytes(id);
      const mime = this.#inner.imageMime(id);
      if (!bytes) {
        this.#inner.markImageFailed(id);
        return;
      }
      // EMF/WMF are vector metafiles no browser decodes. Marking them failed stops the
      // viewer asking again every frame.
      if (mime === 'image/x-emf' || mime === 'image/x-wmf') {
        this.#inner.markImageFailed(id);
        return;
      }
      // Copy out of the wasm heap before handing the bytes to the platform: growing the
      // heap detaches every view into it, and an in-flight decode would see a zero-length
      // buffer. The copy is also what makes the `Uint8Array` a plain `ArrayBuffer` view.
      const blob = new Blob([copyBytes(bytes)], { type: mime });
      const image = await decodeBlob(blob);
      if (this.#destroyed) return;
      this.#inner.setImage(id, image);
    } catch (e) {
      console.warn(`[pptx-viewer] image ${id} could not be decoded`, e);
      if (!this.#destroyed) this.#inner.markImageFailed(id);
    } finally {
      this.#decoding.delete(id);
      // Only once the whole batch is in: firing per image would redraw the slide once
      // per picture, which on an image-heavy deck is worse than waiting.
      if (this.#decoding.size === 0 && !this.#destroyed) this.#notifyAssetsReady();
    }
  }

  async #installEmbeddedFonts(): Promise<void> {
    if (typeof document === 'undefined' || !document.fonts || !('FontFace' in globalThis)) {
      return;
    }
    for (const font of this.#info.embeddedFonts) {
      for (const variant of font.variants) {
        const bytes = this.#inner.embeddedFontBytes(variant.rel);
        if (!bytes) continue;
        try {
          const face = new FontFace(font.typeface, copyBytes(bytes), {
            weight: variant.bold ? '700' : '400',
            style: variant.italic ? 'italic' : 'normal',
          });
          await face.load();
          document.fonts.add(face);
          this.#installedFonts.push(face);
        } catch (e) {
          // A single unusable variant should not stop the others installing.
          console.warn(`[pptx-viewer] embedded font ${font.typeface} could not be used`, e);
        }
      }
    }
  }
}

/**
 * Copies bytes out of the WASM heap into a standalone `ArrayBuffer`.
 *
 * Necessary rather than merely tidy: any later WASM allocation can grow the heap, and
 * growing it detaches every `Uint8Array` view into it. A `Blob` or `FontFace` holding a
 * detached view sees zero bytes.
 */
function copyBytes(bytes: Uint8Array): ArrayBuffer {
  const buffer = new ArrayBuffer(bytes.byteLength);
  new Uint8Array(buffer).set(bytes);
  return buffer;
}

/** `createImageBitmap` where available, an `<img>` otherwise (Safari < 17 for SVG). */
async function decodeBlob(blob: Blob): Promise<ImageBitmap | HTMLImageElement> {
  if (typeof createImageBitmap === 'function' && blob.type !== 'image/svg+xml') {
    try {
      return await createImageBitmap(blob);
    } catch {
      // Fall through to the <img> path, which handles a few formats createImageBitmap
      // refuses.
    }
  }
  const url = URL.createObjectURL(blob);
  try {
    const img = new Image();
    img.decoding = 'sync';
    await new Promise<void>((resolve, reject) => {
      img.onload = () => resolve();
      img.onerror = () => reject(new Error('image failed to load'));
      img.src = url;
    });
    // `decode()` guarantees the bitmap is ready before the next draw; without it the
    // first frame after loading can draw nothing.
    if (typeof img.decode === 'function') await img.decode().catch(() => undefined);
    return img;
  } finally {
    // Revoking immediately is safe: the image has been decoded into memory by now.
    URL.revokeObjectURL(url);
  }
}

async function toBytes(
  src: string | File | Blob | ArrayBuffer | Uint8Array,
  signal?: AbortSignal,
): Promise<Uint8Array> {
  if (src instanceof Uint8Array) return src;
  if (src instanceof ArrayBuffer) return new Uint8Array(src);
  if (typeof Blob !== 'undefined' && src instanceof Blob) {
    return new Uint8Array(await src.arrayBuffer());
  }
  if (typeof src === 'string') {
    const response = await fetch(src, { signal });
    if (!response.ok) {
      throw new PptxError(`could not fetch ${src}: ${response.status} ${response.statusText}`);
    }
    return new Uint8Array(await response.arrayBuffer());
  }
  throw new PptxError('src must be a URL, File, Blob, ArrayBuffer or Uint8Array');
}

function parseJson<T>(text: string, fallback: T): T {
  try {
    return JSON.parse(text) as T;
  } catch {
    return fallback;
  }
}

function errorText(e: unknown): string {
  if (typeof e === 'string') return e;
  if (e instanceof Error) return e.message;
  return String(e);
}
