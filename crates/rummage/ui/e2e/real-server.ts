// Runs the real `rummage serve` (RUMMAGE_BIN) for the `real` Playwright project, with a fresh
// config, data dir and home directory, under the /search base path.
import { mkdtempSync, mkdirSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const bin = process.env.RUMMAGE_BIN!;
const root = mkdtempSync(join(tmpdir(), 'rummage-e2e-'));
const home = join(root, 'home');
for (const d of ['Pictures', 'notes', 'Music']) mkdirSync(join(home, d), { recursive: true });
writeFileSync(join(home, 'notes', 'garden.md'), '# Garden\n\nThe red bicycle leaning on the garden fence.\n');
const config = join(root, 'config.toml');
writeFileSync(config, `data_dir = "${join(root, 'data')}"\n`);

const proc = Bun.spawn([bin, 'serve', '--bind', '127.0.0.1:4180', '--base-path', '/search'], {
  env: { ...process.env, HOME: home, RUMMAGE_CONFIG: config },
  stdout: 'inherit',
  stderr: 'inherit',
});
const stop = () => proc.kill();
process.on('SIGINT', stop);
process.on('SIGTERM', stop);
process.exit(await proc.exited);
