/**
 * How long to wait before opening a new stream after the browser gave up on
 * the old one. Long enough not to hammer a server that is restarting, short
 * enough that a dashboard is live again soon after it is back.
 */
const RESUBSCRIBE_MS = 5000;

/**
 * The longest wait between attempts. Each attempt that gets no event doubles
 * the wait up to this: a server refusing streams (its cap is full, and the
 * `Retry-After` it sends is a header EventSource never shows us) was asked
 * again, with a stats read first, every five seconds by every open page.
 */
const MAX_RESUBSCRIBE_MS = 60_000;

/**
 * The wait before attempt `failures + 1`: doubling from `RESUBSCRIBE_MS`, and
 * jittered down by up to half so pages refused together do not come back
 * together.
 */
export function resubscribeDelay(failures: number, random: () => number = Math.random): number {
  const ceiling = Math.min(MAX_RESUBSCRIBE_MS, RESUBSCRIBE_MS * 2 ** Math.max(0, failures));
  return Math.round(ceiling * (0.5 + 0.5 * random()));
}

/**
 * Subscribe to a job's live stat stream. The server pushes the same payload
 * `GET /api/jobs/:id` returns after every accepted result, so the handler can
 * simply replace local state rather than merging deltas.
 *
 * Returns the unsubscribe function. Callers subscribe from `onMount` and return
 * this as its cleanup — registering `onDestroy` in here instead would run
 * outside component initialization and throw.
 */
export function subscribeToJob<T>(jobId: string, onUpdate: (stats: T) => void): () => void {
  let source: EventSource | null = null;
  let retry: ReturnType<typeof setTimeout> | null = null;
  let unsubscribed = false;
  // Attempts since the last event; an event means the stream works again.
  let failures = 0;

  const open = async () => {
    retry = null;
    // A stream is closed for good by any answer but a 200 -- a deployment's
    // 503, and also a job that is gone. Asked first, so a deleted job (or a
    // bad id) is not requested every five seconds for as long as the tab
    // stays open; anything else is tried again.
    if (source !== null) {
      const status = await fetch(`/api/jobs/${jobId}`, { method: 'GET' })
        .then((response) => response.status)
        .catch(() => 0);
      if (unsubscribed) return;
      // Any refusal but a timeout or a rate limit will not change by asking
      // again: a deleted job (404), or an id that is not one (a 400 before
      // the API answered those as 404 too).
      if (status >= 400 && status < 500 && status !== 408 && status !== 429) {
        console.debug(`job stream refused (${status}); not subscribing again`);
        return;
      }
    }
    const opened = new EventSource(`/api/jobs/${jobId}/stream`);
    source = opened;

    opened.addEventListener('stats', (event) => {
      failures = 0;
      try {
        onUpdate(JSON.parse((event as MessageEvent).data) as T);
      } catch (error) {
        console.error('could not parse SSE payload', error);
      }
    });

    // EventSource reconnects on its own after a dropped connection -- but only
    // then. A reconnect that is answered with anything but a 200 ends it for
    // good (readyState CLOSED), and that is what every deployment produces: the
    // server ends the stream as it stops, the browser reconnects a few seconds
    // later, and the load balancer answers 503 until the new task is in
    // service. The page then sat there looking live and never updated again.
    // So a stream the browser has given up on is opened afresh, which also
    // re-sends the current stats as its first event.
    opened.addEventListener('error', () => {
      if (opened.readyState === EventSource.CLOSED && !unsubscribed && retry === null) {
        console.debug('job stream closed; subscribing again shortly');
        retry = setTimeout(open, resubscribeDelay(failures));
        failures += 1;
      } else {
        console.debug('job stream interrupted; retrying');
      }
    });
  };
  open();

  return () => {
    unsubscribed = true;
    if (retry !== null) {
      clearTimeout(retry);
      retry = null;
    }
    source?.close();
  };
}
