<script lang="ts">
  import { ChevronLeft, ChevronRight, FileType } from '@lucide/svelte';
  import { media, type Item, type Segment } from '../../lib/api';

  let { item, page = $bindable(1), segments }: { item: Item; page?: number; segments: Segment[] } = $props();

  const pages = $derived(Math.max(0, ...segments.map((s) => s.page ?? 0)));
  const matched = $derived(new Set(segments.filter((s) => s.score != null && s.page != null).slice(0, 5).map((s) => s.page!)));
  let failed = $state(false);
  let loaded = $state(false);

  $effect(() => {
    void page;
    failed = false;
    loaded = false;
  });

  const pageText = $derived(
    segments
      .filter((s) => s.page === page && s.text)
      .map((s) => s.text)
      .join('\n\n'),
  );

  function go(d: number) {
    const next = page + d;
    if (next < 1 || (pages && next > pages)) return;
    page = next;
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === 'PageDown') go(1);
    else if (e.key === 'PageUp') go(-1);
    else return;
    e.preventDefault();
  }
</script>

<svelte:window {onkeydown} />

<div class="doc">
  <div class="page" class:loaded>
    {#if failed}
      <div class="text">
        {#if pageText}
          <p>{pageText}</p>
        {:else}
          <span class="none"><FileType size={28} />No picture of this page yet. Use "Open" to read the document.</span>
        {/if}
      </div>
    {:else}
      {#key page}
        <img src={media(item.urls.preview, { page })} alt="Page {page} of {item.name}" onload={() => (loaded = true)} onerror={() => (failed = true)} />
      {/key}
    {/if}
  </div>
  <div class="pager">
    <button class="icon-btn" type="button" aria-label="Previous page" disabled={page <= 1} onclick={() => go(-1)}><ChevronLeft size={18} /></button>
    <span class="num" aria-live="polite">Page {page}{pages ? ` of ${pages}` : ''}</span>
    <button class="icon-btn" type="button" aria-label="Next page" disabled={!!pages && page >= pages} onclick={() => go(1)}><ChevronRight size={18} /></button>
    {#if matched.size > 1}
      <span class="jump">
        Matches on
        {#each [...matched].sort((a, b) => a - b) as p (p)}
          <button type="button" class="pg" class:on={p === page} onclick={() => (page = p)}>{p}</button>
        {/each}
      </span>
    {/if}
  </div>
</div>

<style>
  .doc {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .page {
    display: grid;
    place-items: center;
    min-height: 300px;
    max-height: 70vh;
    aspect-ratio: 1 / 1.3;
    border-radius: var(--radius);
    background: var(--surface-2);
    overflow: auto;
  }
  .page img {
    width: 100%;
    height: 100%;
    object-fit: contain;
    background: #fff;
  }
  .text {
    align-self: start;
    padding: 18px;
    font-size: 14px;
    white-space: pre-wrap;
    color: var(--text-2);
  }
  .text p {
    margin: 0;
  }
  .none {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    color: var(--muted);
    text-align: center;
    padding-top: 80px;
  }
  .pager {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
    font-size: 13.5px;
    color: var(--text-2);
  }
  .jump {
    margin-left: auto;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    color: var(--muted);
  }
  .pg {
    min-width: 28px;
    height: 26px;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: var(--surface);
    font-size: 12.5px;
  }
  .pg.on {
    border-color: var(--accent);
    color: var(--accent);
  }
</style>
