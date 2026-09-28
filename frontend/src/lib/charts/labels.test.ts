import { describe, expect, it } from 'vitest';
import { fitLabel, fitLabels, LABEL_CHARS } from './labels';

describe('label shortening', () => {
  it('shortens a long name in the middle, keeping the end that tells configs apart', () => {
    const a = fitLabel('simmer-CSW24-4ply-1000iters-gk16');
    const b = fitLabel('simmer-CSW24-4ply-1000iters-none');
    expect(a.length).toBe(LABEL_CHARS);
    expect(a).not.toBe(b);
    expect(a.startsWith('simmer')).toBe(true);
    expect(a.endsWith('-gk16')).toBe(true);
    expect(a).toContain('…');
    expect(fitLabel('short-name')).toBe('short-name');
    expect(fitLabel('x'.repeat(LABEL_CHARS))).toBe('x'.repeat(LABEL_CHARS));
  });
});

describe('labels drawn together', () => {
  it('tell apart configs that differ in the middle of their names', () => {
    const names = [
      'static-CSW24-equity',
      'simmer-CSW24-4ply-1000iters-gk16',
      'simmer-CSW24-4ply-1000iters-none',
      'simmer-CSW24-2ply-inference-on',
      'simmer-CSW24-2ply-inference-off',
      'simmer-CSW24-4ply-inference-on',
      'simmer-CSW24-4ply-inference-off'
    ];
    const labels = fitLabels(names);
    expect(new Set(labels).size).toBe(names.length);
    for (const label of labels) expect(label.length).toBeLessThanOrEqual(LABEL_CHARS);
    expect(labels[0]).toBe(fitLabel('static-CSW24-equity'));
    // Alike even after the family's shared start is dropped: widened where they differ.
    const alike = fitLabels(['x-aaaaaaaaaaaa1bbbbbbbbbbbb', 'x-aaaaaaaaaaaa2bbbbbbbbbbbb', 'y']);
    expect(alike[0]).not.toBe(alike[1]);
    expect(alike[0].length).toBeLessThanOrEqual(LABEL_CHARS);
  });
});
