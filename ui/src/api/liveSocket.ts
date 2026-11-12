// The WebSocket the live state arrives over. It opens at the page's own address (the access token
// is in the path, as for every endpoint), sends nothing, checks every frame with the live guard,
// and reports what it knows to a sink: each accepted frame, and the moment the connection is lost.
// A lost connection is opened again after a delay that starts at one second and doubles to thirty.
// A frame the guard refuses is dropped with a console warning; it never stops the page.
import { LIVE } from '../theme/tokens';
import { LIVE_URL, parseLive, type LiveState } from './live';

/** Where the socket reports to. */
export interface LiveSink {
  /** A frame passed the guard: this is now the whole live state. */
  readonly onState: (state: LiveState) => void;
  /** The connection closed or could not open; another attempt is scheduled. */
  readonly onPaused: () => void;
}

/** The part of a WebSocket the connection uses, so tests can stand in for one. */
export interface SocketLike {
  onopen: ((event: Event) => void) | null;
  onmessage: ((event: MessageEvent) => void) | null;
  onclose: ((event: CloseEvent) => void) | null;
  close: () => void;
}

/** What the connection needs from the page: how to open a socket and how to wait. */
export interface LiveEnvironment {
  readonly open: (url: string) => SocketLike;
  readonly setTimer: (run: () => void, delayMs: number) => number;
  readonly clearTimer: (timer: number) => void;
  readonly warn: (message: string) => void;
}

/**
 * How long to wait before reconnect attempt `attempt` (0 for the first after a loss): one second,
 * doubling each time, never more than thirty.
 */
export function reconnectDelayMs(attempt: number): number {
  const delay = LIVE.reconnectFirstMs * 2 ** Math.max(0, attempt);
  return Math.min(delay, LIVE.reconnectMaxMs);
}

/** The socket address for `api/live` under `baseUri`: the same host and path, as `ws:` or `wss:`. */
export function liveSocketUrl(baseUri: string): string {
  // INVARIANT: local-only (the page's own address, so only the daemon that served it is asked)
  const url = new URL(LIVE_URL, baseUri);
  url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:';
  return url.href;
}

/** Parses one text frame; a frame that is not text, not JSON or not the contract is refused. */
function readFrame(data: unknown): LiveState | string {
  if (typeof data !== 'string') return 'the frame is not text';
  let value: unknown;
  try {
    value = JSON.parse(data);
  } catch {
    return 'the frame is not JSON';
  }
  const result = parseLive(value);
  return result.ok ? result.value : `${result.error.code}: ${result.error.message}`;
}

/**
 * Connects to `url` and keeps reconnecting until the returned function is called, which closes the
 * socket and cancels any attempt waiting to run.
 */
export function connectLive(url: string, sink: LiveSink, environment: LiveEnvironment): () => void {
  let isStopped = false;
  let attempt = 0;
  let socket: SocketLike | null = null;
  let timer: number | null = null;

  const connect = (): void => {
    timer = null;
    const opened = environment.open(url);
    socket = opened;
    opened.onopen = () => {
      attempt = 0;
    };
    opened.onmessage = (event) => {
      const frame = readFrame(event.data);
      if (typeof frame === 'string') environment.warn(`A live frame was dropped (${frame}).`);
      else sink.onState(frame);
    };
    opened.onclose = () => {
      socket = null;
      if (isStopped) return;
      sink.onPaused();
      timer = environment.setTimer(connect, reconnectDelayMs(attempt));
      attempt += 1;
    };
  };

  connect();
  return () => {
    isStopped = true;
    if (timer !== null) environment.clearTimer(timer);
    const open = socket;
    socket = null;
    if (open === null) return;
    open.onclose = null;
    open.onmessage = null;
    open.close();
  };
}

/** The page's own environment: real sockets and timers, warnings to the console. */
export const BROWSER_LIVE_ENVIRONMENT: LiveEnvironment = {
  open: (url) => new WebSocket(url),
  setTimer: (run, delayMs) => window.setTimeout(run, delayMs),
  clearTimer: (timer) => {
    window.clearTimeout(timer);
  },
  warn: (message) => {
    console.warn(message);
  },
};
