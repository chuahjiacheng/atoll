// Spotify + lyrics: types mirror src-tauri/src/spotify.rs and lyrics.rs.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { settingsApi } from "./settings";

export interface Track {
  id: string;
  name: string;
  artists: string;
  album: string;
  image: string | null;
  durationMs: number;
}

export interface Device {
  id: string | null;
  name: string;
  kind: string;
  isActive: boolean;
  isRestricted: boolean;
  volume: number | null;
}

export interface Player {
  isPlaying: boolean;
  track: Track | null;
  progressMs: number;
  fetchedAt: number;
  device: Device | null;
  /** Spotify accepts seeking from Windows. */
  canSeek: boolean;
}

export interface SpotifyStatus {
  /** Reading Spotify on this PC is on (the Connect button). */
  connected: boolean;
  /** The Spotify app is open on this PC. */
  appOpen: boolean;
  /** An account is linked for device switching (optional). */
  account: boolean;
  hasClientId: boolean;
  user: string | null;
  error: string | null;
  redirectUri: string;
}

export interface LyricLine {
  t: number;
  text: string;
}

export interface Lyrics {
  trackId: string;
  synced: LyricLine[];
  plain: string | null;
  instrumental: boolean;
  state: "found" | "not_found" | "error";
}

export type SpotifyAction = "play" | "pause" | "toggle" | "next" | "previous" | "seek" | "transfer" | "volume";

export const spotifyApi = {
  status: () => invoke<SpotifyStatus>("spotify_status"),
  /** Connect = read Spotify on this PC through Windows. No sign-in. */
  connect: (on: boolean) => settingsApi.update({ spotify: { connected: on } }),
  openApp: () => invoke<void>("spotify_open_app"),
  /** Optional account link, only for device switching. */
  linkAccount: () => invoke<void>("spotify_login"),
  unlinkAccount: () => invoke<void>("spotify_logout"),
  player: () => invoke<Player | null>("spotify_player"),
  action: (action: SpotifyAction, value?: number | string) => invoke<void>("spotify_action", { action, value }),
  /** Spotify's own volume as the app on this PC shows it (read only, no account). */
  appVolume: () => invoke<number | null>("spotify_app_volume"),
  devices: () => invoke<Device[]>("spotify_devices"),
  lyrics: (trackId: string) => invoke<Lyrics | null>("get_lyrics", { trackId }),
};

export const onPlayer = (cb: (p: Player | null) => void): Promise<UnlistenFn> =>
  listen<Player | null>("spotify-player", (e) => cb(e.payload));
export const onSpotifyStatus = (cb: (s: SpotifyStatus) => void): Promise<UnlistenFn> =>
  listen<SpotifyStatus>("spotify-status", (e) => cb(e.payload));
export const onLyrics = (cb: (l: Lyrics) => void): Promise<UnlistenFn> => listen<Lyrics>("lyrics", (e) => cb(e.payload));

/** Progress estimated locally between polls. */
export function positionMs(p: Player, now: number): number {
  const elapsed = p.isPlaying ? Math.max(0, now - p.fetchedAt) : 0;
  return Math.min(p.track?.durationMs ?? 0, p.progressMs + elapsed);
}

export function clock(ms: number): string {
  const s = Math.floor(ms / 1000);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}
