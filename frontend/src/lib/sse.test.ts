import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { resubscribeDelay, subscribeToJob } from './sse';

/** A stand-in EventSource the test drives by hand. */
class FakeEventSource {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSED = 2;
  static instances: FakeEventSource[] = [];

  readyState = FakeEventSource.OPEN;
  closeCalls = 0;
  private listeners = new Map<string, ((event: Event) => void)[]>();

  constructor(readonly url: string) {
    FakeEventSource.instances.push(this);
  }

  addEventListener(type: string, listener: (event: Event) => void) {
    const list = this.listeners.get(type) ?? [];
    list.push(listener);
    this.listeners.set(type, list);
  }

  close() {
    this.closeCalls += 1;
    this.readyState = FakeEventSource.CLOSED;
  }

  emit(type: string, data?: string) {
    const event = Object.assign(new Event(type), data === undefined ? {} : { data });
    for (const listener of this.listeners.get(type) ?? []) listener(event);
  }

  /** What the browser does when a reconnect is answered with a non-200. */
  giveUp() {
    this.readyState = FakeEventSource.CLOSED;
    this.emit('error');
  }
}

beforeEach(() => {
  FakeEventSource.instances = [];
  vi.stubGlobal('EventSource', FakeEventSource);
  vi.spyOn(console, 'error').mockImplementation(() => undefined);
  vi.spyOn(console, 'debug').mockImplementation(() => undefined);
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe('F-SSE-1 decoding', () => {
  it("opens the job's stream and calls back with the decoded payload", () => {
    const onUpdate = vi.fn();
    subscribeToJob('job-42', onUpdate);
    expect(FakeEventSource.instances).toHaveLength(1);
    const source = FakeEventSource.instances[0];
    expect(source.url).toBe('/api/jobs/job-42/stream');

    source.emit('stats', JSON.stringify({ tasks_completed: 7, games: { wins: 3 } }));
    expect(onUpdate).toHaveBeenCalledTimes(1);
    expect(onUpdate).toHaveBeenCalledWith({ tasks_completed: 7, games: { wins: 3 } });

    source.emit('stats', JSON.stringify({ tasks_completed: 8 }));
    expect(onUpdate).toHaveBeenLastCalledWith({ tasks_completed: 8 });
  });
});

describe('F-SSE-2 malformed events', () => {
  it('skips a malformed event without tearing down the subscription', () => {
    const onUpdate = vi.fn();
    subscribeToJob('j', onUpdate);
    const source = FakeEventSource.instances[0];

    expect(() => source.emit('stats', '{not json')).not.toThrow();
    expect(onUpdate).not.toHaveBeenCalled();
    expect(source.closeCalls).toBe(0);

    // The same stream still delivers the next good event.
    source.emit('stats', '{"tasks_completed":1}');
    expect(onUpdate).toHaveBeenCalledWith({ tasks_completed: 1 });
    expect(FakeEventSource.instances).toHaveLength(1);
  });

  it('an exception thrown by the handler does not escape the listener either', () => {
    const onUpdate = vi.fn(() => {
      throw new Error('handler bug');
    });
    subscribeToJob('j', onUpdate);
    const source = FakeEventSource.instances[0];
    expect(() => source.emit('stats', '{}')).not.toThrow();
    expect(source.closeCalls).toBe(0);
  });
});

describe('F-SSE-3 unsubscribe', () => {
  it('closes the EventSource, and calling it twice is safe', () => {
    const unsubscribe = subscribeToJob('j', vi.fn());
    const source = FakeEventSource.instances[0];
    unsubscribe();
    expect(source.closeCalls).toBe(1);
    expect(source.readyState).toBe(FakeEventSource.CLOSED);
    expect(() => unsubscribe()).not.toThrow();
    // Nothing new was opened by either call.
    expect(FakeEventSource.instances).toHaveLength(1);
  });

  it('a stream the browser gave up on is reopened after a pause', async () => {
    const onUpdate = vi.fn();
    vi.stubGlobal('fetch', vi.fn(async () => ({ status: 200 })));
    const unsubscribe = subscribeToJob('j', onUpdate);
    FakeEventSource.instances[0].giveUp();
    expect(FakeEventSource.instances).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(5000);
    expect(FakeEventSource.instances).toHaveLength(2);

    FakeEventSource.instances[1].emit('stats', '{"n":2}');
    expect(onUpdate).toHaveBeenCalledWith({ n: 2 });

    // Unsubscribing closes the current stream, not the dead one.
    unsubscribe();
    expect(FakeEventSource.instances[1].closeCalls).toBe(1);
  });

  it('a job that is gone is not subscribed to again', async () => {
    // A deleted job's stream closes like a deployment's; asked, the job
    // answers 404, and the page stops asking every five seconds.
    const fetch = vi.fn(async () => ({ status: 404 }));
    vi.stubGlobal('fetch', fetch);
    subscribeToJob('gone', vi.fn());
    FakeEventSource.instances[0].giveUp();
    await vi.advanceTimersByTimeAsync(60_000);
    expect(fetch).toHaveBeenCalledWith('/api/jobs/gone', { method: 'GET' });
    expect(FakeEventSource.instances).toHaveLength(1);
  });

  it('a transient error (still reconnecting) does not open a second stream', () => {
    subscribeToJob('j', vi.fn());
    FakeEventSource.instances[0].readyState = FakeEventSource.CONNECTING;
    FakeEventSource.instances[0].emit('error');
    vi.advanceTimersByTime(60_000);
    expect(FakeEventSource.instances).toHaveLength(1);
  });

  it('unsubscribing while a reopen is pending cancels it', () => {
    const unsubscribe = subscribeToJob('j', vi.fn());
    FakeEventSource.instances[0].giveUp();
    unsubscribe();
    unsubscribe();
    vi.advanceTimersByTime(60_000);
    expect(FakeEventSource.instances).toHaveLength(1);
  });

  it('an error after unsubscribing does not reopen the stream', () => {
    const unsubscribe = subscribeToJob('j', vi.fn());
    unsubscribe();
    FakeEventSource.instances[0].giveUp();
    vi.advanceTimersByTime(60_000);
    expect(FakeEventSource.instances).toHaveLength(1);
  });
});

describe('F-SSE-4 backoff', () => {
  it('doubles from five seconds to a minute, jittered down by at most half', () => {
    expect([0, 1, 2, 3, 4, 10].map((n) => resubscribeDelay(n, () => 1))).toEqual([
      5000, 10_000, 20_000, 40_000, 60_000, 60_000
    ]);
    expect(resubscribeDelay(0, () => 0)).toBe(2500);
    expect(resubscribeDelay(4, () => 0)).toBe(30_000);
  });

  it('a stream refused again and again is asked less often, and an event resets it', async () => {
    vi.spyOn(Math, 'random').mockReturnValue(1);
    vi.stubGlobal('fetch', vi.fn(async () => ({ status: 200 })));
    subscribeToJob('busy', vi.fn());
    FakeEventSource.instances[0].giveUp();
    await vi.advanceTimersByTimeAsync(5000);
    expect(FakeEventSource.instances).toHaveLength(2);

    FakeEventSource.instances[1].giveUp();
    await vi.advanceTimersByTimeAsync(9_999);
    expect(FakeEventSource.instances).toHaveLength(2);
    await vi.advanceTimersByTimeAsync(1);
    expect(FakeEventSource.instances).toHaveLength(3);

    // Working again: the next failure waits five seconds, not twenty.
    FakeEventSource.instances[2].emit('stats', '{}');
    FakeEventSource.instances[2].giveUp();
    await vi.advanceTimersByTimeAsync(5000);
    expect(FakeEventSource.instances).toHaveLength(4);
  });
});
