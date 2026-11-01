import { MockBackend as SharedMockBackend, type MockBackendOptions } from "@voltip/shared/mock";

/** The in-memory core of the page tests. It closes the microphone at once on a stop, as these tests
 *  were written for: a new install keeps recording `DEFAULT_EXTRA_RECORDING_MS` after the stop
 *  (docs/dictation.md §30.4), which `packages/shared`'s mock tests and the core's tests check. A
 *  test of the extra recording asks for it (`settings: { extra_recording_ms }`). */
export class MockBackend extends SharedMockBackend {
  constructor(options: MockBackendOptions = {}) {
    super({ ...options, settings: { extra_recording_ms: 0, ...options.settings } });
  }
}
