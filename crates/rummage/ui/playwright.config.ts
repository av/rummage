import { defineConfig, devices } from '@playwright/test';

// Mock servers (see mock/server.ts), one per kind of state so parallel tests don't collide:
//   4173  10k items, read-only use        4174  empty (first run)
//   4175  needs a token                   4176  10k items, tests that change folders
// RUMMAGE_BIN=<path to rummage> adds the `real` project, which runs the built binary.
export const MOCK = 'http://127.0.0.1:4173';
const real = process.env.RUMMAGE_BIN;

const mock = (port: number, env: Record<string, string> = {}) => ({
  command: 'bun mock/server.ts',
  env: { PORT: String(port), ...env },
  url: `http://127.0.0.1:${port}/api/v1/health`,
  reuseExistingServer: !process.env.CI,
  stdout: 'ignore' as const,
});

export default defineConfig({
  testDir: 'e2e',
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? [['list'], ['html', { open: 'never' }]] : 'list',
  use: { baseURL: `${MOCK}/search/`, trace: 'retain-on-failure' },
  webServer: [
    mock(4173),
    mock(4174, { MOCK_EMPTY: '1' }),
    mock(4175, { MOCK_TOKEN: 'secret' }),
    mock(4176),
    ...(real
      ? [{ command: 'bun e2e/real-server.ts', url: 'http://127.0.0.1:4180/search/api/v1/health', reuseExistingServer: false, stdout: 'ignore' as const }]
      : []),
  ],
  projects: [
    { name: 'desktop', use: { ...devices['Desktop Chrome'], viewport: { width: 1440, height: 900 } }, testIgnore: /(phone|real|screenshots)\.spec/ },
    { name: 'phone', use: { ...devices['Pixel 7'] }, testMatch: /phone\.spec/ },
    ...(real ? [{ name: 'real', use: { ...devices['Desktop Chrome'], baseURL: 'http://127.0.0.1:4180/search/' }, testMatch: /real\.spec/ }] : []),
    ...(process.env.SCREENSHOTS ? [{ name: 'screenshots', testMatch: /screenshots\.spec/ }] : []),
  ],
});
