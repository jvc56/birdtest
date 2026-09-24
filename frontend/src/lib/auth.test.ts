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
  fetchMock = vi.fn();
  vi.stubGlobal('fetch', fetchMock);
  vi.stubGlobal('document', { cookie: 'birdtest_csrf=t' });
});

afterEach(() => {
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

  // Deliberate: any failure resolves the store to null, not only a 401. Left
  // undefined, the layout guards that wait on "not yet known" would wait for
  // ever during an outage; null sends a signed-in user to the login page
  // instead, which is the recoverable failure of the two.
  it('resolves to null on a server error and on a network failure, never staying unknown', async () => {
    const { session, refreshSession } = await freshAuth();
    fetchMock.mockResolvedValueOnce(json(503, { code: 'unavailable', message: 'busy' }));
    await expect(refreshSession()).resolves.toBeNull();
    expect(get(session)).toBeNull();

    const again = await freshAuth();
    fetchMock.mockRejectedValueOnce(new TypeError('Failed to fetch'));
    await expect(again.refreshSession()).resolves.toBeNull();
    expect(get(again.session)).toBeNull();
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

  it('clears the store even if the network is down', async () => {
    const { session, refreshSession, signOut } = await freshAuth();
    fetchMock.mockResolvedValueOnce(json(200, ME));
    await refreshSession();
    fetchMock.mockRejectedValueOnce(new TypeError('Failed to fetch'));
    await expect(signOut()).rejects.toThrow('Failed to fetch');
    expect(get(session)).toBeNull();
  });
});
