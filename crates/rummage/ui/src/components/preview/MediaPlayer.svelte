<script lang="ts">
  // Video or audio with its own controls, so the matched windows can be marked on the scrubber.
  import { Play, Pause, Volume2, VolumeX, Maximize, AudioLines } from '@lucide/svelte';
  import { media, type Item } from '../../lib/api';
  import { clock } from '../../lib/format';
  import { waveform, type Wave } from '../../lib/waveform';
  import Waveform from '../Waveform.svelte';

  let {
    item,
    start = 0,
    marks = [],
    seekTo = $bindable(null),
  }: { item: Item; start?: number; marks?: [number, number][]; seekTo?: number | null } = $props();

  const isVideo = $derived(item.kind === 'video');
  let el = $state<HTMLMediaElement>();
  let box = $state<HTMLDivElement>();
  let t = $state(0);
  let dur = $state(0);
  let paused = $state(true);
  let muted = $state(false);
  let wave = $state<Wave | null>(null);
  let failed = $state(false);
  let dragging = false;

  $effect(() => {
    if (!isVideo) waveform(item, 300, true).then((w) => (wave = w));
  });

  $effect(() => {
    if (seekTo != null && el) {
      el.currentTime = seekTo;
      el.play().catch(() => {});
      seekTo = null;
    }
  });

  const length = $derived(dur || wave?.duration || 0);

  function onloaded() {
    if (!el) return;
    dur = el.duration;
    if (start > 0 && start < el.duration) el.currentTime = start;
    el.play().catch(() => {});
  }

  function toggle() {
    if (!el) return;
    if (el.paused) el.play().catch(() => {});
    else el.pause();
  }

  function seekAt(e: PointerEvent, track: HTMLElement) {
    if (!el || !length) return;
    const r = track.getBoundingClientRect();
    el.currentTime = Math.max(0, Math.min(1, (e.clientX - r.left) / r.width)) * length;
  }

  function down(e: PointerEvent) {
    const track = e.currentTarget as HTMLElement;
    dragging = true;
    track.setPointerCapture(e.pointerId);
    seekAt(e, track);
  }
  function move(e: PointerEvent) {
    if (dragging) seekAt(e, e.currentTarget as HTMLElement);
  }
  function up() {
    dragging = false;
  }

  function trackKey(e: KeyboardEvent) {
    if (!el) return;
    const steps: Record<string, number> = { ArrowLeft: -5, ArrowRight: 5, ArrowDown: -5, ArrowUp: 5, PageDown: -30, PageUp: 30 };
    if (e.key in steps) el.currentTime = Math.max(0, Math.min(length, el.currentTime + steps[e.key]));
    else if (e.key === 'Home') el.currentTime = 0;
    else if (e.key === 'End') el.currentTime = length;
    else if (e.key === ' ' || e.key === 'k') toggle();
    else return;
    e.preventDefault();
    e.stopPropagation();
  }

  function fullscreen() {
    if (document.fullscreenElement) document.exitFullscreen();
    else box?.requestFullscreen?.();
  }
</script>

<div class="player" class:video={isVideo} bind:this={box}>
  {#if isVideo}
    <!-- svelte-ignore a11y_media_has_caption -->
    <video
      bind:this={el}
      src={media(item.urls.raw)}
      poster={media(item.urls.preview, { t: Math.floor(start) })}
      preload="metadata"
      playsinline
      bind:currentTime={t}
      bind:paused
      bind:muted
      onloadedmetadata={onloaded}
      onerror={() => (failed = true)}
      onclick={toggle}
    ></video>
  {:else}
    <audio
      bind:this={el}
      src={media(item.urls.raw)}
      preload="metadata"
      bind:currentTime={t}
      bind:paused
      bind:muted
      onloadedmetadata={onloaded}
      onerror={() => (failed = true)}
    ></audio>
  {/if}
  {#if failed}
    <p class="failed">This file can't be played here. It may be gone, or in a format the browser can't play. Try "Open".</p>
  {/if}

  <div class="controls">
    <button class="icon-btn play" type="button" aria-label={paused ? 'Play' : 'Pause'} onclick={toggle}>
      {#if paused}<Play size={18} fill="currentColor" />{:else}<Pause size={18} fill="currentColor" />{/if}
    </button>
    <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
    <div
      class="track"
      class:wavy={!isVideo}
      role="slider"
      tabindex="0"
      aria-label="Seek"
      aria-valuemin={0}
      aria-valuemax={Math.round(length)}
      aria-valuenow={Math.round(t)}
      aria-valuetext="{clock(t)} of {clock(length)}"
      onpointerdown={down}
      onpointermove={move}
      onpointerup={up}
      onpointercancel={up}
      onkeydown={trackKey}
    >
      {#if !isVideo}
        {#if wave}
          <Waveform peaks={wave.peaks} duration={wave.duration} {marks} played={t} />
        {:else}
          <span class="flat"><AudioLines size={16} /></span>
        {/if}
      {:else}
        <span class="rail"></span>
        <span class="fill" style:width="{length ? (100 * t) / length : 0}%"></span>
      {/if}
      {#if length && (isVideo || !wave)}
        {#each marks as [a, b], i (i)}
          <span class="mark" style:left="{(100 * a) / length}%" style:width="{Math.max(0.8, (100 * (b - a)) / length)}%" title="Match at {clock(a)}"></span>
        {/each}
      {/if}
      {#if length}<span class="head" style:left="{(100 * t) / length}%"></span>{/if}
    </div>
    <span class="time num">{clock(t)} / {clock(length)}</span>
    <button class="icon-btn" type="button" aria-label={muted ? 'Unmute' : 'Mute'} onclick={() => (muted = !muted)}>
      {#if muted}<VolumeX size={18} />{:else}<Volume2 size={18} />{/if}
    </button>
    {#if isVideo}
      <button class="icon-btn" type="button" aria-label="Full screen" onclick={fullscreen}><Maximize size={18} /></button>
    {/if}
  </div>
</div>

<style>
  .player {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  video {
    width: 100%;
    max-height: 62vh;
    background: #000;
    border-radius: var(--radius);
    display: block;
    cursor: pointer;
  }
  .player:fullscreen {
    background: #000;
    justify-content: center;
    padding: 16px;
  }
  .player:fullscreen video {
    max-height: calc(100vh - 80px);
  }
  .controls {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .play {
    background: var(--text);
    color: var(--bg);
    border-radius: 999px;
  }
  .play:hover {
    background: var(--text);
    color: var(--bg);
    filter: brightness(1.15);
  }
  .track {
    position: relative;
    flex: 1;
    height: 28px;
    cursor: pointer;
    touch-action: none;
    border-radius: 6px;
  }
  .track.wavy {
    height: 64px;
  }
  .rail,
  .fill {
    position: absolute;
    left: 0;
    top: 50%;
    height: 4px;
    margin-top: -2px;
    border-radius: 999px;
  }
  .rail {
    right: 0;
    background: var(--surface-3);
  }
  .fill {
    background: var(--text-2);
  }
  .mark {
    position: absolute;
    top: 50%;
    height: 10px;
    margin-top: -5px;
    border-radius: 3px;
    background: color-mix(in srgb, var(--accent) 75%, transparent);
    pointer-events: none;
  }
  .wavy .mark {
    height: 100%;
    top: 0;
    margin-top: 0;
    opacity: 0.25;
  }
  .head {
    position: absolute;
    top: 50%;
    width: 12px;
    height: 12px;
    margin: -6px 0 0 -6px;
    border-radius: 999px;
    background: var(--text);
    box-shadow: 0 0 0 2px var(--surface);
    pointer-events: none;
  }
  .wavy .head {
    width: 2px;
    height: 100%;
    top: 0;
    margin: 0 0 0 -1px;
    border-radius: 1px;
    box-shadow: none;
  }
  .flat {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    color: var(--muted);
    border-top: 2px solid var(--surface-3);
    top: 50%;
  }
  .time {
    font-size: 12.5px;
    color: var(--muted);
    min-width: 88px;
    text-align: right;
  }
  .failed {
    margin: 0;
    color: var(--muted);
    font-size: 14px;
  }
</style>
