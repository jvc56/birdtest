/**
 * Shortened config names for chart labels, where a fixed margin clips what
 * runs past it.
 */

/**
 * The most characters a label shows by default. A chart's margin is fixed,
 * and an SVG clips what runs past it: a name that did lost its tail -- usually the
 * part that tells two configs apart (`…-gk16`, `…-none`) -- and two such
 * labels read the same (the audit's pass 25). 16 characters at 11px fit the
 * margin in lower case and ordinary mixed case; a name in wide capitals can
 * still run past it (KL-76), and the label's title holds the whole name.
 */
export const LABEL_CHARS = 16;

/**
 * `name`, shortened in the middle to at most `max` characters: its start
 * and, longer, its end, which is where configs named in a family differ.
 * The full name goes in the label's title.
 */
export function fitLabel(name: string, max: number = LABEL_CHARS): string {
  if (name.length <= max) return name;
  const head = Math.max(1, Math.floor((max - 1) * 0.4));
  return `${name.slice(0, head)}…${name.slice(name.length - (max - 1 - head))}`;
}

/**
 * Labels for `names` drawn together, each at most `max` characters, told
 * apart from one another: shortened one at a time, `simmer-CSW24-2ply-…-off`
 * and `simmer-CSW24-4ply-…-off` read the same, since they differ in the
 * middle. Segments every config of a family (the part before the first `-`)
 * shares are dropped first -- the table below names them in full -- and two
 * labels still alike are widened around the first character where they differ.
 */
export function fitLabels(names: string[], max: number = LABEL_CHARS): string[] {
  const family = (name: string) => name.split('-')[0];
  const trimmed = names.map((name) => {
    if (name.length <= max) return name;
    const kin = names.filter((other) => other !== name && family(other) === family(name));
    if (!kin.length) return name;
    const segments = name.split('-');
    let shared = 0;
    while (
      shared < segments.length - 1 &&
      kin.every((other) => other.split('-')[shared] === segments[shared])
    ) {
      shared++;
    }
    return shared ? segments.slice(shared).join('-') : name;
  });
  const labels = trimmed.map((t) => fitLabel(t, max));
  return labels.map((label, i) => {
    const twins = labels.flatMap((other, j) => (j !== i && other === label ? [j] : []));
    if (!twins.length) return label;
    const own = trimmed[i];
    let differ = 0;
    while (differ < own.length && twins.every((j) => trimmed[j][differ] === own[differ])) differ++;
    const start = Math.max(0, Math.min(differ - 4, own.length - (max - 1)));
    return start > 0 ? `…${own.slice(start, start + max - 1)}` : own.slice(0, max);
  });
}
