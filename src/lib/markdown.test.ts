// node --experimental-strip-types --test src/lib/markdown.test.ts
import { test } from "node:test";
import assert from "node:assert/strict";
import { inline, renderMarkdown } from "./markdown.ts";

test("escapes HTML everywhere", () => {
  const html = renderMarkdown('<img src=x onerror="alert(1)"> **<b>x</b>**\n\n```\n<script>bad()</script>\n```\n`<i>`');
  assert.ok(!html.includes("<img"));
  assert.ok(!html.includes("<script>"));
  assert.ok(!html.includes("<b>"));
  assert.ok(!html.includes("<i>"));
  assert.ok(html.includes("&lt;script&gt;bad()&lt;/script&gt;"));
});

test("only web links become links", () => {
  assert.equal(inline("[ok](https://a.com/x?y=1&z=2)"), '<a href="https://a.com/x?y=1&amp;z=2" data-href="https://a.com/x?y=1&amp;z=2">ok</a>');
  const js = inline("[bad](javascript:alert(1))");
  assert.ok(!js.includes("<a"));
  const quote = inline('[q](https://a.com/"onmouseover="x)');
  assert.ok(!/href="[^"]*"onmouseover/.test(quote));
  assert.equal(inline("see https://example.com."), 'see <a href="https://example.com" data-href="https://example.com">https://example.com</a>.');
});

test("inline formatting", () => {
  assert.equal(inline("**bold** and *it* and _it2_ and ~~gone~~"), "<strong>bold</strong> and <em>it</em> and <em>it2</em> and <del>gone</del>");
  assert.equal(inline("`**not bold**`"), "<code>**not bold**</code>");
  assert.equal(inline("snake_case_name and 2*3*4"), "snake_case_name and 2*3*4");
});

test("code fences, including one still streaming", () => {
  const done = renderMarkdown("Hi\n```ts\nconst a = 1;\n\nlet b;\n```\nAfter");
  assert.match(done, /<p>Hi<\/p><div class="md-code">.*<span>ts<\/span>.*<code>const a = 1;\n\nlet b;<\/code>.*<p>After<\/p>/s);
  const open = renderMarkdown("```py\nprint(1)");
  assert.match(open, /<code>print\(1\)<\/code>/);
});

test("lists, headings, quotes, rules, paragraphs", () => {
  assert.equal(renderMarkdown("- a\n- **b**\n  - nested"), "<ul><li>a</li><li><strong>b</strong><br>• nested</li></ul>");
  assert.equal(renderMarkdown("3. x\n4. y"), '<ol start="3"><li>x</li><li>y</li></ol>');
  assert.equal(renderMarkdown("## Title"), '<p class="md-h md-h2">Title</p>');
  assert.equal(renderMarkdown("> quoted"), "<blockquote><p>quoted</p></blockquote>");
  assert.equal(renderMarkdown("a\nb\n\n---\n\nc"), "<p>a<br>b</p><hr><p>c</p>");
});

test("backslashes in code survive", () => {
  assert.equal(inline("`\\d{4}`"), "<code>\\d{4}</code>");
  assert.match(renderMarkdown("```\n/^\\d+$/\n```"), /<code>\/\^\\d\+\$\/<\/code>/);
});

test("tables", () => {
  const html = renderMarkdown("| A | B |\n|---|:-:|\n| 1 | **2** |\n| 3 |");
  assert.equal(
    html,
    '<div class="md-table"><table><thead><tr><th>A</th><th>B</th></tr></thead><tbody><tr><td>1</td><td><strong>2</strong></td></tr><tr><td>3</td><td></td></tr></tbody></table></div>',
  );
});
