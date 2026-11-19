/**
 * Stops a background poll from overwriting a value you just set by hand.
 * Spotify's own volume takes a moment to show a change, so a poll that was
 * started before the edit, or that lands soon after it, still reports the old
 * value and would drag the slider back to it.
 */
export function createHoldoff(ms: number, clock: () => number = Date.now) {
  let editedAt = -Infinity;
  return {
    /** Call whenever the user changes the value. */
    touch() {
      editedAt = clock();
    },
    /** `startedAt` is when the poll began (clock() taken before the request). */
    accepts(startedAt: number): boolean {
      return startedAt > editedAt && clock() - editedAt >= ms;
    },
    now: clock,
  };
}
