<script lang="ts">
  import { tick } from 'svelte';
  import { media, type Item, type Segment } from '../../lib/api';
  import { highlight } from '../../lib/highlight';

  let { item, segment, words }: { item: Item; segment: Segment | null; words: string[] } = $props();

  /** Larger files show only the matched passage. */
  const MAX = 2 * 1024 * 1024;
  let text = $state<string | null>(null);
  let error = $state('');
  let hit = $state<HTMLElement>();

  $effect(() => {
    text = null;
    error = '';
    if (item.size > MAX) {
      error = 'big';
      return;
    }
    const ctrl = new AbortController();
    fetch(media(item.urls.raw), { signal: ctrl.signal })
      .then(async (r) => {
        if (!r.ok) throw new Error(r.status === 404 ? 'The file is no longer on disk.' : `${r.status} ${r.statusText}`);
        text = await r.text();
      })
      .catch((e) => {
        if (e.name !== 'AbortError') error = e.message;
      });
    return () => ctrl.abort();
  });

  /** Where the matched passage sits in the file. */
  const span = $derived.by(() => {
    const chunk = segment?.text?.replace(/…$/, '').trim();
    if (text == null || !chunk) return null;
    let at = text.indexOf(chunk);
    let len = chunk.length;
    if (at < 0) {
      const head = chunk.slice(0, 80);
      at = text.indexOf(head);
      len = Math.min(text.length - at, chunk.length);
    }
    return at < 0 ? null : { at, len };
  });

  $effect(() => {
    if (span && hit) tick().then(() => hit?.scrollIntoView({ block: 'center' }));
  });
</script>

{#snippet marked(s: string)}{#each highlight(s, words) as p, i (i)}{#if p.hit}<mark>{p.text}</mark>{:else}{p.text}{/if}{/each}{/snippet}

<div class="text">
  {#if error === 'big' || (error && segment?.text)}
    {#if error !== 'big'}<p class="note">{error} Showing the matched passage.</p>{:else}<p class="note">This file is large; showing the matched passage. Use "Open" for all of it.</p>{/if}
    <pre><span class="hit">{@render marked(segment?.text ?? '')}</span></pre>
  {:else if error}
    <p class="note">Can't show this file: {error}</p>
  {:else if text == null}
    <div class="loading" aria-label="Loading"><span></span><span></span><span></span></div>
  {:else if span}
    <pre>{text.slice(0, span.at)}<span class="hit" bind:this={hit}>{@render marked(text.slice(span.at, span.at + span.len))}</span>{text.slice(span.at + span.len)}</pre>
  {:else}
    <pre>{@render marked(text)}</pre>
  {/if}
</div>

<style>
  .text {
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    max-height: 68vh;
    overflow: auto;
  }
  pre {
    margin: 0;
    padding: 16px 18px;
    font: 13px/1.6 var(--mono);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    color: var(--text-2);
  }
  .hit {
    display: inline;
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    box-shadow: -6px 0 0 color-mix(in srgb, var(--accent) 14%, transparent);
    border-left: 3px solid var(--accent);
    color: var(--text);
    border-radius: 2px;
    scroll-margin: 80px;
  }
  .note {
    margin: 0;
    padding: 12px 18px 0;
    color: var(--muted);
    font-size: 13px;
  }
  .loading {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 18px;
  }
  .loading span {
    height: 12px;
    border-radius: 4px;
    background: var(--surface-2);
  }
  .loading span:nth-child(2) {
    width: 80%;
  }
  .loading span:nth-child(3) {
    width: 60%;
  }
</style>
