// A small line tokenizer for the read-only file viewer (from the Nodal design prototype).
// shortcut: regex-based, one line at a time (multi-line comments/strings aren't tracked);
// switch to a real grammar (e.g. shiki) if the viewer grows into an editor.

export type TokenKind = "kw" | "str" | "com" | "num" | "type" | "fn" | "tag" | "attr" | "key" | "punc" | "txt" | "head";
export interface Token {
  t: string;
  k: TokenKind;
}

const LANGS: Record<string, string> = {
  ts: "TypeScript",
  tsx: "TypeScript",
  js: "JavaScript",
  jsx: "JavaScript",
  mjs: "JavaScript",
  cjs: "JavaScript",
  json: "JSON",
  md: "Markdown",
  rs: "Rust",
  css: "CSS",
  html: "HTML",
  py: "Python",
  go: "Go",
  toml: "TOML",
  yml: "YAML",
  yaml: "YAML",
  sh: "Shell",
  sql: "SQL",
};

export const extOf = (path: string) => {
  const name = path.split("/").pop() ?? "";
  const i = name.lastIndexOf(".");
  return i > 0 ? name.slice(i + 1).toLowerCase() : "";
};
export const languageOf = (path: string) => LANGS[extOf(path)] ?? "Plain text";

const KEYWORDS = new Set(
  (
    "import from export default function const let var return if else for while do switch case break continue " +
    "async await type interface new typeof as class extends implements enum in of try catch finally throw " +
    "fn mut pub impl struct trait match use mod crate self Self where loop def lambda package func"
  ).split(" "),
);
const LITERALS = new Set(["true", "false", "null", "undefined", "None", "Some", "Ok", "Err", "nil"]);
const CODE =
  /(\/\/.*$|#(?!\[).*$|\{\/\*.*?\*\/\}|\/\*.*?\*\/)|('(?:[^'\\]|\\.)*'|"(?:[^"\\]|\\.)*"|`(?:[^`\\]|\\.)*`)|(=>|->)|(\b\d+(?:\.\d+)*\b)|(<\/?[A-Za-z][\w.]*|\/?>)|([A-Za-z_$][\w$]*)|(\s+)|([\s\S])/g;
/** `#` starts a comment only in these. */
const HASH_COMMENTS = new Set(["py", "sh", "toml", "yml", "yaml"]);

export function tokenize(line: string, ext: string): Token[] {
  const out: Token[] = [];
  const push = (t: string, k: TokenKind) => {
    if (!t) return;
    const last = out[out.length - 1];
    if (last && last.k === k) last.t += t;
    else out.push({ t, k });
  };
  if (!(ext in LANGS)) {
    push(line, "txt");
    return out;
  }
  if (ext === "md") {
    if (/^#/.test(line)) push(line, "head");
    else if (/^( {4}|\t|```)/.test(line)) push(line, "str");
    else if (/^\s*[-*] /.test(line)) {
      const m = /^(\s*[-*] )/.exec(line)![1];
      push(m, "kw");
      push(line.slice(m.length), "txt");
    } else push(line, "txt");
    return out;
  }
  const re = new RegExp(CODE.source, "g");
  let m: RegExpExecArray | null;
  while ((m = re.exec(line))) {
    const rest = line.slice(re.lastIndex);
    if (m[1] && (m[1][0] !== "#" || HASH_COMMENTS.has(ext))) push(m[1], "com");
    else if (m[1]) push(m[1], "punc");
    else if (m[2]) push(m[2], ext === "json" && /^\s*:/.test(rest) ? "key" : "str");
    else if (m[3]) push(m[3], "kw");
    else if (m[4]) push(m[4], "num");
    else if (m[5]) push(m[5], "tag");
    else if (m[6]) {
      const w = m[6];
      push(
        w,
        KEYWORDS.has(w)
          ? "kw"
          : LITERALS.has(w)
            ? "num"
            : /^\(/.test(rest)
              ? "fn"
              : /^=[^=>]/.test(rest)
                ? "attr"
                : /^[A-Z]/.test(w)
                  ? "type"
                  : "txt",
      );
    } else if (m[7]) push(m[7], out.length ? out[out.length - 1]!.k : "txt");
    else push(m[8]!, "punc");
  }
  return out;
}
