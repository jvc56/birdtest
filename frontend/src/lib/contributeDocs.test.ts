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
    // The hooks tier 5's E-2 reads the key by: a class both share matched two.
    expect(page).toContain('data-testid="fresh-key"');
    expect(page).toContain('data-testid="fresh-key-line"');
  });

  it('never put contribute.txt beside the binary', () => {
    // MAGPIE reads it, and data/, from its working directory.
    const offenders = svelteFiles(routes).filter((f) => /contribute\.txt<\/code>\s+beside/.test(readFileSync(f, 'utf8')));
    expect(offenders).toEqual([]);
  });

  it('never send a second process to a directory of its own', () => {
    // MAGPIE loads its default board from ./data before it parses anything,
    // so a directory holding only a contribute.txt cannot start. (Running a
    // second process, on a settings file of its own, is the README's.)
    const offenders = svelteFiles(routes).filter((f) => /directory of its own/.test(readFileSync(f, 'utf8')));
    expect(offenders).toEqual([]);
  });
});
