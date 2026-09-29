/**
 * Reading something again and again for as long as a page shows it, faster
 * while it is moving, and not at all while the tab is hidden. For the derived
 * data pages: nothing pushes those rows (the builder is a separate process on
 * a schedule), and a poll that stopped once everything was idle never saw a
 * build queued later.
 *
 * Modelled on `importWatch.ts`, and for the same reason: a read still in
 * flight when the poller was restarted or stopped must not report, or
 * schedule a second chain of reads beside the first.
 */

export interface PollerHooks<T> {
  read: () => Promise<T>;
  /** The newest answer. */
  onValue: (value: T) => void;
  /** The newest read failed. */
  onError: (error: unknown) => void;
  /**
   * How long to wait before reading again, after `value`, or after a read
   * that failed with `failure.error` (which `onError` has just been told,
   * `value` null); null stops polling for good.
   */
  delay: (value: T | null, failure: { error: unknown } | null) => number | null;
}

/** What the poller needs of `document`, so tests can hide and show a tab. */
export interface Visibility {
  readonly hidden: boolean;
  addEventListener(type: 'visibilitychange', listener: () => void): void;
  removeEventListener(type: 'visibilitychange', listener: () => void): void;
}

/** A read every 3 s while something is queued or building. */
export const BUSY_POLL_MS = 3000;
/** And every 10 s otherwise, so a build queued later shows up by itself. */
export const IDLE_POLL_MS = 10000;

export function createPoller<T>(hooks: PollerHooks<T>, visibility: Visibility | null = null) {
  // Every read belongs to one run of reads, and `refresh` or `stop` starts a
  // new one: an older run's answer neither reports nor schedules.
  let generation = 0;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let running = false;
  let stopped = false;

  const clear = () => {
    if (timer !== null) {
      clearTimeout(timer);
      timer = null;
    }
  };

  const halt = () => {
    running = false;
    clear();
    visibility?.removeEventListener('visibilitychange', onVisibility);
  };

  async function tick() {
    clear();
    const me = ++generation;
    let value: T | null = null;
    let error: unknown = null;
    let failed = false;
    try {
      value = await hooks.read();
    } catch (e) {
      error = e;
      failed = true;
    }
    if (me !== generation || !running) return;
    if (failed) hooks.onError(error);
    else hooks.onValue(value as T);
    const wait = hooks.delay(value, failed ? { error } : null);
    if (wait === null) {
      halt();
      return;
    }
    // Hidden: the next read waits for the tab to be shown again.
    if (!visibility?.hidden) timer = setTimeout(tick, wait);
  }

  function onVisibility() {
    if (!running) return;
    if (visibility?.hidden) {
      clear();
      // A read in flight still reports, but schedules nothing.
    } else {
      // Shown again: whatever changed meanwhile, at once.
      tick();
    }
  }

  return {
    /** Reads at once, then as `delay` says, until stopped. */
    start() {
      if (stopped || running) return;
      running = true;
      visibility?.addEventListener('visibilitychange', onVisibility);
      if (!visibility?.hidden) tick();
    },
    /**
     * Reads again now, after an action changed what is read; resolves once
     * that read has reported (or been overtaken).
     */
    refresh(): Promise<void> {
      return running ? tick() : Promise.resolve();
    },
    /** Stops for good; answers still in flight are ignored. */
    stop() {
      stopped = true;
      generation++;
      halt();
    },
    /** Whether a read is scheduled. */
    get scheduled() {
      return timer !== null;
    }
  };
}
