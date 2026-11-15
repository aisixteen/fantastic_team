// Renders src/*.dc.html (the design canvas sources) into plain static pages.
// Run: node build.mjs   (no dependencies)
import { readFileSync, writeFileSync, readdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const esc = (v) => String(v).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
const look = (scope, path) => path.trim().split('.').reduce((o, k) => (o == null ? o : o[k]), scope);

function closeOf(s, from, tag) {
  const re = new RegExp(`<(/?)${tag}\\b`, 'g');
  re.lastIndex = from;
  let depth = 0, m;
  while ((m = re.exec(s))) {
    depth += m[1] ? -1 : 1;
    if (depth === 0) return m.index;
  }
  throw new Error('unclosed ' + tag);
}

function render(tpl, scope) {
  let out = '', i = 0;
  const open = /<(sc-for|sc-if)\b([^>]*)>/g;
  for (;;) {
    open.lastIndex = i;
    const m = open.exec(tpl);
    if (!m) break;
    out += fill(tpl.slice(i, m.index), scope);
    const end = closeOf(tpl, m.index, m[1]);
    const inner = tpl.slice(m.index + m[0].length, end);
    const attr = (n) => (new RegExp(`${n}="([^"]*)"`).exec(m[2]) || [])[1];
    if (m[1] === 'sc-for') {
      const list = look(scope, attr('list').replace(/[{}]/g, '')) || [];
      for (const item of list) out += render(inner, { ...scope, [attr('as')]: item });
    } else if (look(scope, attr('value').replace(/[{}]/g, ''))) {
      out += render(inner, scope);
    }
    i = end + `</${m[1]}>`.length;
  }
  return out + fill(tpl.slice(i), scope);
}
const fill = (s, scope) => s.replace(/\{\{([^}]+)\}\}/g, (_, p) => {
  const v = look(scope, p);
  return v == null ? '' : esc(v);
});

const pages = [];
for (const file of readdirSync(join(here, 'src')).filter((f) => f.endsWith('.dc.html')).sort()) {
  const src = readFileSync(join(here, 'src', file), 'utf8');
  const title = /<title>([^<]*)<\/title>/.exec(src)[1];
  const helmet = /<helmet>([\s\S]*?)<\/helmet>/.exec(src)[1];
  const body = /<x-dc>([\s\S]*)<\/x-dc>/.exec(src)[1].replace(/<helmet>[\s\S]*?<\/helmet>/, '');
  const [, propsJson, code] = /data-props='([^']*)'>([\s\S]*?)<\/script>/.exec(src);
  const decl = JSON.parse(propsJson);
  const props = {};
  for (const [k, v] of Object.entries(decl)) if (k[0] !== '$') props[k] = v.default;
  const Component = new Function('DCLogic', code + '; return Component;')(class { constructor(p) { this.props = p; } });
  const html = render(body, new Component(props).renderVals()).replaceAll('/_blob/4391040ca996853236d2105e3388d1f7', 'robo-head.png');
  const w = decl.$preview.width;
  const frame = w >= 1200 ? html
    : `<div style="width:${w}px;max-width:100%;margin:24px auto;border-radius:${w < 600 ? 28 : 16}px;overflow:hidden;box-shadow:0 0 0 1px rgba(0,0,0,.15)">${html}</div>`;
  const name = file.replace('.dc.html', '.html').toLowerCase();
  writeFileSync(join(here, name), `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${title}</title>
${helmet.trim()}
<style>${w >= 1200 ? 'body>[data-theme]{min-height:100vh !important}' : 'html{background:#EDEDED}@media (prefers-color-scheme: dark){html{background:#050505}}'}</style>
</head>
<body>
${frame.trim()}
<script>
// ?theme=dark or ?theme=light switches the page theme.
const t = new URLSearchParams(location.search).get('theme');
if (${'theme' in props} && (t === 'dark' || t === 'light')) document.querySelector('[data-theme]').dataset.theme = t;
</script>
</body>
</html>
`);
  pages.push({ name, title, theme: props.theme });
}
const order = ['main.html', 'file.html', 'board.html', 'phone-tree.html', 'phone-chat.html', 'states.html'];
pages.sort((a, b) => order.indexOf(a.name) - order.indexOf(b.name));
const boards = JSON.parse(readFileSync(join(here, 'src', 'canvas.json'), 'utf8')).boards;
const label = (p) => (Object.entries(boards).find(([f]) => f.toLowerCase().replace('.dc.html', '.html') === p.name) ?? [])[1]?.title ?? p.title;
writeFileSync(join(here, 'index.html'), `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Team app redesign mock</title>
<style>
:root{--bg:#EDEDED;--surface:rgb(255 255 255/.58);--line:rgb(0 0 0/.10);--text:#0B0B0B;--text2:#454545;--accent:#000000;--sunken:rgb(0 0 0/.06);--dot:rgb(0 0 0/.14);--glow:#FFFFFF}
@media (prefers-color-scheme: dark){:root{--bg:#050505;--surface:rgb(20 20 20/.58);--line:rgb(255 255 255/.10);--text:#F2F2F2;--text2:#B8B8B8;--accent:#FFFFFF;--sunken:rgb(255 255 255/.07);--dot:rgb(255 255 255/.14);--glow:rgb(255 255 255/.20);color-scheme:dark}}
*{box-sizing:border-box}
body{margin:0;padding:48px max(16px, env(safe-area-inset-right, 0px)) 48px max(16px, env(safe-area-inset-left, 0px));min-height:100vh;background:radial-gradient(circle at 1px 1px,var(--dot) 1px,transparent 1.6px) 0 0/22px 22px,radial-gradient(70% 55% at 12% -5%,var(--glow) 0,transparent 70%),var(--bg);color:var(--text);font:15px/1.5 system-ui,-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif}
main{max-width:680px;margin:0 auto;display:flex;flex-direction:column;gap:20px}
h1{margin:0;font-size:24px;font-weight:600;text-wrap:balance}
p{margin:0;color:var(--text2)}
ul{list-style:none;margin:0;padding:0;display:flex;flex-direction:column;gap:8px}
li{display:flex;flex-wrap:wrap;align-items:center;gap:8px 12px;padding:14px 16px;border:1px solid var(--line);border-radius:16px;background:var(--surface);box-shadow:0 10px 30px rgb(0 0 0/.10)}
li>a{flex:1;min-width:200px;font-weight:500;color:var(--text);text-decoration:none}
li>a:hover{color:var(--accent)}
.t{display:flex;gap:6px}
.t a{font:12px/24px ui-monospace,SFMono-Regular,Menlo,monospace;padding:0 10px;border-radius:4px;background:var(--sunken);color:var(--text2);text-decoration:none}
.t a:hover{color:var(--accent)}
a:focus-visible{outline:2px solid var(--accent);outline-offset:2px}
code{font:13px ui-monospace,SFMono-Regular,Menlo,monospace}
</style>
</head>
<body>
<main>
<h1>Team app redesign mock</h1>
<ul>
${pages.map((p) => `<li><a href="${p.name}">${esc(label(p))}</a>${p.theme ? `<span class="t"><a href="${p.name}?theme=light">light</a><a href="${p.name}?theme=dark">dark</a></span>` : ''}</li>`).join('\n')}
</ul>
</main>
</body>
</html>
`);
console.log(['index.html', ...pages.map((p) => p.name)].join('\n'));
