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
  const source = new EventSource(`/api/jobs/${jobId}/stream`);

  source.addEventListener('stats', (event) => {
    try {
      onUpdate(JSON.parse((event as MessageEvent).data) as T);
    } catch (error) {
      console.error('could not parse SSE payload', error);
    }
  });

  // EventSource reconnects on its own; this only fires for the surfaced error.
  source.addEventListener('error', () => console.debug('job stream interrupted; retrying'));

  return () => source.close();
}
