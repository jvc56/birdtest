import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { api, ApiError, errorText } from './api';

/**
 * lib/api.ts is under test here, not fetch: fetch and document.cookie are
 * stubbed, and each test inspects what the wrapper asked for and what it made
 * of the answer.
 */
let fetchMock: ReturnType<typeof vi.fn>;

function respond(status: number, body?: string, statusText = '') {
  fetchMock.mockResolvedValueOnce(
    new Response(status === 204 ? null : body ?? '', {
      status,
      statusText,
      headers: body && body.startsWith('{') ? { 'content-type': 'application/json' } : {}
    })
  );
}

function lastInit(): RequestInit {
  return fetchMock.mock.calls.at(-1)![1] as RequestInit;
}

function lastHeaders(): Record<string, string> {
  return (lastInit().headers ?? {}) as Record<string, string>;
}

beforeEach(() => {
  fetchMock = vi.fn();
  vi.stubGlobal('fetch', fetchMock);
  vi.stubGlobal('document', {
    cookie: 'theme=dark; birdtest_csrf=tok%2Fen-123; other=x'
  });
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('F-API-1 CSRF header', () => {
  it('a POST sends x-csrf-token read (and decoded) from the cookie', async () => {
    respond(200, '{"message":"ok"}');
    await api.confirmEmail('abc');
    expect(lastInit().method).toBe('POST');
    expect(lastHeaders()['x-csrf-token']).toBe('tok/en-123');
  });

  it('PATCH and DELETE send it too', async () => {
    respond(204);
    await api.setApiKeyActive('k1', false);
    expect(lastInit().method).toBe('PATCH');
    expect(lastHeaders()['x-csrf-token']).toBe('tok/en-123');

    respond(204);
    await api.revokeApiKey('k1');
    expect(lastInit().method).toBe('DELETE');
    expect(lastHeaders()['x-csrf-token']).toBe('tok/en-123');
  });

  it('a GET does not', async () => {
    respond(200, '{"items":[],"total":0,"page":0,"per_page":50}');
    await api.jobs();
    expect(lastInit().method).toBe('GET');
    expect(lastHeaders()).not.toHaveProperty('x-csrf-token');
  });

  it('does not match a cookie whose name merely ends in birdtest_csrf', async () => {
    vi.stubGlobal('document', { cookie: 'not_birdtest_csrf=wrong' });
    respond(204);
    await api.logout();
    expect(lastHeaders()['x-csrf-token']).toBe('');
  });
});

describe('F-API-2 204 No Content', () => {
  it('resolves to undefined rather than throwing on the empty body', async () => {
    respond(204);
    await expect(api.logout()).resolves.toBeUndefined();
  });

  it('a 200 with an empty body also resolves to undefined', async () => {
    respond(200, '');
    await expect(api.deleteJob('j1')).resolves.toBeUndefined();
  });
});

describe('F-API-3 JSON error body', () => {
  it('rejects with an ApiError carrying status, code, message and fields', async () => {
    respond(
      422,
      JSON.stringify({
        code: 'validation',
        message: 'Some fields are invalid.',
        fields: [
          { field: 'username', message: 'Already taken.' },
          { field: 'email', message: 'Not an email address.' }
        ]
      }),
      'Unprocessable Entity'
    );
    const error = await api
      .register({ username: 'a', email: 'b', password: 'c' })
      .catch((e: unknown) => e);
    expect(error).toBeInstanceOf(ApiError);
    const apiError = error as ApiError;
    expect(apiError.status).toBe(422);
    expect(apiError.code).toBe('validation');
    expect(apiError.message).toBe('Some fields are invalid.');
    expect(apiError.fields).toEqual({
      username: 'Already taken.',
      email: 'Not an email address.'
    });
  });

  it('an error body without fields yields an empty field map', async () => {
    respond(409, JSON.stringify({ code: 'conflict', message: 'Job is active.' }));
    const error = (await api.deleteJob('j1').catch((e: unknown) => e)) as ApiError;
    expect(error).toBeInstanceOf(ApiError);
    expect(error.status).toBe(409);
    expect(error.code).toBe('conflict');
    expect(error.fields).toEqual({});
  });
});

describe('F-API-4 error without a JSON body', () => {
  it('an empty 4xx body rejects with an ApiError, falling back to the status text', async () => {
    respond(404, '', 'Not Found');
    const error = await api.job('missing').catch((e: unknown) => e);
    expect(error).toBeInstanceOf(ApiError);
    expect(error).not.toBeInstanceOf(SyntaxError);
    expect((error as ApiError).status).toBe(404);
    expect((error as ApiError).code).toBe('error');
    expect((error as ApiError).message).toBe('Not Found');
  });

  it('a non-JSON 4xx/5xx body rejects with an ApiError, not a SyntaxError', async () => {
    respond(502, '<html><body>Bad Gateway</body></html>', 'Bad Gateway');
    const error = await api.job('j1').catch((e: unknown) => e);
    expect(error).toBeInstanceOf(ApiError);
    expect(error).not.toBeInstanceOf(SyntaxError);
    expect((error as ApiError).status).toBe(502);
    expect((error as ApiError).message).toBe('Bad Gateway');

    respond(400, 'plain text, not JSON', 'Bad Request');
    const second = await api.login({ username: 'a', password: 'b' }).catch((e: unknown) => e);
    expect(second).toBeInstanceOf(ApiError);
    expect((second as ApiError).status).toBe(400);
  });

  it('says the status when there is neither a body nor a status text (HTTP/2)', async () => {
    // Over HTTP/2 statusText is always empty, and the load balancer's own 503
    // page is HTML: the error must still say something a page can show.
    respond(503, '<html><body>Service Unavailable</body></html>', '');
    const error = await api.job('j1').catch((e: unknown) => e);
    expect(error).toBeInstanceOf(ApiError);
    expect((error as ApiError).message).toBe('The server answered 503.');
  });

  it('a 200 whose body is not JSON rejects with an ApiError too', async () => {
    // What a static host's index.html fallback looks like to an /api call.
    respond(200, '<!doctype html><html></html>');
    const error = await api.me().catch((e: unknown) => e);
    expect(error).toBeInstanceOf(ApiError);
    expect(error).not.toBeInstanceOf(SyntaxError);
    expect((error as ApiError).code).toBe('invalid_response');
  });
});

describe('F-API-5 credentials', () => {
  it("every request sets credentials: 'include'", async () => {
    const calls: [string, () => Promise<unknown>][] = [
      ['GET', () => api.me()],
      ['POST', () => api.logout()],
      ['PATCH', () => api.setApiKeyActive('k', true)],
      ['DELETE', () => api.deleteUser('u')]
    ];
    for (const [method, call] of calls) {
      respond(204);
      await call();
      expect(lastInit().method).toBe(method);
      expect(lastInit().credentials).toBe('include');
    }
    // Error responses are no different on the way out.
    respond(500, '', 'Internal Server Error');
    await api.fleet().catch(() => undefined);
    expect(lastInit().credentials).toBe('include');
    expect(fetchMock).toHaveBeenCalledTimes(5);
  });

  it('a body is sent as JSON with a content-type, and a GET sends none', async () => {
    respond(200, '{"id":"x","state":"running"}');
    await api.startImport({ tarball_date: '2026-01-01' });
    expect(lastHeaders()['content-type']).toBe('application/json');
    expect(JSON.parse(lastInit().body as string)).toEqual({ tarball_date: '2026-01-01' });

    respond(200, '{"id":"x"}');
    await api.job('x');
    expect(lastInit().body).toBeUndefined();
    expect(lastHeaders()).not.toHaveProperty('content-type');
  });
});

describe('F-API-6 errorText', () => {
  it('lists the fields the server named after the message', () => {
    const e = new ApiError(400, 'bad_request', 'import details are invalid', {
      git_ref: "letters, digits, '-', '_', '.' and '/' only"
    });
    expect(errorText(e)).toBe(
      "import details are invalid: git_ref letters, digits, '-', '_', '.' and '/' only"
    );
  });

  it('is the message alone when there are none', () => {
    expect(errorText(new ApiError(409, 'conflict', 'that already exists'))).toBe(
      'that already exists'
    );
    expect(errorText(new Error('offline'))).toBe('offline');
  });
});
