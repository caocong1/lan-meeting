// WebCodecs API type definitions

interface VideoDecoderConfig {
  codec: string;
  codedWidth?: number;
  codedHeight?: number;
  description?: BufferSource;
  hardwareAcceleration?: "no-preference" | "prefer-hardware" | "prefer-software";
  optimizeForLatency?: boolean;
}

interface VideoDecoderInit {
  output: (frame: VideoFrame) => void;
  error: (error: Error) => void;
}

interface EncodedVideoChunkInit {
  type: "key" | "delta";
  timestamp: number;
  duration?: number;
  data: BufferSource;
}

declare class EncodedVideoChunk {
  constructor(init: EncodedVideoChunkInit);
  readonly type: "key" | "delta";
  readonly timestamp: number;
  readonly duration: number | null;
  readonly byteLength: number;
  copyTo(destination: BufferSource): void;
}

declare class VideoDecoder {
  constructor(init: VideoDecoderInit);
  readonly state: "unconfigured" | "configured" | "closed";
  readonly decodeQueueSize: number;
  configure(config: VideoDecoderConfig): void;
  decode(chunk: EncodedVideoChunk): void;
  flush(): Promise<void>;
  reset(): void;
  close(): void;
  static isConfigSupported(config: VideoDecoderConfig): Promise<{ supported: boolean; config: VideoDecoderConfig }>;
}

interface VideoFrameInit {
  duration?: number;
  timestamp?: number;
  alpha?: "discard" | "keep";
  visibleRect?: DOMRectInit;
  displayWidth?: number;
  displayHeight?: number;
}

declare class VideoFrame {
  constructor(image: CanvasImageSource | BufferSource, init?: VideoFrameInit);
  readonly format: string | null;
  readonly codedWidth: number;
  readonly codedHeight: number;
  readonly codedRect: DOMRectReadOnly | null;
  readonly visibleRect: DOMRectReadOnly | null;
  readonly displayWidth: number;
  readonly displayHeight: number;
  readonly duration: number | null;
  readonly timestamp: number;
  readonly colorSpace: VideoColorSpace;
  metadata(): VideoFrameMetadata;
  allocationSize(options?: VideoFrameCopyToOptions): number;
  copyTo(destination: BufferSource, options?: VideoFrameCopyToOptions): Promise<PlaneLayout[]>;
  clone(): VideoFrame;
  close(): void;
}

interface VideoColorSpace {
  readonly primaries: string | null;
  readonly transfer: string | null;
  readonly matrix: string | null;
  readonly fullRange: boolean | null;
}

interface VideoFrameMetadata {
  [key: string]: unknown;
}

interface VideoFrameCopyToOptions {
  rect?: DOMRectInit;
  layout?: PlaneLayout[];
}

interface PlaneLayout {
  offset: number;
  stride: number;
}
