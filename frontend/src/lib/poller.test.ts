import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createPoller, type Visibility } from './poller';

/** A read the test answers by hand, in any order. */
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

/** A tab the test hides and shows. */
function tab(): Visibility & { hidden: boolean; set: (hidden: boolean) => void; listeners: number } {
  const listeners = new Set<() => void>();
  const t = {
    hidden: false,
    addEventListener: (_: 'visibilitychange', l: () => void) => void listeners.add(l),
    removeEventListener: (_: 'visibilitychange', l: () => void) => void listeners.delete(l),
    set(hidden: boolean) {
      t.hidden = hidden;
      listeners.forEach((l) => l());
    },
    get listeners() {
      return listeners.size;
    }
  };
  return t;
}

/** Busy values poll every 100 ms, idle ones every 1000, a failure every 100. */
function harness(visibility: Visibility | null = null, stopOn: string | null = null) {
  const reads: ReturnType<typeof deferred<string>>[] = [];
  const values: string[] = [];
  const errors: unknown[] = [];
  const poller = createPoller<string>(
    {
      read: vi.fn(() => {
        const answer = deferred<string>();
        reads.push(answer);
        return answer.promise;
      }),
      onValue: (v) => values.push(v),
      onError: (e) => errors.push(e),
      delay: (v, failure) =>
        failure ? (failure.error === stopOn ? null : 100) : v === 'busy' ? 100 : 1000
    },
    visibility
  );
  return { reads, values, errors, poller };
}

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe('F-POLL-1 polling the derived data', () => {
  it('reads at once, fast while busy and slower while idle, and never stops by itself', async () => {
    const { reads, values, poller } = harness();
    poller.start();
    expect(reads).toHaveLength(1);
    reads[0].resolve('busy');
    await vi.advanceTimersByTimeAsync(100);
    expect(reads).toHaveLength(2);
    reads[1].resolve('idle');
    await vi.advanceTimersByTimeAsync(999);
    expect(reads).toHaveLength(2);
    await vi.advanceTimersByTimeAsync(1);
    expect(reads).toHaveLength(3);
    // Something queued while idle is seen.
    reads[2].resolve('busy');
    await vi.advanceTimersByTimeAsync(100);
    expect(reads).toHaveLength(4);
    expect(values).toEqual(['busy', 'idle', 'busy']);
  });

  it('keeps polling through a failure, and stops when told to', async () => {
    const { reads, errors, poller } = harness(null, 'gone');
    poller.start();
    reads[0].reject(new Error('503'));
    await vi.advanceTimersByTimeAsync(100);
    expect(reads).toHaveLength(2);
    reads[1].reject('gone');
    await vi.advanceTimersByTimeAsync(5000);
    expect(reads).toHaveLength(2);
    expect(errors).toHaveLength(2);
    expect(poller.scheduled).toBe(false);
  });

  it('pauses while the tab is hidden and reads at once when it is shown', async () => {
    const t = tab();
    const { reads, values, poller } = harness(t);
    poller.start();
    reads[0].resolve('idle');
    await vi.advanceTimersByTimeAsync(0);
    t.set(true);
    expect(poller.scheduled).toBe(false);
    await vi.advanceTimersByTimeAsync(60_000);
    expect(reads).toHaveLength(1);
    t.set(false);
    expect(reads).toHaveLength(2);
    reads[1].resolve('busy');
    await vi.advanceTimersByTimeAsync(100);
    expect(reads).toHaveLength(3);
    expect(values).toEqual(['idle', 'busy']);
  });

  it('a read in flight when the tab is hidden reports but schedules nothing', async () => {
    const t = tab();
    const { reads, values, poller } = harness(t);
    poller.start();
    t.set(true);
    reads[0].resolve('busy');
    await vi.advanceTimersByTimeAsync(5000);
    expect(values).toEqual(['busy']);
    expect(reads).toHaveLength(1);
  });

  it('does not start reading in a hidden tab', async () => {
    const t = tab();
    t.hidden = true;
    const { reads, poller } = harness(t);
    poller.start();
    expect(reads).toHaveLength(0);
    t.set(false);
    expect(reads).toHaveLength(1);
  });

  it('a refresh overtakes a read in flight, whose late answer is dropped, and runs one chain', async () => {
    const { reads, values, poller } = harness();
    poller.start();
    const refreshed = poller.refresh();
    expect(reads).toHaveLength(2);
    reads[1].resolve('idle');
    await refreshed;
    reads[0].resolve('busy');
    await vi.advanceTimersByTimeAsync(0);
    expect(values).toEqual(['idle']);
    // One timer, the refresh's: nothing at the stale answer's 100 ms.
    await vi.advanceTimersByTimeAsync(999);
    expect(reads).toHaveLength(2);
    await vi.advanceTimersByTimeAsync(1);
    expect(reads).toHaveLength(3);
  });

  it('stopping ignores answers in flight, stops listening, and cannot be restarted', async () => {
    const t = tab();
    const { reads, values, errors, poller } = harness(t);
    poller.start();
    expect(t.listeners).toBe(1);
    poller.stop();
    expect(t.listeners).toBe(0);
    reads[0].resolve('busy');
    await vi.advanceTimersByTimeAsync(5000);
    expect(values).toEqual([]);
    expect(errors).toEqual([]);
    poller.start();
    t.set(false);
    await poller.refresh();
    expect(reads).toHaveLength(1);
  });
});
