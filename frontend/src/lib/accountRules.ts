/**
 * The account forms' checks, run before a request so that every error shows
 * the same way: as red text under its field. With the browser's own checks
 * (`required`, `type="email"`) some errors were a native popup and others,
 * the server's, red text; and the browser takes `a@b`, which the server
 * refuses. The forms are `novalidate` and ask these instead.
 *
 * Each rule mirrors one in `backend/src/routes/auth.rs`, with the server's
 * wording, and the server still checks everything: these only answer sooner.
 * A rule the server has and these do not (invisible characters in a username,
 * a password's zxcvbn score) comes back from the server as a field error,
 * which the forms show the same way.
 */

/**
 * Rust's `str::trim`: its white space is Unicode's White_Space property, which
 * JavaScript's `trim` does not quite match (it trims U+FEFF, and not U+0085).
 * A name that is three characters only by the server's count, or the other
 * way round, would be passed here and refused there.
 */
const WHITE_SPACE = String.raw`[\t\n\v\f\r \u0085\u00A0\u1680\u2000-\u200A\u2028\u2029\u202F\u205F\u3000]`;
const EDGE_SPACE = new RegExp(`^${WHITE_SPACE}+|${WHITE_SPACE}+$`, 'g');

export const serverTrim = (s: string) => s.replace(EDGE_SPACE, '');

/**
 * `is_bare_address`: one `local@domain`, nothing else, as SES would take it —
 * a dot-atom before the `@`, host-name labels after it, a top-level domain
 * that is not all digits. Lengths are bytes on the server; any address that
 * could pass is ASCII, where bytes and characters agree.
 */
export function isBareAddress(email: string): boolean {
  const at = email.indexOf('@');
  if (at < 0) return false;
  const local = email.slice(0, at);
  const domain = email.slice(at + 1);
  const atext = /^[A-Za-z0-9!#$%&'*+\-/=?^_`{|}~]+$/;
  const label = (l: string) =>
    l.length >= 1 && l.length <= 63 && !l.startsWith('-') && !l.endsWith('-') && /^[A-Za-z0-9-]+$/.test(l);
  const labels = domain.split('.');
  return (
    email.length <= 254 &&
    local.length <= 64 &&
    local.split('.').every((part) => atext.test(part)) &&
    labels.length >= 2 &&
    labels.every(label) &&
    !/^[0-9]+$/.test(labels[labels.length - 1])
  );
}

/** The username rule `register` checks first: 3 to 32 characters, trimmed. */
export function usernameProblem(username: string): string | null {
  // Characters, as the server counts them: code points, not UTF-16 units.
  const chars = [...serverTrim(username)].length;
  return chars >= 3 && chars <= 32 ? null : 'must be between 3 and 32 characters';
}

/** The address as the server reads it -- trimmed, lower-cased -- checked. */
export function emailProblem(email: string): string | null {
  return isBareAddress(serverTrim(email).toLowerCase()) ? null : 'must be a valid email address';
}

/**
 * A field that must hold something. A password is not trimmed: the server
 * keeps its spaces.
 */
export function requiredProblem(value: string, trim = true): string | null {
  return (trim ? serverTrim(value) : value) === '' ? 'must not be empty' : null;
}

/** The problems found, by field, leaving out the fields that have none. */
export function problems(checks: Record<string, string | null>): Record<string, string> {
  return Object.fromEntries(
    Object.entries(checks).filter((entry): entry is [string, string] => entry[1] !== null)
  );
}
