import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

// The Nginx in front of the app in compose (and in the ECS task) proxies
// /api/ itself. A results stream that fails part-way is cut off without its
// closing chunk -- the only way a client learns the download is short -- and
// with chunked transfer turned off Nginx answered without a length and simply
// closed the connection, so a cut stream read as a complete one: curl exited
// 0 after 15,641 of 150,000 lines (the audit's pass 21).
describe('F-NGINX-1 the proxy keeps a cut stream visible', () => {
  const template = readFileSync(join(__dirname, '..', '..', 'docker', 'default.conf.template'), 'utf8');
  const code = template
    .split('\n')
    .map((line) => line.replace(/#.*$/, ''))
    .join('\n');

  it('leaves chunked transfer encoding on', () => {
    expect(code).not.toMatch(/chunked_transfer_encoding\s+off/);
  });

  it('still streams server-sent events unbuffered', () => {
    expect(code).toMatch(/proxy_buffering\s+off;/);
  });
});
