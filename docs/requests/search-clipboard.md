# Requests from `search-clipboard`

## To `adaptive-island` (width contract)

**Add an island state `peek:palette`, 560 px wide.**

The command palette (Ctrl+Alt+K) is drawn as a peek directly under the
island, but in its own window (`palette`), because it has to take keyboard
focus and the island never does. It does not change the island's own size
or call `set_island_motion`, `set_hit_rect` or `set_backdrop`.

Requested row in section 4:

| State | min | preferred | max | overflow |
| --- | --- | --- | --- | --- |
| peek:palette | 560 | 560 | 560 | ellipsis, height 440 fixed |

Why wider than the other peeks: each result shows a title, its source line
and the Enter action; at 440 px most titles cut off after ~35 characters.

Until this is decided the palette uses 560 px. If you would rather keep every
peek at 440, change `WIDTH` in `src-tauri/src/search/palette.rs` and
`PALETTE_WIDTH` in `src/lib/palette/api.ts`; nothing else depends on it.

The declaration lives in `src/lib/palette/api.ts` as `PALETTE_WIDTH`, shaped
like `WidthContract` (no `measure`, since the width is fixed). Once
`src/lib/island/width.ts` is on `main` I will import the real type.

## To `recorder-forecast` (event store)

Nothing needed. Agent history in search codes against contract v1.1
(`Store::events` with `since_ms`, and the `recorder:changed` event) through a
stub in `src-tauri/src/search/agent.rs`. When your branch is merged,
`fetch_since` becomes one call; see the comment there.
