<script lang="ts">
  import { Folder, Calendar, X, FolderSearch } from '@lucide/svelte';
  import { app } from '../lib/app.svelte';
  import type { Kind } from '../lib/api';
  import { num } from '../lib/format';

  const chips: { kind: Kind | ''; label: string }[] = [
    { kind: '', label: 'All' },
    { kind: 'image', label: 'Images' },
    { kind: 'video', label: 'Video' },
    { kind: 'audio', label: 'Audio' },
    { kind: 'doc', label: 'Docs' },
    { kind: 'text', label: 'Text' },
  ];

  const all = $derived(Object.values(app.kinds).reduce((a, b) => a + (b ?? 0), 0));
  const count = (k: Kind | '') => (k ? (app.kinds[k] ?? 0) : all);
  const folderName = (p: string) => p.split('/').filter(Boolean).pop() ?? p;

  function setDate(v: string) {
    if (v === 'custom') app.url.date = 'custom';
    else app.setFilter({ date: v, from: '', to: '' });
  }

  function onChipKey(e: KeyboardEvent) {
    if (e.key !== 'ArrowLeft' && e.key !== 'ArrowRight') return;
    const btns = [...(e.currentTarget as HTMLElement).querySelectorAll<HTMLButtonElement>('button')];
    const i = btns.indexOf(document.activeElement as HTMLButtonElement);
    if (i < 0) return;
    e.preventDefault();
    e.stopPropagation();
    btns[(i + (e.key === 'ArrowRight' ? 1 : btns.length - 1)) % btns.length].focus();
  }
</script>

<div class="bar">
  <div class="chips" role="radiogroup" aria-label="Kind of file" tabindex="-1" onkeydown={onChipKey}>
    {#each chips as c (c.kind)}
      {@const n = count(c.kind)}
      <button
        type="button"
        role="radio"
        aria-checked={app.url.type === c.kind}
        class="chip"
        class:on={app.url.type === c.kind}
        class:empty={!n && app.url.type !== c.kind && !app.loading}
        tabindex={app.url.type === c.kind ? 0 : -1}
        onclick={() => app.setFilter({ type: c.kind })}
      >
        {c.label}
        <span class="count num">{app.hits.length || n ? num(n) : ''}</span>
      </button>
    {/each}
  </div>

  <div class="side">
    {#if app.url.dir}
      <span class="pill on" title={app.url.dir}>
        <FolderSearch size={14} aria-hidden="true" />
        <span class="pill-text">In {folderName(app.url.dir)}</span>
        <button type="button" class="x" aria-label="Search everywhere, not only in {app.url.dir}" onclick={() => app.setFilter({ dir: '' })}><X size={13} /></button>
      </span>
    {/if}
    {#if app.folders.length > 1}
      <label class="pill" class:on={!!app.url.folder}>
        <Folder size={14} aria-hidden="true" />
        <span class="sr-only">Folder</span>
        <select value={app.url.folder} onchange={(e) => app.setFilter({ folder: e.currentTarget.value })}>
          <option value="">All folders</option>
          {#each app.folders as f (f.id)}
            <option value={f.id}>{folderName(f.path)}</option>
          {/each}
        </select>
      </label>
    {/if}
    <label class="pill" class:on={!!app.url.date}>
      <Calendar size={14} aria-hidden="true" />
      <span class="sr-only">Date</span>
      <select value={app.url.date} onchange={(e) => setDate(e.currentTarget.value)}>
        <option value="">Any time</option>
        <option value="week">Past week</option>
        <option value="month">Past month</option>
        <option value="year">Past year</option>
        <option value="custom">Between dates…</option>
      </select>
    </label>
    {#if app.url.date === 'custom'}
      <span class="range">
        <input class="input sm" type="date" aria-label="From" value={app.url.from} onchange={(e) => app.setFilter({ from: e.currentTarget.value })} />
        <span aria-hidden="true">–</span>
        <input class="input sm" type="date" aria-label="To" value={app.url.to} onchange={(e) => app.setFilter({ to: e.currentTarget.value })} />
      </span>
    {/if}
    {#if app.url.folder || app.url.date || app.url.dir}
      <button type="button" class="clear" onclick={() => app.clearFilters()}>Clear filters</button>
    {/if}
  </div>
</div>

<style>
  .bar {
    display: flex;
    align-items: center;
    gap: 8px 16px;
    max-width: 1600px;
    margin: 0 auto;
    padding: 0 20px 10px;
    overflow-x: auto;
    scrollbar-width: none;
  }
  .bar::-webkit-scrollbar {
    display: none;
  }
  .chips,
  .side {
    display: flex;
    align-items: center;
    gap: 6px;
    flex: none;
  }
  .side {
    margin-left: auto;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 32px;
    padding: 0 12px;
    border-radius: 999px;
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text-2);
    font-size: 13.5px;
    font-weight: 500;
    transition:
      background var(--fast),
      color var(--fast),
      border-color var(--fast);
  }
  .chip:hover {
    border-color: var(--border-strong);
    color: var(--text);
  }
  .chip.on {
    background: var(--text);
    border-color: var(--text);
    color: var(--bg);
  }
  .chip.empty {
    color: var(--muted);
  }
  .count {
    min-width: 1ch;
    font-size: 12px;
    color: var(--muted);
  }
  .chip.on .count {
    color: inherit;
  }
  .pill {
    position: relative;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 32px;
    padding: 0 10px;
    border-radius: 999px;
    color: var(--muted);
    font-size: 13.5px;
    border: 1px solid transparent;
  }
  .pill:hover,
  .pill:focus-within {
    background: var(--surface-2);
    color: var(--text);
  }
  .pill.on {
    color: var(--text);
    border-color: var(--border);
    background: var(--surface);
  }
  .pill select {
    border: 0;
    background: transparent;
    color: inherit;
    font-size: inherit;
    padding: 0 2px;
    cursor: pointer;
    outline: none;
    max-width: 160px;
    field-sizing: content;
  }
  .pill select option {
    background: var(--surface);
    color: var(--text);
  }
  .pill:has(select:focus-visible) {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .pill-text {
    max-width: 180px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .x {
    display: grid;
    place-items: center;
    width: 20px;
    height: 20px;
    margin-right: -4px;
    border: 0;
    border-radius: 999px;
    background: transparent;
    color: var(--muted);
  }
  .x:hover {
    background: var(--surface-3);
    color: var(--text);
  }
  .range {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    color: var(--muted);
  }
  .input.sm {
    height: 30px;
    font-size: 13px;
    padding: 0 8px;
  }
  .clear {
    border: 0;
    background: none;
    color: var(--accent);
    font-size: 13.5px;
    padding: 0 6px;
    white-space: nowrap;
  }
  @media (max-width: 759px) {
    .bar {
      padding: 0 12px 8px;
    }
    .side {
      margin-left: 0;
    }
  }
</style>
