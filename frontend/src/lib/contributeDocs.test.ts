import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

// The pages that tell a contributor how to run MAGPIE. `magpie contribute`
// takes its API key only from an `apikey` line in contribute.txt, never from
// the command line (MAGPIE's config.c): a key there ends up in shell history
// and `ps` output. `--api-key` belongs to the test-only Python worker.
function svelteFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return svelteFiles(path);
    return path.endsWith('.svelte') ? [path] : [];
  });
}

describe('F-DOCS-1 contributor instructions', () => {
  const routes = join(__dirname, '..', 'routes');

  it('never tell a contributor to pass a key on the command line', () => {
    const offenders = svelteFiles(routes).filter((f) => readFileSync(f, 'utf8').includes('--api-key'));
    expect(offenders).toEqual([]);
  });

  it('show the account page the contribute.txt line MAGPIE reads', () => {
    const page = readFileSync(join(routes, 'account', '+page.svelte'), 'utf8');
    expect(page).toContain('contribute.txt');
    expect(page).toMatch(/apikey \{freshKey\}/);
  });
});
