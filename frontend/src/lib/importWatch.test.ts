import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiError, type ImportDetail } from './api';
import { createImportWatcher } from './importWatch';

const detail = (id: string, state: ImportDetail['state']): ImportDetail =>
  ({ id, state, files: [] }) as unknown as ImportDetail;

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

function harness() {
  const reads: { id: string; answer: ReturnType<typeof deferred<ImportDetail>> }[] = [];
  const states: ImportDetail[] = [];
  const errors: unknown[] = [];
  const hooks = {
    read: vi.fn((id: string) => {
      const answer = deferred<ImportDetail>();
      reads.push({ id, answer });
      return answer.promise;
    }),
    onState: vi.fn((d: ImportDetail) => states.push(d)),
    onError: vi.fn((e: unknown) => errors.push(e)),
    forget: vi.fn(),
    signedOut: vi.fn()
  };
  return { hooks, reads, states, errors, watcher: createImportWatcher(hooks, 1000) };
}

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe('F-IMPORT-1 import polling', () => {
  it('reads at once, polls while running, and stops (keeping the id) once staged', async () => {
    const { hooks, reads, states, watcher } = harness();
    watcher.watch('a');
    expect(reads).toHaveLength(1);
    reads[0].answer.resolve(detail('a', 'running'));
    await vi.advanceTimersByTimeAsync(1000);
    expect(reads).toHaveLength(2);
    reads[1].answer.resolve(detail('a', 'staged'));
    await vi.advanceTimersByTimeAsync(0);
    expect(states.map((s) => s.state)).toEqual(['running', 'staged']);
    expect(watcher.polling).toBe(false);
    expect(hooks.forget).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(5000);
    expect(reads).toHaveLength(2);
  });

  it('forgets a failed import', async () => {
    const { hooks, reads, watcher } = harness();
    watcher.watch('a');
    reads[0].answer.resolve(detail('a', 'failed'));
    await vi.advanceTimersByTimeAsync(0);
    expect(hooks.forget).toHaveBeenCalledOnce();
    expect(watcher.polling).toBe(false);
  });

  it("a late `running` cannot undo a newer `staged` within one watch", async () => {
    const { reads, states, watcher } = harness();
    watcher.watch('a');
    await vi.advanceTimersByTimeAsync(1000); // a second read while the first is slow
    reads[1].answer.resolve(detail('a', 'staged'));
    await vi.advanceTimersByTimeAsync(0);
    reads[0].answer.resolve(detail('a', 'running'));
    await vi.advanceTimersByTimeAsync(0);
    expect(states.map((s) => s.state)).toEqual(['staged']);
    expect(watcher.polling).toBe(false);
  });

  it("an earlier import's late answer cannot touch a newer watch", async () => {
    const { hooks, reads, states, watcher } = harness();
    watcher.watch('a');
    watcher.watch('b');
    expect(reads.map((r) => r.id)).toEqual(['a', 'b']);
    reads[0].answer.resolve(detail('a', 'failed'));
    await vi.advanceTimersByTimeAsync(0);
    expect(states).toEqual([]);
    expect(hooks.forget).not.toHaveBeenCalled();
    expect(watcher.polling).toBe(true);
    reads[1].answer.resolve(detail('b', 'running'));
    await vi.advanceTimersByTimeAsync(0);
    expect(states.map((s) => s.id)).toEqual(['b']);
  });

  it('keeps asking through a 503 or a network failure', async () => {
    const { hooks, reads, errors, watcher } = harness();
    watcher.watch('a');
    reads[0].answer.reject(new ApiError(503, 'unavailable', 'deploying'));
    await vi.advanceTimersByTimeAsync(0);
    expect(errors).toHaveLength(1);
    expect(watcher.polling).toBe(true);
    await vi.advanceTimersByTimeAsync(1000);
    reads[1].answer.reject(new TypeError('fetch failed'));
    await vi.advanceTimersByTimeAsync(0);
    expect(watcher.polling).toBe(true);
    expect(hooks.forget).not.toHaveBeenCalled();
  });

  it('forgets a 404, but keeps the import through a 401 and says the session lapsed', async () => {
    const gone = harness();
    gone.watcher.watch('a');
    gone.reads[0].answer.reject(new ApiError(404, 'not_found', 'no such import'));
    await vi.advanceTimersByTimeAsync(0);
    expect(gone.hooks.forget).toHaveBeenCalledOnce();
    expect(gone.watcher.polling).toBe(false);

    const lapsed = harness();
    lapsed.watcher.watch('a');
    lapsed.reads[0].answer.reject(new ApiError(401, 'unauthorized', 'sign in again'));
    await vi.advanceTimersByTimeAsync(0);
    expect(lapsed.hooks.forget).not.toHaveBeenCalled();
    expect(lapsed.hooks.signedOut).toHaveBeenCalledOnce();
    expect(lapsed.watcher.polling).toBe(false);
  });

  it("an older read's error after a newer success is ignored", async () => {
    const { errors, reads, watcher } = harness();
    watcher.watch('a');
    await vi.advanceTimersByTimeAsync(1000);
    reads[1].answer.resolve(detail('a', 'running'));
    await vi.advanceTimersByTimeAsync(0);
    reads[0].answer.reject(new ApiError(403, 'forbidden', 'stale'));
    await vi.advanceTimersByTimeAsync(0);
    expect(errors).toEqual([]);
    expect(watcher.polling).toBe(true);
  });

  it('a watch after stop starts nothing', async () => {
    const { reads, watcher } = harness();
    watcher.stop();
    watcher.watch('late');
    await vi.advanceTimersByTimeAsync(5000);
    expect(reads).toEqual([]);
    expect(watcher.polling).toBe(false);
  });

  it('stop ignores answers still in flight', async () => {
    const { states, reads, watcher } = harness();
    watcher.watch('a');
    watcher.stop();
    reads[0].answer.resolve(detail('a', 'staged'));
    await vi.advanceTimersByTimeAsync(0);
    expect(states).toEqual([]);
    expect(watcher.polling).toBe(false);
  });
});
