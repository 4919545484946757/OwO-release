// OwO Agent 轻量 Markdown / LaTeX 渲染器（自 app.js 机械提取，单一职责：文本 → 安全 HTML）。
//
// 无外部依赖、不访问会话/网络状态；app.js 经 window.OwoMarkdown 解构复用原函数名。
// 安全边界：所有插值先 HTML 转义；链接仅放行 http/https/mailto 与相对路径。
(function (root) {
  "use strict";

// ---------- 轻量 Markdown 渲染（对标 Codex 桌面：代码块/标题/列表/表格/行内样式） ----------

function escapeHtml(text) {
  return text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

function escapeAttribute(text) {
  return escapeHtml(text).replace(/"/g, "&quot;").replace(/'/g, "&#39;");
}

function safeMarkdownHref(raw) {
  const href = raw.replace(/&amp;/g, "&").trim();
  if (!href || /^(?:javascript|data|vbscript):/i.test(href)) return "";
  try {
    const url = new URL(href, window.location.href);
    if (["http:", "https:", "mailto:"].includes(url.protocol)) return url.href;
  } catch (_) {
    // 无法解析的链接按普通文本显示。
  }
  return /^(?:\.|\/|#)/.test(href) ? href : "";
}

// ---------- 轻量 LaTeX 可读化（推理模型的输出习惯） ----------
//
// 接入深度思考（deepseek-reasoner / glm-z1 系）后暴露的新问题：这类模型写数学
// 结论时习惯用 LaTeX（`\frac{1}{6}`、`\times`、`\approx`），轻量 Markdown 渲染器
// 不认 LaTeX，原样吐给用户——一屏 `$\frac{24}{7}$` 看起来就像乱码。
//
// 这里不做完整 KaTeX（体积大、与 md 代码块冲突多），只做**可读化**：
// 分隔符内的公式转成 `a/(b)` 形式并用等宽样式标出，分隔符外的裸符号做等价替换。
// 代码块不经过这里（flushCode 直接转义），所以不会误伤代码。

// 占位符用 NUL (NUL 不会出现在正常文本里，行内规则也不会跨它匹配)
const TEX_MARK = "\u0000";

const TEX_SYMBOLS = [
  ["\\times", "×"],
  ["\\cdot", "·"],
  ["\\div", "÷"],
  ["\\approx", "≈"],
  ["\\neq", "≠"],
  ["\\leq", "≤"],
  ["\\geq", "≥"],
  ["\\le", "≤"],
  ["\\ge", "≥"],
  ["\\pm", "±"],
  ["\\mp", "∓"],
  ["\\rightarrow", "→"],
  ["\\Rightarrow", "⇒"],
  ["\\to", "→"],
  ["\\ldots", "…"],
  ["\\cdots", "…"],
  ["\\infty", "∞"],
  ["\\pi", "π"],
  ["\\alpha", "α"],
  ["\\beta", "β"],
  ["\\gamma", "γ"],
  ["\\theta", "θ"],
  ["\\lambda", "λ"],
  ["\\mu", "μ"],
  ["\\sigma", "σ"],
  ["\\Delta", "Δ"],
  ["\\%", "%"],
];

/// LaTeX 片段 → 可读文本（分数/根号做结构化简写，其余符号等价替换）。
function texToReadable(tex) {
  let out = String(tex || "");
  out = out.replace(/\\[dt]?frac\s*\{([^{}]*)\}\s*\{([^{}]*)\}/g, "($1)/($2)");
  out = out.replace(/\\[dt]?frac\s*(\d)\s*(\d)/g, "($1)/($2)");
  out = out.replace(/\\sqrt\s*\{([^{}]*)\}/g, "√($1)");
  out = out.replace(/\\sqrt\s*(\d)/g, "√$1");
  for (const [command, symbol] of TEX_SYMBOLS) {
    out = out.split(command).join(symbol);
  }
  out = out.replace(/\\(?:left|right|displaystyle|text|mathrm|mathbf|operatorname|mbox)\b/g, "");
  out = out.replace(/\\\\/g, " ");
  out = out.replace(/[{}]/g, "");
  return out.replace(/\s+/g, " ").trim();
}

/// 公式表按"每次渲染"累积：占位符在整篇 html 拼好后统一还原，
/// 这样行内渲染（标题/列表/表格/段落）无需各自关心公式。
let TEX_STORE = [];

/// 把 `\(…\)` / `$$…$$` / `$…$` 的公式抠成占位符（只对非代码行调用）。
function extractTex(text) {
  const hold = (match, body) => {
    TEX_STORE.push(texToReadable(body));
    return `${TEX_MARK}${TEX_STORE.length - 1}${TEX_MARK}`;
  };
  let out = String(text || "");
  out = out.replace(/\\\[([\s\S]+?)\\\]/g, hold);
  out = out.replace(/\$\$([\s\S]+?)\$\$/g, hold);
  out = out.replace(/\$([^$\n]{1,400}?)\$/g, hold);
  out = out.replace(/\\\(([\s\S]+?)\\\)/g, hold);
  return out;
}

/// 占位符还原：公式渲染成等宽的 `<code class="md-tex">`。
function restoreTex(html, store) {
  if (!store.length) return html;
  return html.replace(
    new RegExp(`${TEX_MARK}(\\d+)${TEX_MARK}`, "g"),
    (match, index) => {
      const body = store[Number(index)];
      if (!body) return match;
      return `<code class="md-tex">${escapeHtml(body)}</code>`;
    }
  );
}

// ---------- 代码高亮：零依赖词法着色（对标 Codex / DSH 的代码块观感） ----------
//
// 取舍：桌面端是零外部网络依赖红线，不引入 highlight.js/KaTeX 这类运行时库；
// 用一个「注释 → 字符串 → 数字 → 标识符」的通用扫描器 + 按语言的配置表，
// 覆盖 agent 输出里最常见的语言（rust/js/ts/python/json/bash/yaml/toml/sql/
// html/css/c 系）。未知语言原样转义输出，不做假高亮；所有 token 文本先转义再拼接。

function hlEscapeRe(text) {
  return String(text).replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

// C 家族共用基础（注释/引号一致，关键字按语言拼）。
function hlCFamily(keywords, extra) {
  return {
    keywords: keywords + (extra || ""),
    literals: "true false null nullptr NULL",
    line: "//",
    block: ["/*", "*/"],
    quotes: ['"'],
    directive: true,
  };
}

const HL_MODES = {
  rust: {
    keywords:
      "as async await break const continue crate dyn else enum extern false fn for if impl in let loop match mod move mut pub ref return self static struct super trait true type union unsafe use where while",
    literals: "Self None Some Ok Err",
    line: "//",
    block: ["/*", "*/"],
    quotes: ['"', "'"],
    attrs: "#[",
    macroBang: true,
  },
  javascript: {
    keywords:
      "as async await break case catch class const continue debugger default delete do else export extends finally for from function get if import in instanceof let new of return set static super switch this throw try typeof var void while with yield",
    literals: "true false null undefined NaN Infinity globalThis",
    line: "//",
    block: ["/*", "*/"],
    quotes: ['"', "'"],
    template: true,
  },
  typescript: {
    keywords:
      "as async await break case catch class const continue declare default delete do else export extends finally for from function get if implements import in infer instanceof interface is keyof let namespace new of private protected public readonly return satisfies set static super switch this throw try type typeof var void while with yield",
    literals: "true false null undefined NaN any never unknown bigint symbol",
    line: "//",
    block: ["/*", "*/"],
    quotes: ['"', "'"],
    template: true,
  },
  python: {
    keywords:
      "and as assert async await break case class continue def del elif else except finally for from global if import in is lambda match nonlocal not or pass raise return try while with yield",
    literals: "True False None self cls",
    line: "#",
    quotes: ['"', "'"],
    triple: true,
    decorator: true,
  },
  json: { json: true, literals: "true false null", quotes: ['"'] },
  bash: {
    keywords:
      "if then else elif fi for while until do done case esac function in return select coproc time local export",
    literals: "true false",
    line: "#",
    quotes: ['"', "'"],
    vars: true,
  },
  yaml: { keyed: "yaml", line: "#", quotes: ['"', "'"] },
  toml: { keyed: "toml", line: "#", quotes: ['"""', '"', "'"] },
  sql: {
    keywords:
      "add all alter and any as asc begin between by case check column create cross default delete desc distinct drop else end exists foreign from full group having in index inner insert into is join key left like limit not null offset on or order outer primary references right rollback select set table then transaction union unique update values view when where with",
    literals: "true false null",
    line: "--",
    block: ["/*", "*/"],
    quotes: ["'", '"'],
  },
  html: { tag: true, block: ["<!--", "-->"], quotes: ['"', "'"] },
  css: {
    keywords: "important media supports keyframes import charset",
    line: null,
    block: ["/*", "*/"],
    quotes: ['"', "'"],
    hexColor: true,
    cssProp: true,
    atRule: true,
  },
  c: hlCFamily(
    "auto break case char const continue default do double else enum extern float for goto if inline int long register return short signed sizeof static struct switch typedef union unsigned void volatile while",
  ),
  cpp: hlCFamily(
    "alignas alignof auto break case catch class const constexpr continue decltype default delete do dynamic_cast else enum explicit export extern final for friend goto if mutable namespace new noexcept operator override private protected public register return sizeof static static_cast struct switch template this throw try typedef typeid typename union using virtual void volatile while",
  ),
  java: hlCFamily(
    "abstract assert break byte case catch char class const continue default do double else enum extends final finally float for goto if implements import instanceof int interface long native new package private protected public return short static strictfp super switch synchronized this throw throws transient try void volatile while var record",
  ),
  go: hlCFamily(
    "break case chan const continue default defer else fallthrough for func go goto if import interface map package range return select struct switch type var",
    " nil iota true false",
  ),
  csharp: hlCFamily(
    "abstract as base break case catch checked class const continue decimal default delegate do double else enum event explicit extern false finally fixed float for foreach goto if implicit in int interface internal is lock long namespace new object operator out override params private protected public readonly ref return sbyte sealed short sizeof stackalloc static string struct switch this throw true try typeof uint ulong unchecked unsafe ushort using var virtual void volatile while",
  ),
};

const HL_ALIAS = {
  rs: "rust",
  js: "javascript",
  jsx: "javascript",
  mjs: "javascript",
  cjs: "javascript",
  node: "javascript",
  ts: "typescript",
  tsx: "typescript",
  py: "python",
  python3: "python",
  sh: "bash",
  shell: "bash",
  zsh: "bash",
  console: "bash",
  ps1: "bash",
  powershell: "bash",
  yml: "yaml",
  toml: "toml",
  ini: "toml",
  htm: "html",
  xml: "html",
  vue: "html",
  svg: "html",
  mysql: "sql",
  postgres: "sql",
  sqlite: "sql",
  h: "c",
  cxx: "cpp",
  cc: "cpp",
  hpp: "cpp",
  kt: "java",
  golang: "go",
  cs: "csharp",
};

function hlModeFor(lang) {
  const name = String(lang || "").trim().toLowerCase();
  if (!name) return null;
  return HL_MODES[HL_ALIAS[name] || name] || null;
}

function hlSpan(cls, text) {
  return `<span class="tk-${cls}">${escapeHtml(text)}</span>`;
}

/// 主扫描器：从位置 i 起尝试一条规则，命中返回 { text, next }，未命中返回 null。
function hlTry(rest, mode) {
  // 块注释（html 的 <!-- -->、c 的 /**/、css 的 /* */）。
  if (mode.block && rest.startsWith(mode.block[0])) {
    const end = rest.indexOf(mode.block[1], mode.block[0].length);
    const stop = end === -1 ? rest.length : end + mode.block[1].length;
    return { cls: "c", text: rest.slice(0, stop), next: stop };
  }
  // 行注释（sql 用 --，bash/yaml 用 #，其余 //）。
  if (mode.line && rest.startsWith(mode.line)) {
    const end = rest.indexOf("\n");
    return { cls: "c", text: rest.slice(0, end === -1 ? rest.length : end), next: end === -1 ? rest.length : end };
  }
  // Python 三引号字符串（先于单引号规则，否则会被截断成 ""）。
  if (mode.triple) {
    for (const mark of ['"""', "'''"]) {
      if (rest.startsWith(mark)) {
        const end = rest.indexOf(mark, mark.length);
        const stop = end === -1 ? rest.length : end + mark.length;
        return { cls: "s", text: rest.slice(0, stop), next: stop };
      }
    }
  }
  // rust 属性（#[derive(Debug)] 等，单层即可覆盖绝大多数写法）。
  if (mode.attrs && rest.startsWith(mode.attrs)) {
    const end = rest.indexOf("]", mode.attrs.length);
    const stop = end === -1 ? rest.length : end + 1;
    return { cls: "m", text: rest.slice(0, stop), next: stop };
  }
  // C 家系预处理指令（#include/#define）与 bash 的 # 注释已由 line 覆盖。
  if (mode.directive && rest.startsWith("#")) {
    const match = /^#\s*\w+/.exec(rest);
    if (match) return { cls: "m", text: match[0], next: match[0].length };
  }
  // Python 装饰器（@app.route(...)）。
  if (mode.decorator && rest.startsWith("@")) {
    const match = /^@[\w.]+/.exec(rest);
    if (match) return { cls: "m", text: match[0], next: match[0].length };
  }
  // 引号字符串（含转义；未闭合时吞到行尾，避免整篇被一个坏引号染色）。
  if (mode.quotes) {
    for (const quote of mode.quotes) {
      if (quote.length === 3 && !mode.triple) {
        // toml 的 """…"""：先于单引号匹配。
        if (rest.startsWith(quote)) {
          const end = rest.indexOf(quote, quote.length);
          const stop = end === -1 ? rest.length : end + quote.length;
          return { cls: "s", text: rest.slice(0, stop), next: stop };
        }
        continue;
      }
      if (rest[0] === quote) {
        const match = new RegExp(
          hlEscapeRe(quote) + "(?:\\\\[\\s\\S]|[^\\\\\\n" + hlEscapeRe(quote) + "])*" + hlEscapeRe(quote) + "?",
        ).exec(rest);
        if (match) return { cls: "s", text: match[0], next: match[0].length };
      }
    }
  }
  // JS/TS 模板字符串。
  if (mode.template && rest[0] === "`") {
    const match = /`(?:\\[\s\S]|[^`\\])*`?/.exec(rest);
    if (match) return { cls: "s", text: match[0], next: match[0].length };
  }
  // bash 变量引用（$VAR / ${VAR} / $?）。
  if (mode.vars && rest[0] === "$") {
    const match = /\$\{[^}\n]*\}?|\$[\w@#?]+/.exec(rest);
    if (match) return { cls: "b", text: match[0], next: match[0].length };
  }
  // 数字（含 0x/单位后缀/科学计数）。
  const number = /(?:0[xXoObB][0-9a-fA-F_]+|\d[\d_]*\.?[\d_]*(?:[eE][+-]?\d+)?)(?:[uif](?:8|16|32|64|size)?|n|L|f|d)?/.exec(rest);
  if (number && /^[0-9]/.test(rest)) {
    return { cls: "n", text: number[0], next: number[0].length };
  }
  // css 颜色字面量（#fff / #1a2b3c）。
  if (mode.hexColor && rest[0] === "#") {
    const match = /^#[0-9a-fA-F]{3,8}\b/.exec(rest);
    if (match) return { cls: "n", text: match[0], next: match[0].length };
  }
  // 标识符分类：关键字 / 字面量 / 宏 / 类型 / 函数调用 / 属性键。
  const ident = /^[A-Za-z_$@#][\w$-]*/.exec(rest);
  if (ident) {
    const word = ident[0];
    const after = rest.slice(word.length);
    if (mode.macroBang && after[0] === "!") {
      return { cls: "m", text: word + "!", next: word.length + 1 };
    }
    if (mode.keywords && mode.keywords.split(/\s+/).includes(word)) {
      return { cls: "k", text: word, next: word.length };
    }
    if (mode.literals && mode.literals.split(/\s+/).includes(word)) {
      return { cls: "b", text: word, next: word.length };
    }
    if (mode.cssProp && /^\s*:/.test(after)) {
      return { cls: "p", text: word, next: word.length };
    }
    if (mode.tag && /^=/.test(after)) {
      return { cls: "p", text: word, next: word.length };
    }
    if (mode.atRule && word[0] === "@") {
      return { cls: "m", text: word, next: word.length };
    }
    if (/^\s*\(/.test(after)) return { cls: "f", text: word, next: word.length };
    if (/^[A-Z]/.test(word)) return { cls: "t", text: word, next: word.length };
    return { cls: "", text: word, next: word.length };
  }
  return null;
}

/// 通用高亮入口：返回带 token span 的 HTML；未知语言返回纯转义文本。
function highlightCode(code, lang) {
  const raw = String(code == null ? "" : code);
  const mode = hlModeFor(lang);
  if (!mode) return escapeHtml(raw);
  let out = "";
  let i = 0;
  while (i < raw.length) {
    const rest = raw.slice(i);
    const ws = /^\s+/.exec(rest);
    if (ws) {
      out += ws[0];
      i += ws[0].length;
      continue;
    }
    // JSON：字符串后跟冒号 → 属性键，否则普通字符串。
    if (mode.json && rest[0] === '"') {
      const match = /"(?:\\[\s\S]|[^"\\\n])*"?/.exec(rest);
      if (match) {
        const after = rest.slice(match[0].length);
        const isKey = /^\s*:/.test(after);
        out += hlSpan(isKey ? "p" : "s", match[0]);
        i += match[0].length;
        continue;
      }
    }
    // yaml/toml：行首键（key: / key =）与 [section]。
    if (mode.keyed && /^\S/.test(rest)) {
      const before = raw.lastIndexOf("\n", i - 1);
      const atLineStart = raw.slice(before + 1, i).trim() === "";
      if (atLineStart) {
        if (mode.keyed === "yaml") {
          const key = /^[^\s:#"][^:\n]{0,120}(?=\s*:)/.exec(rest);
          if (key) {
            out += hlSpan("p", key[0]);
            i += key[0].length;
            continue;
          }
        } else {
          const section = /^\[[^\]\n]*\]/.exec(rest);
          if (section) {
            out += hlSpan("t", section[0]);
            i += section[0].length;
            continue;
          }
          const key = /^[\w.-]+(?=\s*=)/.exec(rest);
          if (key) {
            out += hlSpan("p", key[0]);
            i += key[0].length;
            continue;
          }
        }
      }
    }
    // html 标签名（</div、<span）。
    if (mode.tag && rest[0] === "<") {
      const tag = /^<\/?[\w:-]+/.exec(rest);
      if (tag) {
        out += hlSpan("t", tag[0]);
        i += tag[0].length;
        continue;
      }
    }
    const hit = hlTry(rest, mode);
    if (hit) {
      out += hit.cls ? hlSpan(hit.cls, hit.text) : escapeHtml(hit.text);
      i += hit.next;
      continue;
    }
    // 单字符兜底（运算符/标点原样转义）。
    out += escapeHtml(raw[i]);
    i += 1;
  }
  return out;
}

function inlineMarkdown(text) {
  let out = escapeHtml(text);
  out = out.replace(/`([^`]+)`/g, "<code>$1</code>");
  out = out.replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>");
  out = out.replace(/\*([^*]+)\*/g, "<em>$1</em>");
  out = out.replace(/\[([^\]]+)\]\(([^)\s]+)\)/g, (match, label, rawHref) => {
    const href = safeMarkdownHref(rawHref);
    return href
      ? `<a href="${escapeAttribute(href)}" target="_blank" rel="noopener">${label}</a>`
      : label;
  });
  // 分隔符外的裸 LaTeX（模型常常不加 $…$）：符号表 + 花括号边界明确的 frac/sqrt。
  // 取舍说明：行内代码里的 \frac 也会被化简（概率极低——没人会在代码片段里写
  // 公式），换取正文中裸公式可读，这个交换是划算的。
  out = out.replace(/\\[dt]?frac\s*\{([^{}]*)\}\s*\{([^{}]*)\}/g, "($1)/($2)");
  out = out.replace(/\\[dt]?frac\s*(\d)\s*(\d)/g, "($1)/($2)");
  out = out.replace(/\\sqrt\s*\{([^{}]*)\}/g, "√($1)");
  for (const [command, symbol] of TEX_SYMBOLS) {
    out = out.split(command).join(symbol);
  }
  return out;
}

// Markdown 表格按分隔行识别，保留空单元格和转义/代码中的竖线。
function splitMarkdownTableRow(line) {
  const source = String(line || "").trim();
  if (!source.includes("|")) return null;
  const cells = [];
  let cell = "";
  let codeTicks = 0;
  for (let index = 0; index < source.length; index++) {
    const char = source[index];
    if (char === "\\" && source[index + 1] === "|") {
      cell += "|";
      index += 1;
      continue;
    }
    if (char === "`") {
      let end = index + 1;
      while (source[end] === "`") end += 1;
      const run = end - index;
      if (!codeTicks) codeTicks = run;
      else if (run === codeTicks) codeTicks = 0;
      cell += source.slice(index, end);
      index = end - 1;
      continue;
    }
    if (char === "|" && !codeTicks) {
      cells.push(cell.trim());
      cell = "";
      continue;
    }
    cell += char;
  }
  cells.push(cell.trim());
  if (source.startsWith("|")) cells.shift();
  if (source.endsWith("|") && !source.endsWith("\\|")) cells.pop();
  return cells;
}

function isMarkdownTableSeparator(cells) {
  return Array.isArray(cells) && cells.length >= 2 && cells.every((cell) => /^:?-{3,}:?$/.test(cell));
}

function markdownTableAlignment(separator) {
  const left = separator.startsWith(":");
  const right = separator.endsWith(":");
  return left && right ? "center" : right ? "right" : "left";
}

// 把 markdown 文本渲染为 HTML。代码块保留原样（pre/code），行内元素转义。
// 公式可读化只对**非代码行**做占位（代码块里的 $…$ 是代码不是公式），
// 整篇 html 拼好后再统一还原占位符。
function renderMarkdown(text) {
  if (!text) return "";
  TEX_STORE = [];
  const lines = text.split("\n");
  const html = [];
  let inCode = false;
  let codeLang = "";
  let codeFenceChar = "";
  let codeFenceLength = 0;
  let codeLines = [];
  let inList = "";
  let inQuote = false;
  let inTable = false;
  let tableHeader = null;
  let tableAlign = null;

  const flushCode = () => {
    if (codeLines.length) {
      const code = codeLines.join("\n");
      // 行号栏只在多行时出现；复制按钮的 data-code 始终携带完整原文。
      const gutter =
        codeLines.length > 1
          ? `<span class="md-lines" aria-hidden="true">${codeLines.map((_, index) => index + 1).join("\n")}</span>`
          : "";
      html.push(
        `<pre class="md-code"><div class="md-code-head"><span>${escapeHtml(codeLang || "code")}</span><button class="md-copy" data-code="${encodeURIComponent(code)}">复制</button></div><code>${gutter}<span class="md-src">${highlightCode(code, codeLang)}</span></code></pre>`
      );
      codeLines = [];
    }
    inCode = false;
    codeLang = "";
    codeFenceChar = "";
    codeFenceLength = 0;
  };
  const flushList = () => {
    if (inList) {
      html.push("</" + inList + ">");
      inList = "";
    }
  };
  const flushQuote = () => {
    if (inQuote) {
      html.push("</blockquote>");
      inQuote = false;
    }
  };
  const flushTable = () => {
    if (inTable) {
      html.push("</table>");
      inTable = false;
    }
    tableHeader = null;
    tableAlign = null;
  };

  for (let lineIndex = 0; lineIndex < lines.length; lineIndex++) {
    const rawLine = lines[lineIndex];
    const fence = rawLine.match(/^\s*(`{3,}|~{3,})(.*)$/);
    if (fence) {
      const marker = fence[1];
      const info = fence[2] || "";
      if (inCode) {
        const closes = marker[0] === codeFenceChar && marker.length >= codeFenceLength && /^\s*$/.test(info);
        if (closes) {
          flushCode();
          continue;
        }
      } else {
        flushList();
        flushQuote();
        flushTable();
        inCode = true;
        codeFenceChar = marker[0];
        codeFenceLength = marker.length;
        codeLang = (info.trim().split(/\s+/, 1)[0] || "").replace(/[^\w.+#-]/g, "");
        continue;
      }
    }
    if (inCode) {
      codeLines.push(rawLine);
      continue;
    }
    const line = extractTex(rawLine);
    if (/^\s*$/.test(line)) {
      flushList();
      flushQuote();
      flushTable();
      html.push("");
      continue;
    }
    const quote = line.match(/^\s*>\s?(.*)$/);
    if (quote) {
      flushList();
      flushTable();
      if (!inQuote) {
        html.push('<blockquote class="md-quote">');
        inQuote = true;
      }
      html.push('<div class="md-p">' + inlineMarkdown(quote[1]) + "</div>");
      continue;
    }
    flushQuote();
    const heading = line.match(/^(#{1,4})\s+(.*)$/);
    if (heading) {
      flushList();
      flushTable();
      const level = heading[1].length;
      html.push(`<h${level} class="md-h${level}">${inlineMarkdown(heading[2])}</h${level}>`);
      continue;
    }
    const hr = line.match(/^\s*(-{3,}|\*{3,})\s*$/);
    if (hr) {
      flushList();
      flushTable();
      html.push('<hr class="md-hr">');
      continue;
    }
    const unorderedItem = line.match(/^\s*[-*+]\s+(.*)$/);
    const orderedItem = line.match(/^\s*\d+\.\s+(.*)$/);
    const listItem = unorderedItem || orderedItem;
    if (listItem) {
      flushTable();
      const listTag = orderedItem ? "ol" : "ul";
      if (inList && inList !== listTag) flushList();
      if (!inList) {
        html.push("<" + listTag + ' class="md-list">');
        inList = listTag;
      }
      html.push("<li>" + inlineMarkdown(listItem[1]) + "</li>");
      continue;
    }
    flushList();
    const rowCells = splitMarkdownTableRow(line);
    if (inTable) {
      if (rowCells && rowCells.length) {
        const paddedCells = rowCells.slice(0, tableHeader.length);
        while (paddedCells.length < tableHeader.length) paddedCells.push("");
        html.push("<tr>");
        paddedCells.forEach((cell, index) => {
          html.push(`<td style="text-align:${tableAlign[index] || "left"}">${inlineMarkdown(cell)}</td>`);
        });
        html.push("</tr>");
        continue;
      }
      flushTable();
    }
    const separatorCells = splitMarkdownTableRow(lines[lineIndex + 1] || "");
    if (rowCells && rowCells.length >= 2 && isMarkdownTableSeparator(separatorCells) && separatorCells.length === rowCells.length) {
      tableHeader = rowCells;
      tableAlign = separatorCells.map(markdownTableAlignment);
      inTable = true;
      html.push('<table class="md-table"><thead><tr>');
      tableHeader.forEach((cell, index) => {
        html.push(`<th style="text-align:${tableAlign[index]}">${inlineMarkdown(cell)}</th>`);
      });
      html.push("</tr></thead><tbody>");
      lineIndex += 1;
      continue;
    }
    html.push(`<div class="md-p">${inlineMarkdown(line)}</div>`);
  }
  flushCode();
  flushList();
  flushQuote();
  flushTable();
  return restoreTex(html.join("\n"), TEX_STORE);
}

  root.OwoMarkdown = Object.freeze({
    escapeHtml,
    escapeAttribute,
    safeMarkdownHref,
    texToReadable,
    extractTex,
    restoreTex,
    inlineMarkdown,
    splitMarkdownTableRow,
    isMarkdownTableSeparator,
    markdownTableAlignment,
    renderMarkdown,
    highlightCode,
  });
})(typeof window !== "undefined" ? window : globalThis);
