<script lang="ts">
  // Bars of peak levels. `marks` (seconds) are tinted; `played` shows playback progress.
  let {
    peaks,
    duration,
    marks = [],
    played = -1,
  }: { peaks: number[]; duration: number; marks?: [number, number][]; played?: number } = $props();

  const n = $derived(peaks.length);
  const inMark = (i: number) => {
    if (!duration) return false;
    const t = ((i + 0.5) / n) * duration;
    return marks.some(([a, b]) => t >= a && t <= b);
  };
</script>

<svg class="wave" viewBox="0 0 {n * 3} 100" preserveAspectRatio="none" aria-hidden="true">
  {#each peaks as p, i (i)}
    {@const h = Math.max(4, Math.min(1, p) * 96)}
    <rect
      x={i * 3}
      y={(100 - h) / 2}
      width="2"
      height={h}
      rx="1"
      class:mark={inMark(i)}
      class:played={played >= 0 && duration > 0 && ((i + 0.5) / n) * duration <= played}
    />
  {/each}
</svg>

<style>
  .wave {
    display: block;
    width: 100%;
    height: 100%;
  }
  rect {
    fill: var(--wave, var(--border-strong));
    transition: fill var(--fast);
  }
  rect.mark {
    fill: var(--wave-mark, color-mix(in srgb, var(--accent) 70%, transparent));
  }
  rect.played {
    fill: var(--wave-played, var(--text-2));
  }
  rect.mark.played {
    fill: var(--accent);
  }
</style>
