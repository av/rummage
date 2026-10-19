<script lang="ts">
  // The useful EXIF fields up front, everything else behind "All details". Key names vary by
  // extractor, so each field tries the common spellings.
  let { exif }: { exif: Record<string, unknown> } = $props();

  const get = (...keys: string[]) => {
    for (const k of keys) {
      const v = exif[k];
      if (v !== undefined && v !== null && v !== '') return v;
    }
    return undefined;
  };
  const n = (v: unknown) => (typeof v === 'number' ? v : typeof v === 'string' ? parseFloat(v) : NaN);

  function shutter(v: unknown) {
    const s = n(v);
    if (typeof v === 'string' && v.includes('/')) return `${v} s`;
    if (!isFinite(s)) return String(v);
    return s >= 1 ? `${s} s` : `1/${Math.round(1 / s)} s`;
  }

  const fields = $derived.by(() => {
    const out: [string, string][] = [];
    const camera = [get('Make', 'make'), get('Model', 'model')].filter(Boolean).join(' ');
    if (camera) out.push(['Camera', camera]);
    const lens = get('LensModel', 'Lens', 'lens');
    if (lens) out.push(['Lens', String(lens)]);
    const parts = [];
    const f = get('FNumber', 'f_number', 'Aperture');
    if (f !== undefined) parts.push(`f/${String(f).replace(/^f\//, '')}`);
    const t = get('ExposureTime', 'exposure_time', 'Shutter');
    if (t !== undefined) parts.push(shutter(t));
    const iso = get('ISO', 'ISOSpeedRatings', 'PhotographicSensitivity', 'iso');
    if (iso !== undefined) parts.push(`ISO ${iso}`);
    const fl = get('FocalLength', 'focal_length');
    if (fl !== undefined) parts.push(`${String(fl).replace(/\s*mm$/, '')} mm`);
    if (parts.length) out.push(['Exposure', parts.join(' · ')]);
    const w = get('ImageWidth', 'PixelXDimension', 'width');
    const h = get('ImageLength', 'ImageHeight', 'PixelYDimension', 'height');
    if (w && h) out.push(['Dimensions', `${w} × ${h}`]);
    const taken = get('DateTimeOriginal', 'DateTime', 'taken');
    if (taken) out.push(['Taken', String(taken)]);
    const lat = n(get('GPSLatitude', 'latitude', 'lat'));
    const lon = n(get('GPSLongitude', 'longitude', 'lon'));
    if (isFinite(lat) && isFinite(lon)) out.push(['Place', `${lat.toFixed(5)}, ${lon.toFixed(5)}`]);
    return out;
  });

  const all = $derived(Object.entries(exif).filter(([, v]) => v !== null && typeof v !== 'object'));
</script>

{#if fields.length}
  <dl class="exif">
    {#each fields as [k, v] (k)}
      <div><dt>{k}</dt><dd>{v}</dd></div>
    {/each}
  </dl>
{/if}
{#if all.length > fields.length}
  <details>
    <summary>All details</summary>
    <table>
      <tbody>
        {#each all as [k, v] (k)}
          <tr><th scope="row">{k}</th><td>{String(v)}</td></tr>
        {/each}
      </tbody>
    </table>
  </details>
{/if}

<style>
  .exif {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
    gap: 10px 16px;
    margin: 0;
    font-size: 13.5px;
  }
  dt {
    color: var(--muted);
    font-size: 12px;
  }
  dd {
    margin: 0;
    overflow-wrap: anywhere;
  }
  details {
    font-size: 13px;
  }
  summary {
    cursor: pointer;
    color: var(--accent);
    width: max-content;
  }
  table {
    width: 100%;
    border-collapse: collapse;
    margin-top: 8px;
  }
  th,
  td {
    text-align: left;
    padding: 4px 8px 4px 0;
    border-bottom: 1px solid var(--border);
    vertical-align: top;
    overflow-wrap: anywhere;
  }
  th {
    font-weight: 500;
    color: var(--muted);
    width: 40%;
  }
</style>
