/**
 * The Tauri bridge, exposed on `window` because the app sets
 * `withGlobalTauri: true`. Declaring it here rather than importing
 * `@tauri-apps/api` keeps the frontend at zero runtime dependencies.
 */
interface Window {
  __TAURI__?: {
    core: {
      invoke: (cmd: string, args?: unknown) => Promise<unknown>;
    };
  };
}
