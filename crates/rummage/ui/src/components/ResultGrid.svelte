<script lang="ts">
  import { SearchX, WifiOff, TriangleAlert, FolderPlus } from '@lucide/svelte';
  import { app, PAGE } from '../lib/app.svelte';
  import { terms } from '../lib/highlight';
  import ResultCard from './ResultCard.svelte';

  let { focusSearch }: { focusSearch: () => void } = $props();

  let grid = $state<HTMLUListElement>();
  let sentinel = $state<HTMLDivElement>();
  const words = $derived(app.url.like == null ? terms(app.url.q) : []);
  /** Show placeholders only when a first search takes a moment. */
  let slow = $state(false);

  $effect(() => {
    if (app.loading && !app.hits.length) {
      const t = setTimeout(() => (slow = true), 120);
      return () => clearTimeout(t);
    }
    slow = false;
  });

  $effect(() => {
    if (!sentinel) return;
    const io = new IntersectionObserver((e) => e.some((x) => x.isIntersecting) && app.loadMore(), { rootMargin: '900px 0px' });
    io.observe(sentinel);
    return () => io.disconnect();
  });

  // Keep the selected card in view as the preview moves through results.
  $effect(() => {
    const id = app.preview?.item?.id;
    if (id == null) return;
    const i = app.hits.findIndex((h) => h.item.id === id);
    if (i >= 0) document.getElementById(`r-${i}`)?.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
  });

  function columns(): number {
    if (!grid) return 1;
    return getComputedStyle(grid).gridTemplateColumns.split(' ').filter(Boolean).length || 1;
  }

  function onkeydown(e: KeyboardEvent) {
    const el = (e.target as HTMLElement).closest<HTMLElement>('.card');
    if (!el) return;
    const i = Number(el.id.slice(2));
    const cols = columns();
    let next = i;
    switch (e.key) {
      case 'ArrowRight':
        next = i + 1;
        break;
      case 'ArrowLeft':
        next = i - 1;
        break;
      case 'ArrowDown':
        next = i + cols;
        break;
      case 'ArrowUp':
        if (i - cols < 0) {
          e.preventDefault();
          focusSearch();
          return;
        }
        next = i - cols;
        break;
      case 'Home':
        next = 0;
        break;
      case 'End':
        next = app.hits.length - 1;
        break;
      default:
        return;
    }
    e.preventDefault();
    e.stopPropagation();
    next = Math.max(0, Math.min(app.hits.length - 1, next));
    if (next >= app.hits.length - cols * 2) app.loadMore();
    app.selected = next;
    document.getElementById(`r-${next}`)?.focus();
  }
</script>

<section class="results" aria-label="Results" aria-busy={app.loading}>
  {#if app.error && !app.hits.length}
    <div class="state" role="alert">
      {#if app.error.code === 'network'}
        <WifiOff size={28} />
        <h2>Can't reach rummage</h2>
        <p>Check that <code>rummage serve</code> is running and reachable, then try again.</p>
        <button class="btn primary" type="button" onclick={() => app.runSearch()}>Try again</button>
      {:else if app.error.code === 'unsupported'}
        <TriangleAlert size={28} />
        <h2>Can't search by this file</h2>
        <p>{app.error.message}</p>
        <button class="btn" type="button" onclick={() => app.home()}>Start over</button>
      {:else if app.error.code === 'not_ready' || app.error.code === 'conflict'}
        <TriangleAlert size={28} />
        <h2>Not ready yet</h2>
        <p>{app.error.message}. It will work once indexing reaches it.</p>
      {:else if app.error.code === 'unauthorized'}
        <TriangleAlert size={28} />
        <h2>Access token needed</h2>
        <p>Enter the server's access token to search.</p>
        <button class="btn primary" type="button" onclick={() => (app.needToken = true)}>Enter token</button>
      {:else}
        <TriangleAlert size={28} />
        <h2>The search failed</h2>
        <p>{app.error.message}</p>
        <button class="btn" type="button" onclick={() => app.runSearch()}>Try again</button>
      {/if}
    </div>
  {:else if !app.hits.length && !app.loading}
    <div class="state">
      {#if app.foldersLoaded && !app.folders.length}
        <FolderPlus size={28} />
        <h2>Nothing is indexed yet</h2>
        <p>Add a folder and rummage starts indexing it right away.</p>
        <button class="btn primary" type="button" onclick={() => app.openDrawer()}>Add a folder</button>
      {:else}
        <SearchX size={28} />
        <h2>No matches{app.url.q ? ` for “${app.url.q}”` : ''}</h2>
        {#if app.filtered}
          <p>Filters may be hiding results.</p>
          <button class="btn primary" type="button" onclick={() => app.clearFilters()}>Clear filters</button>
        {:else}
          <p>Try other words, or describe what is in the picture or recording.</p>
        {/if}
      {/if}
    </div>
  {:else}
    <!-- Arrow keys move between the result buttons inside (roving focus). -->
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <ul id="results" class="grid" class:stale={app.loading && app.hits.length > 0} bind:this={grid} {onkeydown}>
      {#each app.hits as hit, i (hit.item.id)}
        <li><ResultCard {hit} index={i} {words} /></li>
      {/each}
      {#if (slow && !app.hits.length) || app.loadingMore}
        {#each { length: app.hits.length ? 12 : PAGE / 3 } as _, i (i)}
          <li class="skeleton" aria-hidden="true"><span></span><i></i></li>
        {/each}
      {/if}
    </ul>
    <div class="sentinel" bind:this={sentinel}></div>
    {#if app.done && app.hits.length > PAGE}
      <p class="end">That's everything. <button type="button" class="link" onclick={() => window.scrollTo({ top: 0, behavior: 'smooth' })}>Back to top</button></p>
    {/if}
  {/if}
</section>

<style>
  .results {
    padding: 4px 20px 48px;
    max-width: 1600px;
    margin: 0 auto;
  }
  .grid {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(196px, 1fr));
    gap: 14px;
    transition: opacity var(--fast);
  }
  .grid.stale {
    opacity: 0.6;
    transition-delay: 150ms;
  }
  .grid li {
    min-width: 0;
  }
  .skeleton {
    border: 1px solid var(--border);
    border-radius: var(--radius);
    overflow: hidden;
  }
  .skeleton span,
  .skeleton i {
    display: block;
    background: linear-gradient(110deg, var(--surface-2) 30%, var(--surface-3) 50%, var(--surface-2) 70%);
    background-size: 220% 100%;
    animation: shimmer 1.4s linear infinite;
  }
  .skeleton span {
    aspect-ratio: 4 / 3;
  }
  .skeleton i {
    height: 52px;
    background: var(--surface);
    animation: none;
  }
  @keyframes shimmer {
    from {
      background-position: 120% 0;
    }
    to {
      background-position: -100% 0;
    }
  }
  .sentinel {
    height: 1px;
  }
  .state {
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
    gap: 6px;
    padding: 12vh 16px;
    color: var(--muted);
  }
  .state h2 {
    margin: 8px 0 0;
    font-size: 18px;
    color: var(--text);
    font-weight: 600;
  }
  .state p {
    margin: 0 0 10px;
    max-width: 440px;
  }
  .state code {
    font: 13px var(--mono);
  }
  .end {
    text-align: center;
    color: var(--muted);
    font-size: 13px;
    margin: 28px 0 0;
  }
  .link {
    border: 0;
    background: none;
    color: var(--accent);
    padding: 0;
    font-size: inherit;
    text-decoration: underline;
  }
  @media (max-width: 759px) {
    .results {
      padding: 4px 12px 40px;
    }
    .grid {
      grid-template-columns: repeat(auto-fill, minmax(148px, 1fr));
      gap: 10px;
    }
  }
</style>
