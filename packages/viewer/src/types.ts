/**
 * Public types. Everything a consumer of this package can name lives here.
 */

/** How a slide is fitted into the viewport. */
export type Fit =
  /** Scale to fit entirely, preserving aspect ratio, centred. The default. */
  | 'contain'
  /** Fill the viewport, preserving aspect ratio, cropping the overflow. */
  | 'cover'
  /** Ignore aspect ratio and stretch. */
  | 'fill'
  /** 1 slide point = 1 CSS pixel. */
  | 'actual';

/** Where the WASM module is loaded from. */
export type WasmSource =
  /** A URL to `pptx_bg.wasm`. */
  | string
  /** Already-fetched bytes, for bundlers that inline the module. */
  | ArrayBuffer
  | Response;

export interface SlideInfo {
  /** 0-based position in presentation order. */
  index: number;
  /** `<p:sldId id="...">`, stable across reorderings. */
  id: number;
  /** Package part name, useful for diagnostics. */
  part: string;
}

export interface EmbeddedFontVariant {
  rel: string;
  bold: boolean;
  italic: boolean;
}

export interface EmbeddedFont {
  typeface: string;
  variants: EmbeddedFontVariant[];
}

export interface PresentationInfo {
  slideCount: number;
  /** Slide size in points (1/72 inch). */
  width: number;
  height: number;
  slides: SlideInfo[];
  embeddedFonts: EmbeddedFont[];
}

export interface RenderOptions {
  /** Canvas size in CSS pixels. Defaults to the canvas's own client size. */
  width?: number;
  height?: number;
  /** Defaults to `window.devicePixelRatio`. */
  dpr?: number;
  fit?: Fit;
  /** Extra zoom on top of the fit. 1 = none. */
  zoom?: number;
  panX?: number;
  panY?: number;
}

/** Anything that went wrong that the caller might want to know about. */
export class PptxError extends Error {
  constructor(
    message: string,
    readonly cause?: unknown,
  ) {
    super(message);
    this.name = 'PptxError';
  }
}

export interface PresentationViewerProps {
  /** URL, `File`, `Blob`, or raw bytes of the `.pptx`. */
  src: string | File | Blob | ArrayBuffer | Uint8Array;
  /** CSS width of the viewer element. Default `100%`. */
  width?: string | number;
  /** CSS height of the viewer element. Default `100%`. */
  height?: string | number;
  /** 0-based slide to show first. Default 0. */
  initialSlide?: number;
  /** Controlled slide index. When set, the component does not manage its own. */
  slide?: number;
  fit?: Fit;
  zoom?: number;
  /** Respond to arrow keys, Page Up/Down, Home/End. Default true. */
  keyboard?: boolean;
  /** Where to load the WASM module from. Defaults to the bundled one. */
  wasm?: WasmSource;
  /** Rendered instead of the canvas while the deck loads. */
  loading?: React.ReactNode;
  /** Rendered instead of the canvas when loading fails. */
  renderError?: (error: PptxError) => React.ReactNode;
  className?: string;
  style?: React.CSSProperties;
  onLoad?: (info: PresentationInfo) => void;
  onError?: (error: PptxError) => void;
  onSlideChange?: (index: number) => void;
}

/**
 * One run of text on a slide, positioned in device pixels.
 *
 * Everything needed to lay a transparent, correctly-sized element over the canvas so the
 * browser can handle selection, find-in-page and assistive technology — the things a
 * canvas cannot do for itself.
 */
export interface TextLayerRun {
  text: string;
  /** Left end of the baseline, in device pixels. */
  x: number;
  y: number;
  /** Measured advance width in device pixels. */
  width: number;
  /** Font size in device pixels. */
  size: number;
  /** A CSS `font-family` list. */
  family: string;
  weight: number;
  italic: boolean;
  /** Clockwise rotation in radians. */
  rotation: number;
}
