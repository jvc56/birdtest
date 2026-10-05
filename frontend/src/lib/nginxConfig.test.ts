import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

// The Nginx in front of the app in compose and local stacks proxies /api/
// itself (deployed, the load balancer sends /api/ past it). A results stream
// that fails part-way is cut off without its closing chunk -- the only way a
// client learns the download is short -- and with chunked transfer turned off
// Nginx answered without a length and simply closed the connection, so a cut
// stream read as a complete one: curl exited 0 after 15,641 of 150,000 lines
// (the audit's pass 21). Chunking also needs HTTP/1.1 to the backend: at 1.0
// the backend sends no chunks, and Nginx cannot see a cut either.
describe('F-NGINX-1 the proxy keeps a cut stream visible', () => {
  const template = readFileSync(join(__dirname, '..', '..', 'docker', 'default.conf.template'), 'utf8');
  const code = template
    .split('\n')
    .map((line) => line.replace(/#.*$/, ''))
    .join('\n');

  /** The body of `location /api/ { ... }`, braces balanced. */
  function apiLocation(): string {
    const start = code.search(/location\s+\/api\/\s*\{/);
    expect(start, 'no `location /api/` block').toBeGreaterThanOrEqual(0);
    let depth = 0;
    for (let i = code.indexOf('{', start); i < code.length; i++) {
      if (code[i] === '{') depth++;
      if (code[i] === '}' && --depth === 0) return code.slice(start, i + 1);
    }
    throw new Error('the `location /api/` block is never closed');
  }

  it('turns chunked transfer encoding off nowhere, in any spelling', () => {
    // Nginx takes the value in any case and quoted.
    expect(code).not.toMatch(/chunked_transfer_encoding\s+["']?off["']?/i);
  });

  it('proxies /api/ over HTTP/1.1, unbuffered for server-sent events', () => {
    const api = apiLocation();
    expect(api).toMatch(/proxy_http_version\s+1\.1\s*;/);
    expect(api).toMatch(/proxy_buffering\s+off\s*;/);
  });
});

// Deployed, every page and asset reaches this Nginx from the ALB over a pooled
// connection, which the ALB keeps for its idle timeout. Nginx's own default
// closed it at 65 s while the ALB, raised to 300 s, still counted it open, and
// a request sent on it as it closed was answered 502.
describe('F-NGINX-2 Nginx keeps an idle connection longer than the load balancer', () => {
  const read = (...path: string[]) => readFileSync(join(__dirname, '..', '..', ...path), 'utf8');
  const uncommented = (text: string) =>
    text
      .split('\n')
      .map((line) => line.replace(/#.*$/, ''))
      .join('\n');

  /** Seconds in an Nginx time: `310`, `310s`, `5m`, `1h`. */
  function seconds(value: string): number {
    const match = /^(\d+)(s|m|h)?$/.exec(value);
    expect(match, `not a time Nginx reads: ${value}`).not.toBeNull();
    const [, n, unit] = match!;
    return Number(n) * ({ s: 1, m: 60, h: 3600 } as Record<string, number>)[unit ?? 's'];
  }

  it('sets keepalive_timeout above aws_lb.main.idle_timeout', () => {
    const nginx = uncommented(read('docker', 'default.conf.template'));
    const keepalive = [...nginx.matchAll(/^\s*keepalive_timeout\s+([^\s;]+)[^;]*;/gm)];
    expect(keepalive, 'one keepalive_timeout, in the server block').toHaveLength(1);

    const ecs = uncommented(read('..', 'infra', 'ecs.tf'));
    const alb = /resource\s+"aws_lb"\s+"main"\s*\{[^}]*?\bidle_timeout\s*=\s*(\d+)/.exec(ecs);
    expect(alb, 'no idle_timeout on aws_lb.main in infra/ecs.tf').not.toBeNull();

    expect(seconds(keepalive[0][1])).toBeGreaterThan(Number(alb![1]));
  });
});
