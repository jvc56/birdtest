import fs from 'node:fs';
import path from 'node:path';
import { env } from './env';

/**
 * Reading mail the backend wrote with MAIL_BACKEND=file (TESTING.md, "Reading
 * confirmation codes"). One file per message, named by time and recipient, in
 * a directory bind-mounted from the host. A journey finds its own mail by its
 * own address, which is what keeps journeys from reading each other's codes.
 */

export interface Mail {
  file: string;
  to: string;
  subject: string;
  body: string;
}

/**
 * The file-name suffix for mail to `email`: backend/src/email.rs
 * `outbox_file_name` lowercases ASCII, spells `@` as `-at-` and replaces
 * everything else outside `[a-z0-9-]` with `-`.
 */
export function outboxSuffix(email: string): string {
  const lowered = [...email].map((c) => (c.charCodeAt(0) < 128 ? c.toLowerCase() : c)).join('');
  return `-${lowered.replace(/@/g, '-at-').replace(/[^a-z0-9-]/g, '-')}.txt`;
}

function parse(file: string): Mail {
  const text = fs.readFileSync(file, 'utf8');
  const split = text.indexOf('\n\n');
  const headers = text.slice(0, split);
  const header = (name: string) =>
    headers
      .split('\n')
      .find((line) => line.startsWith(`${name}: `))
      ?.slice(name.length + 2) ?? '';
  return { file, to: header('To'), subject: header('Subject'), body: text.slice(split + 2) };
}

/** Every message to `email` so far, oldest first. Names start with the time. */
export function mailTo(email: string): Mail[] {
  if (!env.outbox) throw new Error('E2E_OUTBOX_DIR is not set; run the suite with e2e/run.sh');
  const suffix = outboxSuffix(email);
  return fs
    .readdirSync(env.outbox)
    .filter((name) => name.endsWith(suffix) && !name.startsWith('.'))
    .sort()
    .map((name) => parse(path.join(env.outbox, name)));
}

/**
 * The newest message to `email` with `subject`, waiting for it to arrive: a
 * password reset is sent off the request path, so it can land a moment after
 * the page has answered.
 */
export async function waitForMail(email: string, subject: string, timeoutMs = 15_000): Promise<Mail> {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const found = mailTo(email).filter((mail) => mail.subject === subject);
    if (found.length) return found[found.length - 1];
    if (Date.now() > deadline) {
      const seen = mailTo(email).map((mail) => mail.subject);
      throw new Error(`no "${subject}" mail to ${email} within ${timeoutMs} ms (have: ${JSON.stringify(seen)})`);
    }
    await new Promise((resolve) => setTimeout(resolve, 250));
  }
}

/** The one link in a message that goes to `pathname` on the site. */
export function linkIn(mail: Mail, pathname: string): string {
  const links = mail.body.match(/https?:\/\/\S+/g) ?? [];
  const link = links.find((candidate) => new URL(candidate).pathname === pathname);
  if (!link) throw new Error(`no ${pathname} link in ${mail.file}:\n${mail.body}`);
  return link;
}

export const CONFIRM_SUBJECT = 'Confirm your birdtest account';
export const RESET_SUBJECT = 'Reset your birdtest password';
