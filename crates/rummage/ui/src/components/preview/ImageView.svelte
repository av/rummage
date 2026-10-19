<script lang="ts">
  import { ZoomIn, ZoomOut, ImageOff } from '@lucide/svelte';
  import { media, type Item } from '../../lib/api';

  let { item }: { item: Item } = $props();

  const ZOOM = 2.5;
  let zoomed = $state(false);
  let loaded = $state(false);
  let failed = $state(false);
  let ox = $state(50);
  let oy = $state(50);
  let frame = $state<HTMLDivElement>();
  let drag: { x: number; y: number; ox: number; oy: number; moved: boolean } | null = null;

  function toggle(e?: MouseEvent) {
    if (e && frame && !zoomed) {
      const r = frame.getBoundingClientRect();
      ox = ((e.clientX - r.left) / r.width) * 100;
      oy = ((e.clientY - r.top) / r.height) * 100;
    } else if (!zoomed) {
      ox = oy = 50;
    }
    zoomed = !zoomed;
  }

  function down(e: PointerEvent) {
    if (!zoomed) return;
    drag = { x: e.clientX, y: e.clientY, ox, oy, moved: false };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }
  function move(e: PointerEvent) {
    if (!drag || !frame) return;
    const r = frame.getBoundingClientRect();
    const dx = ((e.clientX - drag.x) / r.width) * 100;
    const dy = ((e.clientY - drag.y) / r.height) * 100;
    if (Math.abs(dx) + Math.abs(dy) > 1) drag.moved = true;
    ox = Math.max(0, Math.min(100, drag.ox - dx / (ZOOM - 1)));
    oy = Math.max(0, Math.min(100, drag.oy - dy / (ZOOM - 1)));
  }
  function click(e: MouseEvent) {
    const moved = drag?.moved;
    drag = null;
    if (!moved) toggle(e);
  }

  /** Esc unzooms before it closes the preview. */
  export function escape(): boolean {
    if (!zoomed) return false;
    zoomed = false;
    return true;
  }
</script>

<div class="wrap">
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div
    class="frame"
    class:zoomed
    bind:this={frame}
    onpointerdown={down}
    onpointermove={move}
    onclick={click}
  >
    <img class="under" src={media(item.urls.thumb)} alt="" aria-hidden="true" class:hide={loaded} />
    {#if failed}
      <div class="failed"><ImageOff size={28} /><span>The full image can't be shown. Try "Open".</span></div>
    {:else}
      <img
        class="full"
        class:loaded
        src={media(item.urls.preview)}
        alt={item.name}
        decoding="async"
        style:transform-origin="{ox}% {oy}%"
        onload={() => (loaded = true)}
        onerror={() => (failed = true)}
      />
    {/if}
  </div>
  <button class="icon-btn zoom" type="button" aria-label={zoomed ? 'Zoom out' : 'Zoom in'} aria-pressed={zoomed} onclick={() => toggle()}>
    {#if zoomed}<ZoomOut size={18} />{:else}<ZoomIn size={18} />{/if}
  </button>
</div>

<style>
  .wrap {
    position: relative;
  }
  .frame {
    position: relative;
    display: grid;
    place-items: center;
    min-height: 240px;
    max-height: 66vh;
    aspect-ratio: 4 / 3;
    border-radius: var(--radius);
    background: var(--surface-2);
    overflow: hidden;
    cursor: zoom-in;
    touch-action: pan-y;
  }
  .frame.zoomed {
    cursor: grab;
    touch-action: none;
  }
  .frame img {
    grid-area: 1 / 1;
    max-width: 100%;
    max-height: 100%;
    width: 100%;
    height: 100%;
    object-fit: contain;
    user-select: none;
    -webkit-user-drag: none;
  }
  .under {
    filter: blur(16px);
    transform: scale(1.04);
    transition: opacity 300ms;
  }
  .under.hide {
    opacity: 0;
  }
  .full {
    opacity: 0;
    transition:
      opacity 260ms var(--ease),
      transform 260ms var(--ease);
  }
  .full.loaded {
    opacity: 1;
  }
  .zoomed .full {
    transform: scale(2.5);
  }
  .zoom {
    position: absolute;
    right: 10px;
    bottom: 10px;
    background: rgb(10 12 16 / 0.6);
    color: #fff;
    backdrop-filter: blur(6px);
  }
  .zoom:hover {
    background: rgb(10 12 16 / 0.8);
    color: #fff;
  }
  .failed {
    grid-area: 1 / 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    color: var(--muted);
    font-size: 14px;
    z-index: 1;
  }
</style>
