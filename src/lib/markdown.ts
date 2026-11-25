// Small, safe Markdown → HTML for Ask replies (ask-assistant).
// Everything is HTML-escaped first; only the tags below are ever produced.
// Handles half-written Markdown too, since replies render while they stream
// (an unclosed ``` fence is shown as code up to the end).
//
// Blocks: fenced code, headings, lists (- * + and 1.), quotes, tables, rules,
// paragraphs. Inline: `code`, **bold**, *italic* / _italic_, ~~strike~~,
// [links](https://…) and bare https:// links.

const ESC: Record<string, string> = { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" };
export const escapeHtml = (s: string) => s.replace(/[&<>"']/g, (c) => ESC[c]);

/** Inline Markdown on one already-trimmed line of raw text. */
export function inline(raw: string): string {
  // Pull code spans out first so nothing inside them is formatted.
  const codes: string[] = [];
  let s = raw.replace(/`([^`\n]+)`/g, (_, c: string) => {
    codes.push(c);
    return `\u0000${codes.length - 1}\u0000`;
  });
  const links: string[] = [];
  const keepLink = (text: string, url: string) => {
    links.push(`<a href="${escapeHtml(url)}" data-href="${escapeHtml(url)}">${text}</a>`);
    return `\u0001${links.length - 1}\u0001`;
  };
  s = escapeHtml(s);
  // [text](https://url) — only web links.
  s = s.replace(/\[([^\]\n]+)\]\((https?:\/\/[^\s)]+)\)/g, (_, t: string, u: string) => keepLink(t, unescape(u)));
  // Bare links.
  s = s.replace(/(^|[\s(])(https?:\/\/[^\s<)]+[^\s<).,;:!?'"])/g, (_, pre: string, u: string) => pre + keepLink(u, unescape(u)));
  s = s
    .replace(/\*\*([^*\n]+?)\*\*/g, "<strong>$1</strong>")
    .replace(/__([^_\n]+?)__/g, "<strong>$1</strong>")
    .replace(/~~([^~\n]+?)~~/g, "<del>$1</del>")
    .replace(/(^|[^*\w])\*(?!\s)([^*\n]+?)\*(?!\w)/g, "$1<em>$2</em>")
    .replace(/(^|[^_\w])_(?!\s)([^_\n]+?)_(?!\w)/g, "$1<em>$2</em>");
  s = s.replace(/\u0001(\d+)\u0001/g, (_, i: string) => links[+i]);
  return s.replace(/\u0000(\d+)\u0000/g, (_, i: string) => `<code>${escapeHtml(codes[+i])}</code>`);
}

/** Undo escapeHtml on a URL captured from escaped text. */
function unescape(s: string): string {
  return s.replace(/&amp;/g, "&").replace(/&quot;/g, '"').replace(/&#39;/g, "'").replace(/&lt;/g, "<").replace(/&gt;/g, ">");
}

const FENCE = /^\s{0,3}(```+|~~~+)\s*([\w+#.-]*)\s*$/;
const HEADING = /^\s{0,3}(#{1,6})\s+(.*?)\s*#*\s*$/;
const BULLET = /^(\s*)[-*+]\s+(.*)$/;
const NUMBER = /^(\s*)(\d{1,9})[.)]\s+(.*)$/;
const QUOTE = /^\s{0,3}>\s?(.*)$/;
const RULE = /^\s{0,3}([-*_])(\s*\1){2,}\s*$/;
const TABLE_SEP = /^\s*\|?\s*:?-+:?\s*(\|\s*:?-+:?\s*)*\|?\s*$/;

const cells = (line: string) =>
  line
    .trim()
    .replace(/^\|/, "")
    .replace(/\|$/, "")
    .split("|")
    .map((c) => c.trim());

export function renderMarkdown(src: string): string {
  const lines = src.replace(/\r\n?/g, "\n").split("\n");
  const out: string[] = [];
  let para: string[] = [];
  const flush = () => {
    if (para.length) out.push(`<p>${para.map(inline).join("<br>")}</p>`);
    para = [];
  };

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];

    const fence = line.match(FENCE);
    if (fence) {
      flush();
      const close = fence[1];
      const body: string[] = [];
      i++;
      while (i < lines.length && !(lines[i].trim().startsWith(close[0].repeat(close.length)) && lines[i].trim().replace(/[`~]/g, "") === "")) {
        body.push(lines[i]);
        i++;
      }
      const lang = fence[2] ? escapeHtml(fence[2]) : "";
      out.push(
        `<div class="md-code"><div class="md-code-head"><span>${lang || "code"}</span><button type="button" class="md-copy" data-copy>Copy</button></div><pre><code>${escapeHtml(body.join("\n"))}</code></pre></div>`,
      );
      continue;
    }

    if (!line.trim()) {
      flush();
      continue;
    }

    const heading = line.match(HEADING);
    if (heading) {
      flush();
      const level = Math.min(heading[1].length, 3);
      out.push(`<p class="md-h md-h${level}">${inline(heading[2])}</p>`);
      continue;
    }

    if (RULE.test(line)) {
      flush();
      out.push("<hr>");
      continue;
    }

    // Table: header row, then a |---|---| row.
    if (line.includes("|") && i + 1 < lines.length && TABLE_SEP.test(lines[i + 1]) && lines[i + 1].includes("-")) {
      flush();
      const head = cells(line);
      const rows: string[][] = [];
      i += 2;
      while (i < lines.length && lines[i].includes("|") && lines[i].trim()) {
        rows.push(cells(lines[i]));
        i++;
      }
      i--;
      const th = head.map((c) => `<th>${inline(c)}</th>`).join("");
      const tb = rows.map((r) => `<tr>${head.map((_, k) => `<td>${inline(r[k] ?? "")}</td>`).join("")}</tr>`).join("");
      out.push(`<div class="md-table"><table><thead><tr>${th}</tr></thead><tbody>${tb}</tbody></table></div>`);
      continue;
    }

    if (QUOTE.test(line)) {
      flush();
      const body: string[] = [];
      while (i < lines.length && QUOTE.test(lines[i])) {
        body.push(lines[i].match(QUOTE)![1]);
        i++;
      }
      i--;
      out.push(`<blockquote>${renderMarkdown(body.join("\n"))}</blockquote>`);
      continue;
    }

    if (BULLET.test(line) || NUMBER.test(line)) {
      flush();
      const ordered = !BULLET.test(line);
      const start = ordered ? Number(line.match(NUMBER)![2]) : 1;
      const items: string[] = [];
      while (i < lines.length) {
        const l = lines[i];
        const m = ordered ? l.match(NUMBER) : l.match(BULLET);
        if (m && m[1].length < 2) {
          items.push(ordered ? m[3] : m[2]);
        } else if (l.trim() && /^\s+/.test(l) && items.length) {
          // Indented continuation or nested item: keep it with this item.
          const nested = l.trim().match(/^([-*+]|\d{1,9}[.)])\s+(.*)$/);
          items[items.length - 1] += nested ? `\n• ${nested[2]}` : ` ${l.trim()}`;
        } else {
          break;
        }
        i++;
      }
      i--;
      const lis = items.map((t) => `<li>${t.split("\n").map(inline).join("<br>")}</li>`).join("");
      out.push(ordered ? `<ol${start !== 1 ? ` start="${start}"` : ""}>${lis}</ol>` : `<ul>${lis}</ul>`);
      continue;
    }

    para.push(line.trim());
  }
  flush();
  return out.join("");
}
