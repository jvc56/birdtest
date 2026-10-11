/**
 * A games or pairs job request's player configs, and the jobs they make --
 * the form's preview of what `POST /api/admin/jobs` will create, by the
 * server's rules (`routes::admin::pairings`, `plan_jobs`):
 *
 * - one config is a self-play job, under the name as given;
 * - n ≥ 2 configs are a round robin of C(n, 2) jobs, every pairing once,
 *   player 1 the config listed first (on the form, ticked first), each named
 *   "{name}: A vs B" ("A vs B" with no name);
 * - at most twelve configs (66 jobs).
 */

/** The most configs one request names, as the server holds it. */
export const MAX_ROUND_ROBIN_CONFIGS = 12;

export interface Matchup<T> {
  player1: T;
  player2: T;
  /** The job's name, as the server will store it. */
  name: string;
}

/**
 * The jobs `players` (in seat order) make under `name`: one self-play job for
 * one player, a job per pairing for more, none for none.
 */
export function matchups<T>(
  players: T[],
  name: string,
  nameOf: (player: T) => string
): Matchup<T>[] {
  const trimmed = name.trim();
  if (players.length === 1) {
    return [{ player1: players[0], player2: players[0], name: trimmed }];
  }
  const out: Matchup<T>[] = [];
  players.forEach((a, i) => {
    for (const b of players.slice(i + 1)) {
      const pairing = `${nameOf(a)} vs ${nameOf(b)}`;
      out.push({ player1: a, player2: b, name: trimmed ? `${trimmed}: ${pairing}` : pairing });
    }
  });
  return out;
}

/** "4 configs → 6 jobs", "1 config → 1 self-play job", or why it cannot be sent. */
export function matchupSummary(configs: number): string {
  if (configs === 0) return 'Tick at least one player config.';
  if (configs > MAX_ROUND_ROBIN_CONFIGS) {
    return `At most ${MAX_ROUND_ROBIN_CONFIGS} player configs: ${configs} are ticked.`;
  }
  if (configs === 1) return '1 config → 1 self-play job';
  const jobs = (configs * (configs - 1)) / 2;
  return `${configs} configs → ${jobs} job${jobs === 1 ? '' : 's'}`;
}

/** Whether `configs` ticked can be sent: one to twelve. */
export function matchupsAllowed(configs: number): boolean {
  return configs >= 1 && configs <= MAX_ROUND_ROBIN_CONFIGS;
}
