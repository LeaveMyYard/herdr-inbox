// Capture the empty-inbox design 2C (`inboxC`, the tumbleweed in the desert) from the design's
// own cell-grid renderer, herdr-panorama's docs/design/herdr-tui.js, into
// tests/fixtures/design_empty_inbox.json (read by crates/inbox-core/tests/design_empty_inbox.rs).
//
// Usage: node tools/capture_empty_inbox.mjs <path to herdr-panorama/docs/design/herdr-tui.js> \
//          > tests/fixtures/design_empty_inbox.json
//
// The design is a function of its frame index `f`; its loop is `f % (span + 8)` = 42 frames.
// The capture holds frames 0..=PERIOD (the last one shows the loop wrapping to frame 0) and the
// design's palette `C`. Each frame is 20 rows of [text, fg, bg, bold] runs: the cells of one run
// share fg, bg and bold, exactly as the design's `Scr` holds them (a blank keeps the fg it was
// written with). Nothing but the given file is read.
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';

const path = process.argv[2];
if (!path) {
  console.error('usage: node tools/capture_empty_inbox.mjs <herdr-tui.js>');
  process.exit(2);
}
const src = readFileSync(path, 'utf8');
const mod = await import('data:text/javascript;base64,' + Buffer.from(src).toString('base64'));
const { inboxC, C } = mod;

const PERIOD = 42;
// `Scr.rows()` splits a row into runs of one style; re-expand each run to its cells and merge
// them again by [fg, bg, bold] so the fixture does not depend on the HTML styling.
const runs = rows => rows.map(row => {
  const out = [];
  for (const u of row.r) {
    const chars = Array.from(u.t);
    const n = parseInt(u.st.width);
    if (chars.length !== n) throw new Error(`run width ${n} != ${chars.length} chars`);
    const fg = u.st.color.toLowerCase();
    const bg = u.st.background === 'transparent' ? null : u.st.background.toLowerCase();
    const bold = u.st.fontWeight === 700;
    const last = out[out.length - 1];
    if (last && last[1] === fg && last[2] === bg && last[3] === bold) last[0] += u.t;
    else out.push([u.t, fg, bg, bold]);
  }
  return out;
});

const frames = [];
for (let f = 0; f <= PERIOD; f++) {
  const o = inboxC(f);
  frames.push({ f, rows: runs(o.rows) });
}
// The loop is the design's own: frame f + PERIOD draws what frame f draws.
for (let f = 0; f < PERIOD; f++) {
  if (JSON.stringify(runs(inboxC(f + PERIOD).rows)) !== JSON.stringify(frames[f].rows)) throw new Error(`frame ${f} does not repeat`);
}
const sha = createHash('sha256').update(src).digest('hex');
const head = {
  source: `inboxC(f) of herdr-panorama docs/design/herdr-tui.js (sha256 ${sha}); tools/capture_empty_inbox.mjs`,
  size: [frames[0].rows[0].reduce((n, r) => n + Array.from(r[0]).length, 0), frames[0].rows.length],
  period: PERIOD,
  C,
};
// One frame per line, so a regenerated fixture diffs frame by frame.
const lines = frames.map(fr => '  ' + JSON.stringify(fr));
console.log(JSON.stringify(head).slice(0, -1) + ', "frames": [\n' + lines.join(',\n') + '\n]}');
