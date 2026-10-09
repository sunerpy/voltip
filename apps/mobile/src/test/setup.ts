import "@testing-library/jest-dom/vitest";
import { cleanup, configure } from "@testing-library/react";

// The pages show the core's answers asynchronously, and on a busy machine (CI's coverage run,
// 2026-10-01: a page opened after 返回 had not drawn its rows within the default second, in two
// different history tests) they take longer. The waits for them do too; no test here times how
// fast a page reacts.
configure({ asyncUtilTimeout: 5000 });

/** jsdom 27 under Node ≥ 22 leaves `window.localStorage` undefined; tests need a real Storage. */
class MemoryStorage implements Storage {
  private map = new Map<string, string>();
  get length() {
    return this.map.size;
  }
  clear() {
    this.map.clear();
  }
  getItem(key: string) {
    return this.map.get(key) ?? null;
  }
  key(index: number) {
    return [...this.map.keys()][index] ?? null;
  }
  removeItem(key: string) {
    this.map.delete(key);
  }
  setItem(key: string, value: string) {
    this.map.set(key, value);
  }
}

if (typeof window.localStorage !== "object") {
  const storage = new MemoryStorage();
  Object.defineProperty(window, "localStorage", { value: storage, configurable: true });
  Object.defineProperty(globalThis, "localStorage", { value: storage, configurable: true });
}

afterEach(() => {
  cleanup();
  window.localStorage.clear();
  delete document.documentElement.dataset.theme;
});

/** jsdom has no pointer capture; the hold-to-talk button captures the finger on press. */
if (typeof HTMLElement.prototype.setPointerCapture !== "function") {
  HTMLElement.prototype.setPointerCapture = () => undefined;
  HTMLElement.prototype.releasePointerCapture = () => undefined;
}
