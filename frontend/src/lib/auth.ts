import { writable } from 'svelte/store';
import { api, type Me } from './api';

/**
 * The signed-in user, or null. `undefined` means "not resolved yet", which is
 * what the layout guards wait on — redirecting on `null` before the first
 * `/api/me` completes would bounce a signed-in user to the login page on every
 * hard refresh.
 */
export const session = writable<Me | null | undefined>(undefined);

export async function refreshSession(): Promise<Me | null> {
  try {
    const me = await api.me();
    session.set(me);
    return me;
  } catch {
    session.set(null);
    return null;
  }
}

export async function signOut(): Promise<void> {
  try {
    await api.logout();
  } finally {
    session.set(null);
  }
}
