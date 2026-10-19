// Fails when the built JavaScript is over budget (gzipped).
import { gzipSync } from 'node:zlib';
import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';

const BUDGET = 200 * 1024;
const dir = process.argv[2] ?? join(import.meta.dir, '..', 'dist', 'assets');
const total = readdirSync(dir)
  .filter((f) => f.endsWith('.js'))
  .reduce((n, f) => n + gzipSync(readFileSync(join(dir, f))).length, 0);
console.log(`JavaScript: ${(total / 1024).toFixed(1)} KB gzipped (budget ${BUDGET / 1024} KB)`);
if (total > BUDGET) process.exit(1);
