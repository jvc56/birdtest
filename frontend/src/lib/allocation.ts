/**
 * "Share equally" on the allocation page: 100% split as evenly as whole
 * numbers allow among the jobs set above 0% and the jobs the creation form
 * just made (`fresh`), the remainder a point each to the first of them, and
 * every other job at 0%. All of them when none is either.
 *
 * The new jobs are counted with the running ones because they are inactive
 * at 0%: split among the running jobs alone, a round robin made beside a job
 * already at 100% was left out, and the button changed nothing.
 */
export function equalShares(
  ids: string[],
  values: Record<string, number>,
  fresh: Set<string>
): Record<string, number> {
  const chosen = ids.filter((id) => Number(values[id]) > 0 || fresh.has(id));
  const among = chosen.length ? chosen : ids;
  const next: Record<string, number> = { ...values };
  if (!among.length) return next;
  const each = Math.floor(100 / among.length);
  let left = 100 - each * among.length;
  for (const id of ids) next[id] = 0;
  for (const id of among) {
    next[id] = each + (left > 0 ? 1 : 0);
    if (left > 0) left -= 1;
  }
  return next;
}
