// Tests for the live socket: the reconnect delays (one second doubling to thirty), the socket's
// address, and the connection driven through a stand-in socket and clock: frames reach the sink,
// refused frames are dropped with a warning, a lost connection pauses and comes back after the
// delay, an open resets the delays, and stopping cancels everything.
import { describe, expect, it } from 'vitest';
import {
  connectLive,
  liveSocketUrl,
  reconnectDelayMs,
  type LiveEnvironment,
  type SocketLike,
} from './liveSocket';
import type { LiveState } from './live';

describe('reconnectDelayMs', () => {
  it('waits 1, 2, 4, 8 and 16 seconds, then 30 from the sixth attempt on', () => {
    expect([0, 1, 2, 3, 4, 5, 6, 40].map(reconnectDelayMs)).toEqual([
      1000, 2000, 4000, 8000, 16000, 30000, 30000, 30000,
    ]);
  });
});

describe('liveSocketUrl', () => {
  it('opens api/live under the page, token and all, over ws or wss', () => {
    expect(liveSocketUrl('http://127.0.0.1:47613/0123456789abcdef/')).toBe(
      'ws://127.0.0.1:47613/0123456789abcdef/api/live',
    );
    expect(liveSocketUrl('https://localhost:9/t/')).toBe('wss://localhost:9/t/api/live');
  });
});

/** A socket the test drives by hand. */
class FakeSocket implements SocketLike {
  onopen: ((event: Event) => void) | null = null;
  onmessage: ((event: MessageEvent) => void) | null = null;
  onclose: ((event: CloseEvent) => void) | null = null;
  isClosed = false;

  close(): void {
    this.isClosed = true;
  }

  open(): void {
    this.onopen?.(new Event('open'));
  }

  receive(data: unknown): void {
    this.onmessage?.(new MessageEvent('message', { data }));
  }

  lose(): void {
    this.onclose?.(new CloseEvent('close'));
  }
}

/** A stand-in environment: sockets and timers the test runs by hand, warnings collected. */
function harness() {
  const sockets: FakeSocket[] = [];
  const timers = new Map<number, { run: () => void; delayMs: number }>();
  const warnings: string[] = [];
  let next = 1;
  const environment: LiveEnvironment = {
    open: () => {
      const socket = new FakeSocket();
      sockets.push(socket);
      return socket;
    },
    setTimer: (run, delayMs) => {
      timers.set(next, { run, delayMs });
      return next++;
    },
    clearTimer: (timer) => {
      timers.delete(timer);
    },
    warn: (message) => warnings.push(message),
  };
  const fire = (): number => {
    const [id, timer] = [...timers.entries()][0] ?? [];
    if (id === undefined || timer === undefined) throw new Error('no timer is waiting');
    timers.delete(id);
    timer.run();
    return timer.delayMs;
  };
  return { sockets, timers, warnings, environment, fire };
}

const FRAME = JSON.stringify({ version: 1, sessions: [] });

describe('connectLive', () => {
  it('hands accepted frames to the sink and drops a refused one with a warning', () => {
    const { sockets, warnings, environment } = harness();
    const states: LiveState[] = [];
    connectLive(
      'ws://x/api/live',
      { onState: (s) => states.push(s), onPaused: () => {} },
      environment,
    );
    sockets[0]?.receive(FRAME);
    sockets[0]?.receive(
      '{"version":1,"sessions":[{"id":"a","agent":"claude","title":"C","files":[{"file":1,"mode":"write","last_ms":1}]}]}',
    );
    sockets[0]?.receive('not json');
    expect(states).toEqual([{ version: 1, sessions: [] }]);
    expect(warnings).toEqual([
      'A live frame was dropped (bad_kind: sessions[0].files[0].mode: unknown value "write").',
      'A live frame was dropped (the frame is not JSON).',
    ]);
  });

  it('pauses when the connection is lost and tries again after 1, 2, then 4 seconds', () => {
    const { sockets, environment, fire } = harness();
    let paused = 0;
    connectLive(
      'ws://x/api/live',
      { onState: () => {}, onPaused: () => (paused += 1) },
      environment,
    );
    sockets[0]?.lose();
    expect(paused).toBe(1);
    expect(fire()).toBe(1000);
    sockets[1]?.lose();
    expect(fire()).toBe(2000);
    sockets[2]?.lose();
    expect(fire()).toBe(4000);
    expect(sockets).toHaveLength(4);
  });

  it('starts the delays again at one second once a connection opens', () => {
    const { sockets, environment, fire } = harness();
    connectLive('ws://x/api/live', { onState: () => {}, onPaused: () => {} }, environment);
    sockets[0]?.lose();
    expect(fire()).toBe(1000);
    sockets[1]?.lose();
    expect(fire()).toBe(2000);
    sockets[2]?.open();
    sockets[2]?.lose();
    expect(fire()).toBe(1000);
  });

  it('closes the socket and cancels the waiting attempt when stopped, and pauses no more', () => {
    const { sockets, timers, environment } = harness();
    let paused = 0;
    const stop = connectLive(
      'ws://x/api/live',
      { onState: () => {}, onPaused: () => (paused += 1) },
      environment,
    );
    sockets[0]?.lose();
    expect(timers.size).toBe(1);
    stop();
    expect(timers.size).toBe(0);
    const again = harness();
    const stopOpen = connectLive(
      'ws://x/api/live',
      { onState: () => {}, onPaused: () => (paused += 1) },
      again.environment,
    );
    stopOpen();
    expect(again.sockets[0]?.isClosed).toBe(true);
    again.sockets[0]?.lose();
    expect([paused, again.timers.size]).toEqual([1, 0]);
  });
});
