// Light syntax colours for the permission peek: one line at a time, no
// dependencies. Good enough to tell code from strings and comments at a glance;
// it is not a parser and never decides anything.

export type TokenKind = "key" | "str" | "dim" | "cmd" | "";
export interface Token {
  text: string;
  kind: TokenKind;
}

const KEYWORDS: Record<string, string[]> = {
  js: ["const", "let", "var", "function", "return", "if", "else", "for", "while", "of", "in", "new", "import", "export", "from", "default", "async", "await", "class", "extends", "try", "catch", "finally", "throw", "typeof", "interface", "type", "enum", "true", "false", "null", "undefined", "this", "switch", "case", "break", "continue"],
  rs: ["fn", "let", "mut", "pub", "use", "mod", "struct", "enum", "impl", "trait", "match", "if", "else", "for", "while", "loop", "return", "async", "await", "move", "self", "Self", "crate", "super", "const", "static", "where", "as", "in", "true", "false", "Some", "None", "Ok", "Err", "dyn", "ref", "type", "unsafe"],
  py: ["def", "class", "return", "if", "elif", "else", "for", "while", "in", "not", "and", "or", "import", "from", "as", "with", "try", "except", "finally", "raise", "lambda", "yield", "True", "False", "None", "pass", "break", "continue", "global", "async", "await", "is"],
  sh: ["if", "then", "else", "elif", "fi", "for", "do", "done", "while", "until", "case", "esac", "in", "function", "export", "local", "return"],
  css: ["@media", "@import", "@keyframes", "!important", "var"],
  ps1: ["function", "param", "if", "else", "elseif", "foreach", "for", "while", "return", "switch", "try", "catch", "finally", "throw"],
};

const FAMILY: Record<string, keyof typeof KEYWORDS | "md" | "json" | "html"> = {
  js: "js", mjs: "js", cjs: "js", ts: "js", tsx: "js", jsx: "js", svelte: "js", vue: "js",
  rs: "rs", py: "py", sh: "sh", bash: "sh", zsh: "sh", bash_command: "sh", bashrc: "sh", zshrc: "sh", profile: "sh", bash_profile: "sh",
  css: "css", scss: "css", ps1: "ps1", psm1: "ps1", powershell_command: "ps1",
  json: "json", toml: "json", yaml: "json", yml: "json", md: "md", html: "html", xml: "html",
};

const LINE_COMMENT: Record<string, string[]> = {
  js: ["//"], rs: ["//"], css: [], py: ["#"], sh: ["#"], ps1: ["#"], json: ["#"], md: [], html: [],
};

/** Colour one line of code. `lang` is a file extension, or "bash_command" / "powershell_command". */
export function highlight(line: string, lang: string): Token[] {
  const family = FAMILY[lang.toLowerCase()] ?? "";
  if (!family || family === "md") return [{ text: line, kind: "" }];
  const keywords = new Set(KEYWORDS[family] ?? []);
  const comments = LINE_COMMENT[family] ?? [];
  const shell = family === "sh" || family === "ps1";
  const out: Token[] = [];
  let i = 0;
  let plain = "";
  let atCommand = shell; // the first word of a shell command, and after | && ; it is the program
  const flush = () => {
    if (plain) out.push({ text: plain, kind: "" });
    plain = "";
  };
  const push = (text: string, kind: TokenKind) => {
    flush();
    out.push({ text, kind });
  };
  while (i < line.length) {
    const rest = line.slice(i);
    const c = line[i];
    // Comments run to the end of the line ("#" only at a word start in shells: `$#`, `a#b` are not comments).
    const comment = comments.find((m) => rest.startsWith(m) && (!shell || i === 0 || /\s/.test(line[i - 1])));
    if (comment || (family === "html" && rest.startsWith("<!--")) || (family !== "py" && family !== "sh" && rest.startsWith("/*"))) {
      push(rest, "dim");
      break;
    }
    if (c === '"' || c === "'" || c === "`") {
      let j = i + 1;
      while (j < line.length && line[j] !== c) j += line[j] === "\\" && c !== "'" ? 2 : 1;
      push(line.slice(i, Math.min(j + 1, line.length)), "str");
      i = j + 1;
      atCommand = false;
      continue;
    }
    if (shell && (rest.startsWith("&&") || rest.startsWith("||") || c === "|" || c === ";")) {
      const op = rest.startsWith("&&") || rest.startsWith("||") ? rest.slice(0, 2) : c;
      push(op, "key");
      i += op.length;
      atCommand = true;
      continue;
    }
    if (/[A-Za-z_$@!-]/.test(c)) {
      let j = i + 1;
      while (j < line.length && /[\w$@.:/\\-]/.test(line[j])) j++;
      const word = line.slice(i, j);
      if (shell && atCommand) {
        push(word, keywords.has(word) ? "key" : "cmd");
        atCommand = keywords.has(word);
      } else if (shell && word.startsWith("-")) {
        push(word, "dim");
      } else if (keywords.has(word)) {
        push(word, "key");
      } else {
        plain += word;
      }
      i = j;
      continue;
    }
    if (/\s/.test(c) === false) atCommand = shell && atCommand && /[(]/.test(c);
    plain += c;
    i++;
  }
  flush();
  return out;
}
