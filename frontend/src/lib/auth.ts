import { get, writable } from 'svelte/store';
import { api, ApiError, type Me } from './api';

/**
 * The signed-in user, or null. `undefined` means "not resolved yet", which is
 * what the layout guards wait on — redirecting on `null` before the first
 * `/api/me` completes would bounce a signed-in user to the login page on every
 * hard refresh.
 */
export const session = writable<Me | null | undefined>(undefined);

/** The first wait before asking `/api/me` again after it could not answer, doubled per attempt. */
const RETRY_FIRST_MS = 2_000;
const RETRY_MAX_MS = 30_000;
let retry: ReturnType<typeof setTimeout> | null = null;
let retryDelay = RETRY_FIRST_MS;

function stopRetrying() {
  if (retry !== null) clearTimeout(retry);
  retry = null;
  retryDelay = RETRY_FIRST_MS;
}

/**
 * Asks the server who is signed in, and returns the session as it then
 * stands. Only a 401 or 403 means signed out. Any other failure — a deploy's
 * 502 or 503, a timeout, a network blip — says nothing about the session
 * cookie, so the store keeps what it had: a signed-in user stays signed in,
 * and an unresolved session stays unresolved (the admin pages wait rather
 * than send a signed-in admin to the login page) and is asked again, with a
 * doubling wait (at least the server's `Retry-After`) until it answers.
 */
export async function refreshSession(): Promise<Me | null | undefined> {
  try {
    const me = await api.me();
    stopRetrying();
    session.set(me);
    return me;
  } catch (e) {
    if (e instanceof ApiError && (e.status === 401 || e.status === 403)) {
      stopRetrying();
      session.set(null);
      return null;
    }
    if (get(session) === undefined && retry === null) {
      const wait = Math.max(retryDelay, ((e instanceof ApiError && e.retryAfter) || 0) * 1000);
      retryDelay = Math.min(retryDelay * 2, RETRY_MAX_MS);
      retry = setTimeout(() => {
        retry = null;
        void refreshSession();
      }, wait);
    }
    return get(session);
  }
}

/**
 * Asks again after something that changes the session: a sign-in, or a 401
 * from another request. The answer to the last ask no longer stands, so the
 * store goes back to unresolved first. A refresh that then meets a 5xx leaves
 * it unresolved and asks again until it is answered, rather than keeping a
 * stale `null` (which would send a user who has just signed in back to the
 * login page) or a stale user (which would keep a lapsed admin on a page whose
 * polling has stopped). The layout guards wait while it is unresolved.
 */
export function resetSession(): Promise<Me | null | undefined> {
  stopRetrying();
  session.set(undefined);
  return refreshSession();
}

export async function signOut(): Promise<void> {
  try {
    await api.logout();
  } finally {
    stopRetrying();
    session.set(null);
  }
}
