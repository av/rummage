<script lang="ts">
  import { ArrowUp, Folder, Check, Plus, House } from '@lucide/svelte';
  import { api, ApiError, type DirListing } from '../lib/api';

  let { onadd }: { onadd: (path: string) => Promise<void> | void } = $props();

  let listing = $state<DirListing | null>(null);
  let error = $state('');
  let busy = $state('');

  async function go(path?: string) {
    error = '';
    try {
      listing = await api.dirs(path);
    } catch (e) {
      error = e instanceof ApiError ? e.message : String(e);
    }
  }

  async function add(path: string) {
    busy = path;
    try {
      await onadd(path);
      if (listing) listing = { ...listing, dirs: listing.dirs.map((d) => (d.path === path ? { ...d, watched: true } : d)) };
    } finally {
      busy = '';
    }
  }

  $effect(() => {
    go();
  });
</script>

<div class="browser">
  <div class="where">
    <button class="icon-btn" type="button" aria-label="Home folder" onclick={() => go()}><House size={16} /></button>
    <button class="icon-btn" type="button" aria-label="Up one folder" disabled={!listing?.parent} onclick={() => go(listing?.parent ?? undefined)}><ArrowUp size={16} /></button>
    <span class="path" title={listing?.path}>&lrm;{listing?.path ?? '…'}&lrm;</span>
    {#if listing}
      <button class="btn sm" type="button" disabled={busy === listing.path} onclick={() => add(listing!.path)}>Add this folder</button>
    {/if}
  </div>
  {#if error}
    <p class="err" role="alert">{error}</p>
  {:else if listing && !listing.dirs.length}
    <p class="none">No folders inside.</p>
  {:else if listing}
    <ul>
      {#each listing.dirs as d (d.path)}
        <li>
          <button class="open" type="button" onclick={() => go(d.path)} title="Open {d.path}">
            <Folder size={16} aria-hidden="true" />
            <span>{d.name}</span>
          </button>
          {#if d.watched}
            <span class="watched"><Check size={14} />Indexed</span>
          {:else}
            <button class="btn sm" type="button" aria-label="Add {d.path}" disabled={busy === d.path} onclick={() => add(d.path)}><Plus size={14} />Add</button>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .browser {
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    overflow: hidden;
  }
  .where {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 6px 8px;
    border-bottom: 1px solid var(--border);
    background: var(--surface-2);
  }
  .where .icon-btn {
    width: 30px;
    height: 30px;
  }
  .path {
    flex: 1;
    min-width: 0;
    font: 12.5px var(--mono);
    color: var(--text-2);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    direction: rtl;
    text-align: left;
    padding: 0 6px;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 4px;
    max-height: 280px;
    overflow-y: auto;
  }
  li {
    display: flex;
    align-items: center;
    gap: 6px;
    padding-right: 4px;
  }
  .open {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 8px;
    height: 34px;
    padding: 0 8px;
    border: 0;
    border-radius: 6px;
    background: transparent;
    text-align: left;
    font-size: 14px;
  }
  .open:hover {
    background: var(--surface-2);
  }
  .open :global(svg) {
    color: var(--muted);
    flex: none;
  }
  .open span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .watched {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 12.5px;
    color: var(--ok);
    padding: 0 6px;
  }
  .err,
  .none {
    margin: 0;
    padding: 12px;
    font-size: 13.5px;
    color: var(--muted);
  }
  .err {
    color: var(--danger);
  }
</style>
