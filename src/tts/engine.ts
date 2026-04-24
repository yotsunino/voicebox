/**
 * TTS Engine Interface
 * Defines the contract that all TTS engine implementations must fulfill.
 * Engines are registered at startup and selected by the user in settings.
 */

export interface TTSVoice {
  id: string;
  name: string;
  language: string;
  gender?: 'male' | 'female' | 'neutral';
  preview?: string;
}

export interface TTSSpeakOptions {
  text: string;
  voiceId?: string;
  rate?: number;   // 0.5 – 2.0, default 1.0
  pitch?: number;  // 0.5 – 2.0, default 1.0
  volume?: number; // 0.0 – 1.0, default 1.0
}

export interface TTSEngineCapabilities {
  /** Engine supports streaming audio output */
  streaming: boolean;
  /** Engine supports SSML markup */
  ssml: boolean;
  /** Engine can list available voices */
  voiceListing: boolean;
}

/**
 * Base interface every TTS engine must implement.
 */
export interface TTSEngine {
  /** Unique machine-readable identifier, e.g. "elevenlabs" */
  readonly id: string;
  /** Human-readable display name shown in the UI */
  readonly displayName: string;
  readonly capabilities: TTSEngineCapabilities;

  /**
   * Called once when the engine is first loaded.
   * Use this to validate credentials / warm up connections.
   */
  initialize(): Promise<void>;

  /** Return all voices available for the current credentials. */
  listVoices(): Promise<TTSVoice[]>;

  /**
   * Synthesise speech and return a Buffer containing audio data
   * (PCM or a container format such as MP3/OGG).
   */
  speak(options: TTSSpeakOptions): Promise<Buffer>;

  /** Release any held resources (sockets, timers, …). */
  dispose(): Promise<void>;
}

/**
 * Registry that holds all registered TTS engines for the current session.
 */
export class TTSEngineRegistry {
  private engines = new Map<string, TTSEngine>();
  private activeEngineId: string | null = null;

  register(engine: TTSEngine): void {
    if (this.engines.has(engine.id)) {
      throw new Error(`TTS engine "${engine.id}" is already registered.`);
    }
    this.engines.set(engine.id, engine);
  }

  getAll(): TTSEngine[] {
    return Array.from(this.engines.values());
  }

  get(id: string): TTSEngine {
    const engine = this.engines.get(id);
    if (!engine) {
      throw new Error(`TTS engine "${id}" not found.`);
    }
    return engine;
  }

  setActive(id: string): void {
    this.get(id); // throws if not registered
    this.activeEngineId = id;
  }

  getActive(): TTSEngine {
    if (!this.activeEngineId) {
      throw new Error('No active TTS engine has been set.');
    }
    return this.get(this.activeEngineId);
  }

  async disposeAll(): Promise<void> {
    await Promise.all(Array.from(this.engines.values()).map((e) => e.dispose()));
    this.engines.clear();
    this.activeEngineId = null;
  }
}

/** Singleton registry shared across the main process. */
export const engineRegistry = new TTSEngineRegistry();
