<script lang="ts">
  import { app } from '../lib/app.svelte';
  import { eta, num, plural } from '../lib/format';

  const s = $derived(app.status);
  const noFolders = $derived(app.foldersLoaded && app.folders.length === 0);
  const pending = $derived(s ? s.queued || s.totals.pending : 0);
  const working = $derived(!!s && (s.state === 'indexing' || s.state === 'scanning' || pending > 0) && s.state !== 'paused');
  const pct = $derived(s && s.totals.files ? Math.min(100, (100 * s.totals.indexed) / s.totals.files) : 0);
</script>

<p class="status" id="status-line" aria-live="polite">
  {#if app.offline}
    <span class="dot bad" aria-hidden="true"></span>
    <span>Can't reach the server. Is <code>rummage serve</code> running?</span>
    <button class="link" type="button" onclick={() => app.loadIndex()}>Retry</button>
  {:else if noFolders}
    <span>No folders yet.</span>
    <button class="link" type="button" onclick={() => app.openDrawer()}>Add a folder to start</button>
  {:else if !s}
    <span class="muted">Loading…</span>
  {:else if s.state === 'paused'}
    <span class="dot" aria-hidden="true"></span>
    <span><span class="num">{num(s.totals.indexed)}</span> items · indexing paused</span>
  {:else if working}
    <span class="dot live" aria-hidden="true"></span>
    <span>
      {s.state === 'scanning' && !pending ? 'Scanning folders' : 'Indexing'} ·
      <span class="num">{num(s.totals.indexed)}</span> of <span class="num">{num(s.totals.files)}</span>
      {#if s.eta_secs}· {eta(s.eta_secs)} left{/if}
    </span>
    <span class="bar" role="progressbar" aria-label="Indexing progress" aria-valuenow={Math.round(pct)} aria-valuemin="0" aria-valuemax="100">
      <span style:width="{pct}%"></span>
    </span>
  {:else}
    <span class="dot ok" aria-hidden="true"></span>
    <span><span class="num">{num(s.totals.indexed)}</span> items · up to date</span>
  {/if}
  {#if s && s.totals.errors && !app.offline}
    <button class="link warn" type="button" onclick={() => app.openDrawer()}>{plural(s.totals.errors, 'file')} could not be read</button>
  {/if}
</p>

<style>
  .status {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: center;
    gap: 4px 8px;
    margin: 0;
    color: var(--muted);
    font-size: 13.5px;
    min-height: 22px;
  }
  .muted {
    color: var(--muted);
  }
  code {
    font: 12.5px var(--mono);
  }
  .dot {
    width: 7px;
    height: 7px;
    border-radius: 999px;
    background: var(--muted);
    flex: none;
  }
  .dot.ok {
    background: var(--ok);
  }
  .dot.bad {
    background: var(--danger);
  }
  .dot.live {
    background: var(--accent);
    animation: pulse 1.6s ease-in-out infinite;
  }
  @keyframes pulse {
    50% {
      opacity: 0.35;
    }
  }
  .bar {
    display: inline-block;
    width: 80px;
    height: 4px;
    border-radius: 999px;
    background: var(--surface-3);
    overflow: hidden;
  }
  .bar span {
    display: block;
    height: 100%;
    background: var(--accent);
    transition: width var(--med) var(--ease);
  }
  .link {
    border: 0;
    background: none;
    padding: 0;
    color: var(--accent);
    text-decoration: underline;
    text-underline-offset: 2px;
    font-size: inherit;
  }
  .link.warn {
    color: var(--warn);
  }
</style>
