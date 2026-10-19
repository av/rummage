<script lang="ts">
  import { FileSearch } from '@lucide/svelte';
  import { app } from '../lib/app.svelte';

  let depth = $state(0);
  const hasFiles = (e: DragEvent) => !!e.dataTransfer && [...e.dataTransfer.types].includes('Files');

  function enter(e: DragEvent) {
    if (!hasFiles(e)) return;
    e.preventDefault();
    depth++;
  }
  function leave(e: DragEvent) {
    if (!hasFiles(e)) return;
    depth = Math.max(0, depth - 1);
  }
  function over(e: DragEvent) {
    if (!hasFiles(e)) return;
    e.preventDefault();
    e.dataTransfer!.dropEffect = 'copy';
  }
  function drop(e: DragEvent) {
    if (!hasFiles(e)) return;
    e.preventDefault();
    depth = 0;
    const f = e.dataTransfer?.files[0];
    if (f) app.searchByFile(f);
  }
  function paste(e: ClipboardEvent) {
    // Pasting text stays normal; pasting a file (e.g. a screenshot) anywhere searches by it.
    const f = e.clipboardData?.files[0];
    if (!f) return;
    e.preventDefault();
    app.searchByFile(f);
  }
</script>

<svelte:window ondragenter={enter} ondragleave={leave} ondragover={over} ondrop={drop} onpaste={paste} />

{#if depth > 0}
  <div class="drop" aria-hidden="true">
    <div class="inner">
      <FileSearch size={36} />
      <strong>Drop to search by this file</strong>
      <span>Images, audio and text files work</span>
    </div>
  </div>
{/if}

<style>
  .drop {
    position: fixed;
    inset: 0;
    z-index: 80;
    display: grid;
    place-items: center;
    padding: 16px;
    background: color-mix(in srgb, var(--bg) 78%, transparent);
    backdrop-filter: blur(4px);
    pointer-events: none;
    animation: fade 120ms ease;
  }
  @keyframes fade {
    from {
      opacity: 0;
    }
  }
  .inner {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    width: min(520px, 100%);
    padding: 48px 24px;
    border: 2px dashed var(--accent);
    border-radius: 20px;
    background: var(--surface);
    color: var(--text);
    text-align: center;
  }
  .inner :global(svg) {
    color: var(--accent);
  }
  strong {
    font-size: 17px;
  }
  span {
    color: var(--muted);
    font-size: 14px;
  }
</style>
