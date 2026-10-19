<script lang="ts">
  import { Play, FileText, FileType, AudioLines, ImageOff, Film } from '@lucide/svelte';
  import { media, type SearchHit } from '../lib/api';
  import { app } from '../lib/app.svelte';
  import { date, dirname, ext, kindLabel, where } from '../lib/format';
  import { highlight } from '../lib/highlight';
  import { waveform, type Wave } from '../lib/waveform';
  import Waveform from './Waveform.svelte';

  let { hit, index, words }: { hit: SearchHit; index: number; words: string[] } = $props();

  const item = $derived(hit.item);
  const seg = $derived(hit.segment);
  const at = $derived(where(seg, item.kind));
  const folder = $derived(dirname(item.path).split('/').pop() || '/');
  // The server marks the words that matched; names without a passage are marked here.
  const parts = $derived(seg?.snippet?.length ? seg.snippet : highlight(seg?.text || item.name, words));

  let loaded = $state(false);
  let failed = $state(false);
  let triedPoster = $state(false);
  let wave = $state<Wave | null | undefined>(undefined);

  // A video shows the matched window's poster, saved while indexing (small, no ffmpeg run), and
  // falls back to rendering that frame.
  const src = $derived.by(() => {
    if (item.kind === 'video' && seg?.t_start != null) {
      const t = Math.floor(seg.t_start);
      return triedPoster ? media(item.urls.preview, { t }) : media(item.urls.thumb, { t });
    }
    return media(item.urls.thumb);
  });

  function onerror() {
    if (item.kind === 'video' && seg?.t_start != null && !triedPoster) triedPoster = true;
    else failed = true;
  }

  function visible(node: HTMLElement) {
    if (item.kind !== 'audio') return;
    const io = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) {
          io.disconnect();
          waveform(item, 56).then((w) => (wave = w));
        }
      },
      { rootMargin: '200px' },
    );
    io.observe(node);
    return { destroy: () => io.disconnect() };
  }

  function open(e: MouseEvent) {
    app.returnFocus = e.currentTarget as HTMLElement;
    app.openIndex(index);
  }

  const label = $derived(`${item.name}, ${kindLabel[item.kind].toLowerCase()}${at ? `, at ${at}` : ''}, in ${folder}`);
</script>

<button
  type="button"
  id="r-{index}"
  class="card k-{item.kind}"
  class:sel={app.selected === index}
  class:open={app.preview?.item?.id === item.id}
  tabindex={app.selected === index ? 0 : -1}
  aria-label={label}
  aria-current={app.preview?.item?.id === item.id ? 'true' : undefined}
  data-id={item.id}
  onclick={open}
  onfocus={() => (app.selected = index)}
  use:visible
>
  <div class="media">
    {#if item.kind === 'image' || item.kind === 'video'}
      {#if failed}
        <span class="placeholder">
          {#if item.kind === 'video'}<Film size={28} />{:else}<ImageOff size={28} />{/if}
        </span>
      {:else}
        <img {src} alt="" loading="lazy" decoding="async" class:loaded onload={() => (loaded = true)} {onerror} />
      {/if}
      {#if item.kind === 'video'}
        <span class="badge"><Play size={12} fill="currentColor" />{at || 'Video'}</span>
      {/if}
    {:else if item.kind === 'audio'}
      <div class="wave">
        {#if wave}
          <Waveform peaks={wave.peaks} duration={wave.duration} marks={seg?.t_start != null ? [[seg.t_start, seg.t_end ?? seg.t_start + 30]] : []} />
        {:else if wave === null}
          <span class="placeholder"><AudioLines size={30} /></span>
        {/if}
      </div>
      <span class="badge"><AudioLines size={12} />{at || 'Audio'}</span>
    {:else}
      <p class="snippet">
        {#each parts as part, i (i)}{#if part.hit}<mark>{part.text}</mark>{:else}{part.text}{/if}{/each}
      </p>
      <span class="badge soft">
        {#if item.kind === 'doc'}<FileType size={12} />{:else}<FileText size={12} />{/if}
        {at || ext(item.name) || kindLabel[item.kind]}
      </span>
    {/if}
  </div>
  <div class="foot">
    <span class="name" title={item.path}>{item.name}</span>
    <span class="meta">{folder} · {date(item.date)}</span>
  </div>
</button>

<style>
  .card {
    display: flex;
    flex-direction: column;
    width: 100%;
    height: 100%;
    padding: 0;
    text-align: left;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    overflow: hidden;
    transition:
      transform var(--fast) var(--ease),
      box-shadow var(--fast),
      border-color var(--fast);
    contain: layout paint;
    scroll-margin: 132px 0 16px;
  }
  .card:hover {
    transform: translateY(-1px);
    box-shadow: var(--shadow-lg);
    border-color: var(--border-strong);
  }
  .card.sel {
    border-color: var(--border-strong);
  }
  .card.open {
    border-color: var(--accent);
    box-shadow: 0 0 0 2px var(--accent);
  }
  .card:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .media {
    position: relative;
    aspect-ratio: 4 / 3;
    background: var(--surface-2);
    overflow: hidden;
  }
  img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
    opacity: 0;
    filter: blur(14px);
    transform: scale(1.06);
    transition:
      opacity 320ms var(--ease),
      filter 420ms var(--ease),
      transform 420ms var(--ease);
  }
  img.loaded {
    opacity: 1;
    filter: none;
    transform: none;
  }
  .media::before {
    content: '';
    position: absolute;
    inset: 0;
    background: linear-gradient(110deg, transparent 30%, color-mix(in srgb, var(--surface-3) 60%, transparent) 50%, transparent 70%);
    background-size: 220% 100%;
    animation: shimmer 1.4s linear infinite;
    pointer-events: none;
  }
  .media:has(img.loaded)::before,
  .k-doc .media::before,
  .k-text .media::before,
  .k-audio .media::before {
    display: none;
  }
  @keyframes shimmer {
    from {
      background-position: 120% 0;
    }
    to {
      background-position: -100% 0;
    }
  }
  .placeholder {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    color: var(--muted);
  }
  .badge {
    position: absolute;
    left: 8px;
    bottom: 8px;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 22px;
    padding: 0 8px;
    border-radius: 6px;
    background: rgb(10 12 16 / 0.72);
    color: #fff;
    font-size: 12px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    backdrop-filter: blur(6px);
  }
  .badge.soft {
    background: var(--surface);
    color: var(--muted);
    border: 1px solid var(--border);
    backdrop-filter: none;
  }
  .wave {
    position: absolute;
    inset: 22% 10px 30%;
  }
  .k-audio .media {
    background: linear-gradient(180deg, var(--surface-2), var(--surface));
  }
  .k-doc .media,
  .k-text .media {
    background: var(--surface);
    border-bottom: 1px solid var(--border);
  }
  .snippet {
    margin: 0;
    padding: 12px 14px 0;
    height: calc(100% - 34px);
    font-size: 13px;
    line-height: 1.5;
    color: var(--text-2);
    overflow: hidden;
    overflow-wrap: anywhere;
    -webkit-mask-image: linear-gradient(180deg, #000 70%, transparent);
    mask-image: linear-gradient(180deg, #000 70%, transparent);
  }
  .k-text .snippet {
    font-family: var(--mono);
    font-size: 12px;
  }
  .foot {
    display: flex;
    flex-direction: column;
    gap: 1px;
    padding: 8px 12px 10px;
    min-width: 0;
  }
  .name {
    font-size: 13.5px;
    font-weight: 550;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .meta {
    font-size: 12px;
    color: var(--muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
