<script lang="ts">
  import { Search, X, LoaderCircle, Sparkles, FileUp } from '@lucide/svelte';
  import { app } from '../lib/app.svelte';
  import { media } from '../lib/api';

  let { input = $bindable() }: { input?: HTMLInputElement } = $props();
  let fileInput: HTMLInputElement;

  async function onkeydown(e: KeyboardEvent) {
    if (e.key === 'Enter') {
      e.preventDefault();
      await app.settle();
      if (app.hits.length) {
        app.returnFocus = document.getElementById(`r-${app.selected}`);
        app.openIndex(app.selected);
      }
    } else if (e.key === 'ArrowDown' && app.hits.length) {
      e.preventDefault();
      document.getElementById(`r-${app.selected}`)?.focus();
    } else if (e.key === 'Escape' && !app.preview && !app.drawerOpen) {
      if (app.url.q || app.upload || app.url.like != null) {
        e.preventDefault();
        if (app.url.q) app.setQuery('');
        else app.home();
      }
    }
  }

  function clear() {
    if (app.upload || app.url.like != null) app.home();
    else app.setQuery('');
    input?.focus();
  }

  function pick(e: Event) {
    const f = (e.currentTarget as HTMLInputElement).files?.[0];
    if (f) app.searchByFile(f);
    (e.currentTarget as HTMLInputElement).value = '';
  }

  const by = $derived(app.upload?.file.name ?? (app.url.like != null ? (app.likeItem?.name ?? `item ${app.url.like}`) : ''));
  const byThumb = $derived(
    app.upload?.thumb ??
      (app.likeItem && (app.likeItem.kind === 'image' || app.likeItem.kind === 'video') ? media(app.likeItem.urls.thumb) : ''),
  );
</script>

<div class="box" class:busy={app.loading}>
  <span class="lead" aria-hidden="true">
    {#if app.loading}
      <LoaderCircle size={20} class="spin" />
    {:else}
      <Search size={20} />
    {/if}
  </span>
  {#if by}
    <span class="by" title={by}>
      {#if byThumb}<img src={byThumb} alt="" />{:else}<Sparkles size={15} />{/if}
      <span class="by-label">{app.upload ? 'Like file' : 'Like'}</span>
      <span class="by-name">{by}</span>
    </span>
  {/if}
  <input
    bind:this={input}
    type="search"
    placeholder={by ? 'Or type to search by words' : 'Search your files: words, places, things in photos'}
    aria-label="Search your files"
    aria-controls={app.hits.length ? 'results' : undefined}
    aria-describedby="status-line"
    autocomplete="off"
    autocapitalize="off"
    spellcheck="false"
    enterkeyhint="search"
    value={app.url.q}
    oninput={(e) => app.setQuery(e.currentTarget.value)}
    {onkeydown}
  />
  {#if app.url.q || by}
    <button class="icon-btn" type="button" aria-label="Clear search" title="Clear (Esc)" onclick={clear}><X size={18} /></button>
  {/if}
  <button
    class="icon-btn"
    type="button"
    aria-label="Search by a file"
    title="Search by an image, audio or text file (or drop one anywhere)"
    onclick={() => fileInput.click()}><FileUp size={18} /></button
  >
  <input bind:this={fileInput} class="sr-only" type="file" tabindex="-1" aria-hidden="true" accept="image/*,audio/*,text/*,.md,.txt" onchange={pick} />
  <kbd class="hint" aria-hidden="true">/</kbd>
</div>

<style>
  .box {
    position: relative;
    display: flex;
    align-items: center;
    gap: 4px;
    width: 100%;
    height: var(--box-h, 56px);
    padding: 0 8px 0 14px;
    border-radius: 16px;
    background: var(--surface);
    border: 1px solid var(--border-strong);
    box-shadow: var(--shadow);
    transition:
      height var(--med) var(--ease),
      border-radius var(--med) var(--ease),
      box-shadow var(--fast);
  }
  .box:focus-within {
    border-color: var(--accent);
    box-shadow:
      0 0 0 3px var(--accent-soft),
      var(--shadow);
  }
  .lead {
    display: grid;
    place-items: center;
    color: var(--muted);
    width: 24px;
    flex: none;
  }
  .lead :global(.spin) {
    animation: spin 0.9s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  input[type='search'] {
    flex: 1;
    min-width: 0;
    height: 100%;
    border: 0;
    outline: 0;
    background: transparent;
    font-size: var(--box-font, 18px);
    padding: 0 6px;
    transition: font-size var(--med) var(--ease);
  }
  input[type='search']::-webkit-search-cancel-button {
    display: none;
  }
  input::placeholder {
    color: var(--muted);
    opacity: 1;
  }
  .by {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    max-width: 45%;
    height: 30px;
    padding: 0 10px 0 4px;
    border-radius: 999px;
    background: var(--accent-soft);
    color: var(--text);
    font-size: 13px;
    flex: none;
  }
  .by img {
    width: 24px;
    height: 24px;
    border-radius: 999px;
    object-fit: cover;
  }
  .by :global(svg) {
    margin-left: 4px;
    color: var(--accent);
  }
  .by-label {
    color: var(--muted);
  }
  .by-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: 500;
  }
  .hint {
    display: none;
    font: 12px var(--mono);
    color: var(--muted);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 1px 7px;
    margin: 0 6px 0 2px;
  }
  @media (min-width: 760px) {
    .box:not(:focus-within) .hint {
      display: inline-block;
    }
  }
</style>
