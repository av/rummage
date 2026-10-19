<script lang="ts">
  import { FolderCog, Sun, Moon, Monitor, X } from '@lucide/svelte';
  import { onMount } from 'svelte';
  import { app } from './lib/app.svelte';
  import { cycleTheme, theme } from './lib/theme.svelte';
  import { num } from './lib/format';
  import SearchBox from './components/SearchBox.svelte';
  import StatusLine from './components/StatusLine.svelte';
  import FilterBar from './components/FilterBar.svelte';
  import ResultGrid from './components/ResultGrid.svelte';
  import Preview from './components/Preview.svelte';
  import FoldersDrawer from './components/FoldersDrawer.svelte';
  import DropOverlay from './components/DropOverlay.svelte';
  import TokenPrompt from './components/TokenPrompt.svelte';

  let input = $state<HTMLInputElement>();
  let box = $state<HTMLDivElement>();
  let gear = $state<HTMLButtonElement>();
  let preview = $state<{ escape: () => boolean }>();

  // The search box glides between the centre and the top bar (FLIP: measure, then animate the
  // difference), so the switch to results never jumps.
  let before: DOMRect | null = null;
  $effect.pre(() => {
    void app.active;
    before = box?.getBoundingClientRect() ?? null;
  });
  $effect(() => {
    void app.active;
    if (!box || !before || matchMedia('(prefers-reduced-motion: reduce)').matches) return;
    const after = box.getBoundingClientRect();
    const dy = before.top - after.top;
    if (Math.abs(dy) < 1) return;
    box.animate([{ transform: `translateY(${dy}px)` }, { transform: 'none' }], {
      duration: 260,
      easing: 'cubic-bezier(0.2, 0.8, 0.2, 1)',
    });
  });

  onMount(() => {
    app.init();
    input?.focus();
  });

  function focusSearch() {
    input?.focus();
    input?.select();
  }

  function closeDrawer() {
    app.drawerOpen = false;
    gear?.focus();
  }

  const typing = (t: EventTarget | null) => t instanceof HTMLElement && (t.matches('input, textarea, select') || t.isContentEditable);

  function onkeydown(e: KeyboardEvent) {
    if (e.defaultPrevented || e.metaKey || e.ctrlKey || e.altKey) return;
    if (e.key === 'Escape') {
      if (app.needToken) return;
      if (app.drawerOpen) {
        e.preventDefault();
        closeDrawer();
      } else if (app.preview) {
        e.preventDefault();
        if (!preview?.escape()) app.closePreview();
      }
      return;
    }
    if (app.drawerOpen || app.needToken) return;
    if (e.key === '/' && !typing(e.target)) {
      e.preventDefault();
      focusSearch();
      return;
    }
    if (app.preview && !typing(e.target) && !(e.target as HTMLElement).closest?.('[role="slider"], video, audio')) {
      if (e.key === 'ArrowRight') {
        e.preventDefault();
        app.step(1);
      } else if (e.key === 'ArrowLeft') {
        e.preventDefault();
        app.step(-1);
      }
    }
  }

  const themeLabel = $derived({ system: 'Theme: follows the system', light: 'Theme: light', dark: 'Theme: dark' }[theme.value]);
  const announce = $derived(
    app.loading ? '' : app.active && !app.error ? (app.hits.length ? `${num(app.hits.length)}${app.done ? '' : ' or more'} results` : 'No results') : '',
  );
</script>

<svelte:window {onkeydown} />

<div class="app" class:active={app.active} class:previewing={!!app.preview}>
  <div class="tools">
    <button class="icon-btn" type="button" aria-label={themeLabel} title={themeLabel} onclick={cycleTheme}>
      {#if theme.value === 'light'}<Sun size={19} />{:else if theme.value === 'dark'}<Moon size={19} />{:else}<Monitor size={19} />{/if}
    </button>
    <button class="icon-btn" type="button" aria-label="Folders and settings" title="Folders and settings" bind:this={gear} onclick={() => app.openDrawer()}>
      <FolderCog size={20} />
    </button>
  </div>

  <header class="top">
    <div class="inner">
      <button type="button" class="brand" aria-label="rummage, start over" onclick={() => (app.home(), input?.focus())}>
        <svg viewBox="0 0 32 32" aria-hidden="true"><rect width="32" height="32" rx="9" fill="var(--accent)" /><circle cx="14" cy="14" r="6.5" fill="none" stroke="var(--accent-text)" stroke-width="3" /><path d="M19 19l6 6" stroke="var(--accent-text)" stroke-width="3" stroke-linecap="round" /></svg>
        <span>rummage</span>
      </button>
      <div class="search" bind:this={box}><SearchBox bind:input /></div>
    </div>
    {#if app.active}
      <FilterBar />
    {/if}
  </header>

  {#if !app.active}
    <div class="landing">
      <StatusLine />
      {#if app.recent.length}
        <div class="recent" aria-label="Recent searches">
          {#each app.recent as r (r)}
            <button type="button" class="rq" onclick={() => (app.setQuery(r), input?.focus())}>{r}</button>
          {/each}
          <button type="button" class="forget" aria-label="Clear recent searches" title="Clear recent searches" onclick={() => app.forgetRecent()}><X size={14} /></button>
        </div>
      {/if}
      <p class="tips"><kbd>/</kbd> to search from anywhere · drop or paste a file to find similar ones</p>
    </div>
  {:else}
    <main>
      <ResultGrid {focusSearch} />
    </main>
  {/if}
</div>

<Preview bind:this={preview} />
{#if app.drawerOpen}
  <FoldersDrawer onclose={closeDrawer} />
{/if}
<DropOverlay />
{#if app.needToken}
  <TokenPrompt />
{/if}
{#if app.toast}
  <div class="toast" role="status">{app.toast}</div>
{/if}
<div class="sr-only" aria-live="polite">{announce}</div>

<style>
  .app {
    min-height: 100vh;
    transition: padding-right var(--med) var(--ease);
  }
  .tools {
    position: fixed;
    top: 12px;
    right: 14px;
    z-index: 30;
    display: flex;
    gap: 2px;
  }
  .top {
    position: sticky;
    top: 0;
    z-index: 20;
    padding-top: max(20vh, 72px);
    transition: background var(--med);
  }
  .inner {
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 22px;
    max-width: 680px;
    margin: 0 auto;
    padding: 0 20px 12px;
  }
  .search {
    width: 100%;
  }
  .brand {
    display: inline-flex;
    align-items: center;
    gap: 10px;
    border: 0;
    background: none;
    padding: 4px 8px;
    border-radius: 10px;
    font-size: 30px;
    font-weight: 700;
    letter-spacing: -0.03em;
    color: var(--text);
  }
  .brand svg {
    width: 36px;
    height: 36px;
  }
  .active .top {
    padding-top: 10px;
    background: color-mix(in srgb, var(--bg) 88%, transparent);
    backdrop-filter: saturate(1.4) blur(12px);
    border-bottom: 1px solid var(--border);
  }
  .active .inner {
    flex-direction: row;
    max-width: 1600px;
    gap: 12px;
    padding-right: 104px;
    --box-h: 46px;
    --box-font: 16px;
  }
  .active .search {
    max-width: 760px;
  }
  .active .brand {
    font-size: 0;
    gap: 0;
    padding: 4px;
  }
  .active .brand svg {
    width: 30px;
    height: 30px;
  }
  .active .brand span {
    display: none;
  }
  .landing {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 14px;
    max-width: 680px;
    margin: 4px auto 0;
    padding: 0 20px 40px;
    animation: fade var(--med) var(--ease);
  }
  @keyframes fade {
    from {
      opacity: 0;
    }
  }
  .recent {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: 6px;
  }
  .rq {
    height: 30px;
    padding: 0 12px;
    border-radius: 999px;
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text-2);
    font-size: 13.5px;
    max-width: 240px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .rq:hover {
    border-color: var(--border-strong);
    color: var(--text);
  }
  .forget {
    display: grid;
    place-items: center;
    width: 30px;
    height: 30px;
    border: 0;
    border-radius: 999px;
    background: transparent;
    color: var(--muted);
  }
  .forget:hover {
    background: var(--surface-2);
  }
  .tips {
    margin: 18px 0 0;
    font-size: 12.5px;
    color: var(--muted);
    text-align: center;
  }
  kbd {
    font: 11.5px var(--mono);
    border: 1px solid var(--border);
    border-radius: 5px;
    padding: 0 5px;
  }
  main {
    padding-top: 14px;
  }
  @media (min-width: 760px) {
    .previewing {
      padding-right: var(--panel-w);
    }
    .previewing .tools {
      display: none;
    }
    .previewing.active .inner {
      padding-right: 20px;
    }
  }
  .toast {
    position: fixed;
    left: 50%;
    bottom: 24px;
    z-index: 90;
    transform: translateX(-50%);
    padding: 10px 16px;
    border-radius: 10px;
    background: var(--text);
    color: var(--bg);
    font-size: 14px;
    box-shadow: var(--shadow-lg);
    animation: up var(--med) var(--ease);
  }
  @keyframes up {
    from {
      transform: translate(-50%, 8px);
      opacity: 0;
    }
  }
  @media (max-width: 759px) {
    .top {
      padding-top: max(14vh, 64px);
    }
    .brand {
      font-size: 26px;
    }
    .active .inner {
      padding: 0 92px 10px 12px;
      gap: 6px;
    }
    .active .brand {
      display: none;
    }
    .tools {
      top: 14px;
      right: 8px;
    }
  }
</style>
