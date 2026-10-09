// `@tauri-apps/api/core` and `@tauri-apps/api/event` in the React Native app (metro.config.js,
// jest.config.js). `@voltip/shared`'s TauriBackend imports them for its default transport; the app
// always passes its own (`transport.ts`), so nothing may reach these.
function unavailable(): never {
  throw new Error("@tauri-apps/api is not available in the React Native app");
}

export function invoke(): never {
  return unavailable();
}

export function listen(): never {
  return unavailable();
}

// A stand-in for the class `@tauri-apps/api/core` exports: only its constructor can be reached.
// oxlint-disable-next-line no-extraneous-class
export class Channel {
  constructor() {
    unavailable();
  }
}
