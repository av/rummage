<script lang="ts">
  import {
    X,
    ChevronLeft,
    ChevronRight,
    Sparkles,
    ExternalLink,
    Download,
    Copy,
    FolderOpen,
    FolderSearch,
    Image as ImageIcon,
    Film,
    AudioLines,
    FileType,
    FileText,
    TriangleAlert,
  } from '@lucide/svelte';
  import { tick } from 'svelte';
  import { MediaQuery } from 'svelte/reactivity';
  import { media, type Segment } from '../lib/api';
  import { app } from '../lib/app.svelte';
  import { bytes, clock, dateTime, dirname, kindLabel } from '../lib/format';
  import { filesLink } from '../lib/files';
  import { terms } from '../lib/highlight';
  import ImageView from './preview/ImageView.svelte';
  import MediaPlayer from './preview/MediaPlayer.svelte';
  import DocView from './preview/DocView.svelte';
  import TextView from './preview/TextView.svelte';
  import Exif from './preview/Exif.svelte';

  const p = $derived(app.preview);
  const item = $derived(p?.item ?? null);
  const segments = $derived(p?.detail?.segments ?? []);
  const words = $derived(app.url.like == null ? terms(app.url.q) : []);
  const index = $derived(item ? app.hits.findIndex((h) => h.item.id === item.id) : -1);
  // A side panel beside the results on wide screens; a full-screen (modal) sheet on phones.
  const phone = new MediaQuery('max-width: 759px');
  const icons = { image: ImageIcon, video: Film, audio: AudioLines, doc: FileType, text: FileText };

  let heading = $state<HTMLElement>();
  let imageView = $state<{ escape: () => boolean }>();
  let seekTo = $state<number | null>(null);
  let page = $state(1);

  // Focus moves into the panel when it opens, so keys and screen readers follow.
  let lastId: number | null = null;
  $effect(() => {
    const id = item?.id ?? null;
    if (id === lastId) return;
    const wasOpen = lastId != null;
    lastId = id;
    page = p?.segment?.page ?? 1;
    if (id != null && !wasOpen) tick().then(() => heading?.focus({ preventScroll: true }));
  });

  $effect(() => {
    if (p?.segment?.page) page = p.segment.page;
  });

  /** Matched time windows, best first: the open one plus close runners-up. */
  const marks = $derived.by((): [number, number][] => {
    const timed = segments.filter((s) => s.t_start != null);
    const scored = timed.filter((s) => s.score != null).sort((a, b) => b.score! - a.score!);
    const best = scored[0]?.score ?? 0;
    const pick: Segment[] = scored.filter((s, i) => i < 5 && s.score! >= best - 0.08);
    const open = p?.segment;
    if (open?.t_start != null && !pick.some((s) => s.id === open.id)) pick.unshift(open);
    return pick.map((s) => [s.t_start!, s.t_end ?? s.t_start! + 10]);
  });

  const matches = $derived(
    segments
      .filter((s) => s.score != null && (s.t_start != null || s.page != null))
      .sort((a, b) => b.score! - a.score!)
      .slice(0, 5),
  );

  const filesUrl = $derived(item ? filesLink(app.filesUrl, item.path) : '');

  async function copyPath() {
    if (!item) return;
    try {
      await navigator.clipboard.writeText(item.path);
    } catch {
      const ta = document.createElement('textarea');
      ta.value = item.path;
      ta.style.position = 'fixed';
      ta.style.opacity = '0';
      document.body.appendChild(ta);
      ta.select();
      document.execCommand('copy');
      ta.remove();
    }
    app.say('Path copied');
  }

  export function escape(): boolean {
    return imageView?.escape() ?? false;
  }

  function jump(s: Segment) {
    if (s.t_start != null) seekTo = s.t_start;
    else if (s.page != null) page = s.page;
  }
</script>

{#if p}
  <div class="panel" role="dialog" aria-modal={phone.current} aria-labelledby="preview-title">
    <header class="head">
      <div class="nav">
        <button class="icon-btn" type="button" aria-label="Previous result" title="Previous (Left arrow)" disabled={index <= 0} onclick={() => app.step(-1)}><ChevronLeft size={20} /></button>
        <button class="icon-btn" type="button" aria-label="Next result" title="Next (Right arrow)" disabled={index < 0 || (index >= app.hits.length - 1 && app.done)} onclick={() => app.step(1)}><ChevronRight size={20} /></button>
        {#if index >= 0}<span class="pos num">{index + 1} of {app.hits.length}{app.done ? '' : '+'}</span>{/if}
      </div>
      <button class="icon-btn" type="button" aria-label="Close preview" title="Close (Esc)" onclick={() => app.closePreview()}><X size={20} /></button>
    </header>

    <div class="body">
      {#if !item}
        {#if p.error}
          <div class="state" role="alert"><TriangleAlert size={26} /><p>{p.error}</p></div>
        {:else}
          <div class="loading" aria-label="Loading preview"></div>
        {/if}
      {:else}
        {@const Icon = icons[item.kind]}
        <div class="title">
          <h2 id="preview-title" tabindex="-1" bind:this={heading}><Icon size={18} aria-hidden="true" />{item.name}</h2>
          <p class="path" title={item.path}>&lrm;{dirname(item.path)}&lrm;</p>
        </div>

        {#key item.id}
          <div class="view">
            {#if item.kind === 'image'}
              <ImageView {item} bind:this={imageView} />
            {:else if item.kind === 'video' || item.kind === 'audio'}
              <MediaPlayer {item} start={p.segment?.t_start ?? 0} {marks} bind:seekTo />
            {:else if item.kind === 'doc'}
              <DocView {item} bind:page {segments} />
            {:else}
              <TextView {item} segment={p.segment} passage={segments.find((s) => s.id === p.segment?.id)?.text ?? null} {words} />
            {/if}
          </div>
        {/key}

        <div class="actions">
          <button class="btn primary" type="button" onclick={() => app.moreLike(item)}><Sparkles size={16} />More like this</button>
          <a class="btn" href={media(item.urls.raw)} target="_blank" rel="noopener"><ExternalLink size={16} />Open</a>
          <a class="btn" href={media(item.urls.raw)} download={item.name}><Download size={16} />Download</a>
          <button class="btn" type="button" onclick={copyPath}><Copy size={16} />Copy path</button>
          {#if filesUrl}
            <a class="btn" href={filesUrl} target="_blank" rel="noopener"><FolderOpen size={16} />Open in Files</a>
          {/if}
          <button class="btn" type="button" title="Search only in this folder" onclick={() => app.setFilter({ dir: dirname(item.path) })}><FolderSearch size={16} />Reveal folder</button>
        </div>

        {#if matches.length > 1}
          <section class="matches" aria-label="Best matches in this file">
            <h3>Best matches</h3>
            <div class="mlist">
              {#each matches as s (s.id)}
                <button type="button" class="m" onclick={() => jump(s)}>
                  <span class="at num">{s.page != null ? `p. ${s.page}` : `${clock(s.t_start!)}${s.t_end != null ? `–${clock(s.t_end)}` : ''}`}</span>
                  {#if s.text}<span class="mt">{s.text}</span>{/if}
                </button>
              {/each}
            </div>
          </section>
        {/if}

        <dl class="facts">
          <div><dt>Kind</dt><dd>{kindLabel[item.kind]}{item.mime ? ` · ${item.mime}` : ''}</dd></div>
          <div><dt>Size</dt><dd>{bytes(item.size)}</dd></div>
          <div><dt>Date</dt><dd>{dateTime(item.date)}</dd></div>
          {#if item.mtime !== item.date}<div><dt>Modified</dt><dd>{dateTime(item.mtime)}</dd></div>{/if}
          {#if item.status !== 'indexed'}<div><dt>Index</dt><dd>{item.status}{item.error ? `: ${item.error}` : ''}</dd></div>{/if}
        </dl>
        {#if item.exif && Object.keys(item.exif).length}
          <Exif exif={item.exif} />
        {/if}
      {/if}
    </div>
  </div>
{/if}

<style>
  .panel {
    position: fixed;
    z-index: 40;
    top: 0;
    right: 0;
    bottom: 0;
    width: var(--panel-w);
    display: flex;
    flex-direction: column;
    background: var(--bg-elev);
    border-left: 1px solid var(--border);
    box-shadow: var(--shadow-lg);
    animation: slide-in var(--med) var(--ease);
  }
  @keyframes slide-in {
    from {
      transform: translateX(24px);
      opacity: 0;
    }
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px 12px;
    border-bottom: 1px solid var(--border);
  }
  .nav {
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .pos {
    margin-left: 6px;
    font-size: 13px;
    color: var(--muted);
  }
  .body {
    flex: 1;
    overflow-y: auto;
    padding: 16px 20px 28px;
    display: flex;
    flex-direction: column;
    gap: 16px;
    overscroll-behavior: contain;
  }
  .title h2 {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 0;
    font-size: 17px;
    font-weight: 600;
    line-height: 1.35;
    overflow-wrap: anywhere;
    outline: none;
  }
  .title h2 :global(svg) {
    flex: none;
    color: var(--muted);
  }
  .path {
    margin: 2px 0 0 26px;
    font-size: 13px;
    color: var(--muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    direction: rtl;
    text-align: left;
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }
  .actions a {
    text-decoration: none;
  }
  .matches h3 {
    margin: 0 0 6px;
    font-size: 12px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--muted);
    font-weight: 600;
  }
  .mlist {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .m {
    display: flex;
    gap: 10px;
    align-items: baseline;
    text-align: left;
    border: 0;
    background: transparent;
    padding: 6px 8px;
    border-radius: 8px;
    font-size: 13.5px;
  }
  .m:hover {
    background: var(--surface-2);
  }
  .at {
    flex: none;
    min-width: 88px;
    font-weight: 600;
    color: var(--accent);
  }
  .mt {
    color: var(--text-2);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .facts {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
    gap: 10px 16px;
    margin: 0;
    padding: 14px 0 0;
    border-top: 1px solid var(--border);
    font-size: 13.5px;
  }
  .facts dt {
    color: var(--muted);
    font-size: 12px;
  }
  .facts dd {
    margin: 0;
    overflow-wrap: anywhere;
  }
  .state {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    padding: 60px 10px;
    color: var(--muted);
    text-align: center;
  }
  .loading {
    height: 320px;
    border-radius: var(--radius);
    background: var(--surface-2);
  }
  @media (max-width: 759px) {
    .panel {
      top: 0;
      left: 0;
      width: auto;
      border-left: 0;
      animation-name: sheet-in;
    }
    @keyframes sheet-in {
      from {
        transform: translateY(32px);
        opacity: 0;
      }
    }
    .body {
      padding: 12px 14px 32px;
    }
  }
</style>
