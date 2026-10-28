// `@tauri-apps/api` in the React Native app: nothing may reach it, and anything that does fails
// loudly instead of waiting for a Tauri bridge that is not there.
import { Channel, invoke, listen } from "./tauri-stub";

describe("the @tauri-apps/api stand-in", () => {
  it("refuses every call", () => {
    const refused = "@tauri-apps/api is not available in the React Native app";
    expect(() => invoke()).toThrow(refused);
    expect(() => listen()).toThrow(refused);
    expect(() => new Channel()).toThrow(refused);
  });
});
