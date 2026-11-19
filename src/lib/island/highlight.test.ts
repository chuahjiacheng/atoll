// node --test src/lib/island/highlight.test.ts
import { test } from "node:test";
import assert from "node:assert/strict";
import { highlight } from "./highlight.ts";

const kinds = (line: string, lang: string) => highlight(line, lang).filter((t) => t.kind).map((t) => `${t.kind}:${t.text}`);

test("shell commands: programs, operators, flags and strings", () => {
  assert.deepEqual(kinds('rm -rf "src" && npm test | tail -5', "bash_command"), ["cmd:rm", "dim:-rf", 'str:"src"', "key:&&", "cmd:npm", "key:|", "cmd:tail", "dim:-5"]);
  // A "#" inside a word is not a comment.
  assert.deepEqual(kinds("echo a#b # note", "bash_command"), ["cmd:echo", "dim:# note"]);
});

test("code: keywords, strings, comments", () => {
  assert.deepEqual(kinds("const s = 'x'; // done", "ts"), ["key:const", "str:'x'", "dim:// done"]);
  assert.deepEqual(kinds("def f(p): return 'a'", "py"), ["key:def", "key:return", "str:'a'"]);
  assert.deepEqual(kinds("let x = 1;", "rs"), ["key:let"]);
});

test("unknown languages and Markdown stay plain, and text is never lost", () => {
  assert.deepEqual(highlight("# Title", "md"), [{ text: "# Title", kind: "" }]);
  const line = "if (a) { return `${b}`; } // x \"y\"";
  assert.equal(highlight(line, "js").map((t) => t.text).join(""), line);
  assert.equal(highlight("unterminated 'string", "py").map((t) => t.text).join(""), "unterminated 'string");
});
