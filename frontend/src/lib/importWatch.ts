/**
 * Polling an input-data import until it stops running, for the admin import
 * page. Kept apart from the page so its races can be tested: this logic was
 * wrong three audits running.
 */
import { ApiError, type ImportDetail } from './api';

export interface ImportWatchHooks {
  /** Reads an import (`api.getImport`). */
  read: (id: string) => Promise<ImportDetail>;
  /** The newest state of the import being watched. */
  onState: (detail: ImportDetail) => void;
  /** A read failed (the newest one). */
  onError: (error: unknown) => void;
  /** The import is gone, or finished in a way nothing more can be done with. */
  forget: () => void;
  /** The session lapsed (a 401). */
  signedOut: () => void;
}

const status = (e: unknown) => (e instanceof ApiError ? e.status : 0);
/** The import is gone, or the id is not one. */
export const isGone = (e: unknown) => status(e) === 404 || status(e) === 400;
/**
 * A refusal that asking again will not change. A 401 or 403 is about the
 * session, not the import: polling stops, but the import is kept. A deploy's
 * 503, a timeout, a rate limit or a network blip is worth asking again.
 */
export const stopsPolling = (e: unknown) => {
  const s = status(e);
  return s >= 400 && s < 500 && s !== 408 && s !== 429;
};

export function createImportWatcher(hooks: ImportWatchHooks, intervalMs = 1000) {
  // Every read belongs to one watch, and a newer watch makes every older one's
  // answers stale: none of them may report, stop the poll or forget the id. (A
  // late tick of an earlier import put that import on the page, cleared the new
  // one's poll and forgot its id, and Insert then confirmed the wrong import.)
  let generation = 0;
  let poll: ReturnType<typeof setInterval> | null = null;

  const clear = () => {
    if (poll !== null) {
      clearInterval(poll);
      poll = null;
    }
  };

  return {
    /** Watches `id`, reading it at once and then every interval. */
    watch(id: string) {
      clear();
      const me = ++generation;
      const live = () => me === generation;
      // Within a watch, reads can overlap (a slow one, then the next tick);
      // only the newest answer counts, so a late `running` cannot undo a
      // newer `staged`.
      let asked = 0;
      let applied = 0;
      const tick = async () => {
        const mine = ++asked;
        try {
          const read = await hooks.read(id);
          if (!live() || mine < applied) return;
          applied = mine;
          hooks.onState(read);
          if (read.state !== 'running') {
            clear();
            if (read.state !== 'staged') hooks.forget();
          }
        } catch (e) {
          if (!live() || mine < applied) return;
          hooks.onError(e);
          if (status(e) === 401) hooks.signedOut();
          if (stopsPolling(e)) {
            clear();
            if (isGone(e)) hooks.forget();
          }
        }
      };
      poll = setInterval(tick, intervalMs);
      tick();
    },
    /** Stops watching; answers still in flight are ignored. */
    stop() {
      generation++;
      clear();
    },
    /** Whether a poll is running. */
    get polling() {
      return poll !== null;
    }
  };
}
