import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { get } from 'svelte/store';

/**
 * lib/auth.ts talks to the API through lib/api.ts, so fetch is the seam: the
 * store's behaviour is checked against real API-layer responses and failures.
 * Each test imports a fresh module so the store starts unresolved.
 */
let fetchMock: ReturnType<typeof vi.fn>;

const ME = {
  id: 'u1',
  username: 'alice',
  email: 'alice@example.com',
  is_admin: false,
  tasks_completed: 12
};

async function freshAuth() {
  vi.resetModules();
  return import('./auth');
}

function json(status: number, body: unknown) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' }
  });
}

beforeEach(() => {
  // Fake: a failed refresh schedules another, which must not fire into a
  // later test's fetch.
  vi.useFakeTimers();
  fetchMock = vi.fn();
  vi.stubGlobal('fetch', fetchMock);
  vi.stubGlobal('document', { cookie: 'birdtest_csrf=t' });
});

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe('F-AUTH-1 refreshSession', () => {
  it('starts as undefined — not yet known — before any request resolves', async () => {
    const { session } = await freshAuth();
    expect(get(session)).toBeUndefined();
  });

  it('sets the store to the user on 200', async () => {
    const { session, refreshSession } = await freshAuth();
    fetchMock.mockResolvedValueOnce(json(200, ME));
    await expect(refreshSession()).resolves.toEqual(ME);
    expect(get(session)).toEqual(ME);
    expect(fetchMock.mock.calls[0][0]).toBe('/api/me');
  });

  it('sets the store to null on 401 — signed out, which is not the same as unknown', async () => {
    const { session, refreshSession } = await freshAuth();
    fetchMock.mockResolvedValueOnce(
      json(401, { code: 'unauthorized', message: 'Not signed in.' })
    );
    await expect(refreshSession()).resolves.toBeNull();
    expect(get(session)).toBeNull();
    expect(get(session)).not.toBeUndefined();
  });

  it('sets the store to null on 403 too', async () => {
    const { session, refreshSession } = await freshAuth();
    fetchMock.mockResolvedValueOnce(json(403, { code: 'forbidden', message: 'No.' }));
    await expect(refreshSession()).resolves.toBeNull();
    expect(get(session)).toBeNull();
  });

  it('stays unknown on a server error or a network failure, and asks again until it is answered', async () => {
    const { session, refreshSession } = await freshAuth();
    fetchMock.mockResolvedValueOnce(json(503, { code: 'unavailable', message: 'busy' }));
    await expect(refreshSession()).resolves.toBeUndefined();
    expect(get(session)).toBeUndefined();
    expect(fetchMock).toHaveBeenCalledTimes(1);

    // Asked again after 2 s, then after twice that.
    fetchMock.mockRejectedValueOnce(new TypeError('Failed to fetch'));
    await vi.advanceTimersByTimeAsync(1_999);
    expect(fetchMock).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(1);
    expect(fetchMock).toHaveBeenCalledTimes(2);
    expect(get(session)).toBeUndefined();

    fetchMock.mockResolvedValueOnce(json(200, ME));
    await vi.advanceTimersByTimeAsync(3_999);
    expect(fetchMock).toHaveBeenCalledTimes(2);
    await vi.advanceTimersByTimeAsync(1);
    expect(fetchMock).toHaveBeenCalledTimes(3);
    expect(get(session)).toEqual(ME);

    // Answered: nothing more is asked.
    await vi.advanceTimersByTimeAsync(60_000);
    expect(fetchMock).toHaveBeenCalledTimes(3);
  });

  it("waits at least the server's Retry-After before asking again", async () => {
    const { session, refreshSession } = await freshAuth();
    fetchMock.mockResolvedValueOnce(
      new Response(JSON.stringify({ code: 'unavailable', message: 'busy' }), {
        status: 503,
        headers: { 'content-type': 'application/json', 'retry-after': '10' }
      })
    );
    await refreshSession();
    fetchMock.mockResolvedValueOnce(json(401, { code: 'unauthorized', message: 'Not signed in.' }));
    await vi.advanceTimersByTimeAsync(9_999);
    expect(fetchMock).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(1);
    expect(fetchMock).toHaveBeenCalledTimes(2);
    expect(get(session)).toBeNull();
  });

  it('a server error keeps a signed-in user signed in, and asks nothing more', async () => {
    const { session, refreshSession } = await freshAuth();
    fetchMock.mockResolvedValueOnce(json(200, ME));
    await refreshSession();
    fetchMock.mockResolvedValueOnce(json(502, { code: 'bad_gateway', message: 'deploying' }));
    await expect(refreshSession()).resolves.toEqual(ME);
    expect(get(session)).toEqual(ME);
    await vi.advanceTimersByTimeAsync(60_000);
    expect(fetchMock).toHaveBeenCalledTimes(2);
  });

  it('stays undefined while the request is in flight', async () => {
    const { session, refreshSession } = await freshAuth();
    let answer!: (response: Response) => void;
    fetchMock.mockReturnValueOnce(new Promise<Response>((resolve) => (answer = resolve)));
    const pending = refreshSession();
    expect(get(session)).toBeUndefined();
    answer(json(200, ME));
    await pending;
    expect(get(session)).toEqual(ME);
  });

  it('a later 401 replaces a signed-in user with null', async () => {
    const { session, refreshSession } = await freshAuth();
    fetchMock.mockResolvedValueOnce(json(200, ME));
    await refreshSession();
    fetchMock.mockResolvedValueOnce(json(401, { code: 'unauthorized', message: 'Expired.' }));
    await refreshSession();
    expect(get(session)).toBeNull();
  });
});

describe('F-AUTH-2 signOut', () => {
  it('clears the store when the request succeeds', async () => {
    const { session, refreshSession, signOut } = await freshAuth();
    fetchMock.mockResolvedValueOnce(json(200, ME));
    await refreshSession();
    fetchMock.mockResolvedValueOnce(new Response(null, { status: 204 }));
    await signOut();
    expect(get(session)).toBeNull();
    const [path, init] = fetchMock.mock.calls[1];
    expect(path).toBe('/api/auth/logout');
    expect((init as RequestInit).method).toBe('POST');
  });

  it('clears the store even if the request fails with an error response', async () => {
    const { session, refreshSession, signOut } = await freshAuth();
    fetchMock.mockResolvedValueOnce(json(200, ME));
    await refreshSession();
    fetchMock.mockResolvedValueOnce(json(500, { code: 'internal', message: 'boom' }));
    await expect(signOut()).rejects.toThrow('boom');
    expect(get(session)).toBeNull();
  });

  it('stops asking /api/me again after a failed refresh', async () => {
    const { session, refreshSession, signOut } = await freshAuth();
    fetchMock.mockResolvedValueOnce(json(503, { code: 'unavailable', message: 'busy' }));
    await refreshSession();
    fetchMock.mockResolvedValueOnce(new Response(null, { status: 204 }));
    await signOut();
    await vi.advanceTimersByTimeAsync(60_000);
    expect(fetchMock).toHaveBeenCalledTimes(2);
    expect(get(session)).toBeNull();
  });

  it('clears the store even if the network is down', async () => {
    const { session, refreshSession, signOut } = await freshAuth();
    fetchMock.mockResolvedValueOnce(json(200, ME));
    await refreshSession();
    fetchMock.mockRejectedValueOnce(new TypeError('Failed to fetch'));
    await expect(signOut()).rejects.toThrow('Failed to fetch');
    expect(get(session)).toBeNull();
  });
});

describe('F-AUTH-3 resetSession', () => {
  it('after a sign-in, a 503 leaves the session unresolved rather than signed out, and the retry finds the user', async () => {
    const { session, refreshSession, resetSession } = await freshAuth();
    // The login page is reached signed out.
    fetchMock.mockResolvedValueOnce(json(401, { code: 'unauthorized', message: 'Not signed in.' }));
    await refreshSession();
    expect(get(session)).toBeNull();

    const seen: unknown[] = [];
    const unsubscribe = session.subscribe((value) => seen.push(value));
    // Signed in; the deploy's 503, then the user two seconds later.
    fetchMock.mockResolvedValueOnce(json(503, { code: 'unavailable', message: 'busy' }));
    await expect(resetSession()).resolves.toBeUndefined();
    expect(get(session)).toBeUndefined();
    fetchMock.mockResolvedValueOnce(json(200, ME));
    await vi.advanceTimersByTimeAsync(2_000);
    expect(get(session)).toEqual(ME);
    unsubscribe();
    // null (subscribed), then unresolved, then the user: never null again.
    expect(seen).toEqual([null, undefined, ME]);
    expect(fetchMock).toHaveBeenCalledTimes(3);
  });

  it('a signed-in user whose session lapsed is asked about until the server answers', async () => {
    const { session, refreshSession, resetSession } = await freshAuth();
    fetchMock.mockResolvedValueOnce(json(200, ME));
    await refreshSession();
    fetchMock.mockResolvedValueOnce(json(502, { code: 'bad_gateway', message: 'deploying' }));
    await resetSession();
    expect(get(session)).toBeUndefined();
    fetchMock.mockResolvedValueOnce(json(401, { code: 'unauthorized', message: 'Expired.' }));
    await vi.advanceTimersByTimeAsync(2_000);
    expect(get(session)).toBeNull();
    await vi.advanceTimersByTimeAsync(60_000);
    expect(fetchMock).toHaveBeenCalledTimes(3);
  });

  it('starts a pending retry over from the first wait', async () => {
    const { session, refreshSession, resetSession } = await freshAuth();
    fetchMock.mockResolvedValueOnce(json(503, { code: 'unavailable', message: 'busy' }));
    await refreshSession();
    fetchMock.mockResolvedValueOnce(json(503, { code: 'unavailable', message: 'busy' }));
    await vi.advanceTimersByTimeAsync(2_000);
    expect(fetchMock).toHaveBeenCalledTimes(2);
    // The next wait would be 4 s; a reset asks now and waits 2 s after a failure.
    fetchMock.mockResolvedValueOnce(json(503, { code: 'unavailable', message: 'busy' }));
    await resetSession();
    expect(fetchMock).toHaveBeenCalledTimes(3);
    fetchMock.mockResolvedValueOnce(json(200, ME));
    await vi.advanceTimersByTimeAsync(2_000);
    expect(fetchMock).toHaveBeenCalledTimes(4);
    expect(get(session)).toEqual(ME);
  });
});
