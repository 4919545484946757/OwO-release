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

const SUPERSCRIPT = Object.freeze({
  "0": "⁰", "1": "¹", "2": "²", "3": "³", "4": "⁴", "5": "⁵", "6": "⁶", "7": "⁷", "8": "⁸", "9": "⁹",
  "+": "⁺", "-": "⁻", "=": "⁼", "(": "⁽", ")": "⁾", "n": "ⁿ", "i": "ⁱ", "x": "ˣ", "y": "ʸ",
});
const SUBSCRIPT = Object.freeze({
  "0": "₀", "1": "₁", "2": "₂", "3": "₃", "4": "₄", "5": "₅", "6": "₆", "7": "₇", "8": "₈", "9": "₉",
  "+": "₊", "-": "₋", "=": "₌", "(": "₍", ")": "₎", "n": "ₙ", "i": "ᵢ", "x": "ₓ", "y": "ᵧ",
});

/// 只在每个字符都有 Unicode 上下标时转换；宁可不转，也不要出现半个怪字符。
function toScript(text, table) {
  const chars = [...String(text || "")];
  if (!chars.length) return null;
  const mapped = chars.map((char) => table[char]);
  return mapped.some((char) => !char) ? null : mapped.join("");
}

/// LaTeX 片段 → 可读文本（分数/根号/上下标做结构化简写，其余符号等价替换）。
function texToReadable(tex) {
  let out = String(tex || "");
  out = out.replace(/\\[dt]?frac\s*\{([^{}]*)\}\s*\{([^{}]*)\}/g, "($1)/($2)");
  out = out.replace(/\\[dt]?frac\s*(\d)\s*(\d)/g, "($1)/($2)");
  out = out.replace(/\\sqrt\s*\{([^{}]*)\}/g, "√($1)");
  out = out.replace(/\\sqrt\s*(\d)/g, "√$1");
  for (const [command, symbol] of TEX_SYMBOLS) {
    out = out.split(command).join(symbol);
  }
  out = out.replace(/\^\{([^{}]*)\}/g, (match, body) => toScript(body, SUPERSCRIPT) || match);
  out = out.replace(/\^([0-9A-Za-z+\-=()])/g, (match, body) => toScript(body, SUPERSCRIPT) || match);
  out = out.replace(/_\{([^{}]*)\}/g, (match, body) => toScript(body, SUBSCRIPT) || match);
  out = out.replace(/_([0-9A-Za-z+\-=()])/g, (match, body) => toScript(body, SUBSCRIPT) || match);
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

function inlineMarkdown(text) {
  let out = escapeHtml(text);
  out = out.replace(/`([^`]+)`/g, "<code>$1</code>");
  // 图片必须早于链接：否则 `![alt](url)` 的 `[alt](url)` 会先被当成链接。
  // 只允许 http(s)（safeMarkdownHref 已挡掉 javascript:/data:），并懒加载。
  out = out.replace(/!\[([^\]]*)\]\(([^)\s]+)\)/g, (match, alt, rawSrc) => {
    const src = safeMarkdownHref(rawSrc);
    if (!src) return escapeHtml(alt);
    return `<img class="md-img" src="${escapeAttribute(src)}" alt="${escapeAttribute(alt)}" loading="lazy">`;
  });
  out = out.replace(/\*\*\*([^*]+)\*\*\*/g, "<strong><em>$1</em></strong>");
  out = out.replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>");
  out = out.replace(/~~([^~]+)~~/g, "<del>$1</del>");
  out = out.replace(/\*([^*\n]+)\*/g, "<em>$1</em>");
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
  // 裸链接（模型常直接贴 URL）：前缀排除引号/等号/尖括号，不会改写刚生成的 href。
  out = out.replace(
    /(^|[\s(（])(https?:\/\/[^\s<)）"']+)/g,
    (match, prefix, url) =>
      `${prefix}<a href="${escapeAttribute(url)}" target="_blank" rel="noopener">${url}</a>`
  );
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
/// 解析 ```chart / ```pie 围栏的两列数据（`标签, 数值` 或 `标签: 数值`）。
/// 解析失败返回 null，调用方回退成普通代码块，绝不吞数据。
function parseChartRows(lines) {
  const rows = [];
  let title = "";
  for (const raw of lines) {
    const line = String(raw || "").trim();
    if (!line || line.startsWith("#") || line.startsWith("//")) continue;
    const match = line.match(/^(.+?)\s*(?:,|\t|\s:\s|:|\s{2,})\s*(-?\d+(?:\.\d+)?)\s*%?$/);
    if (!match) {
      // 第一行不是数据 → 当作标题；其余无法解析的行直接放弃（宁可回退代码块）。
      if (!rows.length && !title) {
        title = line.slice(0, 80);
        continue;
      }
      return null;
    }
    const label = match[1].trim().replace(/^["']|["']$/g, "").slice(0, 60);
    const value = Number(match[2]);
    if (!label || !Number.isFinite(value)) return null;
    rows.push({ label, value });
    if (rows.length > 24) return null;
  }
  if (!rows.length) return null;
  return { title, rows };
}

const CHART_COLORS = [
  "#60a5fa",
  "#818cf8",
  "#34d399",
  "#fbbf24",
  "#f87171",
  "#22d3ee",
  "#c084fc",
  "#fb923c",
];

/// 条形图：```chart / ```bar。
function renderChart(lines) {
  const parsed = parseChartRows(lines);
  if (!parsed) return "";
  const { title, rows } = parsed;
  const max = Math.max(...rows.map((row) => Math.abs(row.value))) || 1;
  const bars = rows
    .map((row) => {
      const width = Math.max(2, Math.round((Math.abs(row.value) / max) * 100));
      return (
        `<div class="md-chart-row">` +
        `<span class="md-chart-label">${escapeHtml(row.label)}</span>` +
        `<span class="md-chart-track"><span class="md-chart-bar" style="width:${width}%"></span></span>` +
        `<span class="md-chart-value">${escapeHtml(String(row.value))}</span>` +
        `</div>`
      );
    })
    .join("");
  return (
    `<div class="md-chart" role="img" aria-label="${escapeAttribute(title || "数据图表")}">` +
    (title ? `<div class="md-chart-title">${escapeHtml(title)}</div>` : "") +
    bars +
    `</div>`
  );
}

/// 占比环图：```pie / ```donut。用 conic-gradient 画环，零依赖。
function renderPie(lines) {
  const parsed = parseChartRows(lines);
  if (!parsed) return "";
  const { title, rows } = parsed;
  const values = rows.map((row) => Math.abs(row.value));
  const total = values.reduce((sum, value) => sum + value, 0);
  if (!total) return "";
  let cursor = 0;
  const stops = [];
  const legend = [];
  rows.forEach((row, index) => {
    const share = values[index] / total;
    const color = CHART_COLORS[index % CHART_COLORS.length];
    const from = (cursor * 100).toFixed(2);
    cursor += share;
    const to = (cursor * 100).toFixed(2);
    stops.push(`${color} ${from}% ${to}%`);
    legend.push(
      `<li><i style="background:${color}"></i>` +
        `<span class="md-pie-label">${escapeHtml(row.label)}</span>` +
        `<b class="md-pie-value">${escapeHtml(String(row.value))}` +
        `<em>${Math.round(share * 100)}%</em></b></li>`
    );
  });
  return (
    `<div class="md-chart md-pie" role="img" aria-label="${escapeAttribute(title || "占比图")}">` +
    (title ? `<div class="md-chart-title">${escapeHtml(title)}</div>` : "") +
    `<div class="md-pie-body">` +
    `<div class="md-pie-ring" style="background:conic-gradient(${stops.join(",")})"></div>` +
    `<ul class="md-pie-legend">${legend.join("")}</ul>` +
    `</div></div>`
  );
}

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
  let quoteLines = null;
  let inTable = false;
  let tableHeader = null;
  let tableAlign = null;

  const flushCode = () => {
    if (codeLines.length) {
      const chart = /^(chart|bar|bars)$/i.test(codeLang)
        ? renderChart(codeLines)
        : /^(pie|donut|share)$/i.test(codeLang)
          ? renderPie(codeLines)
          : "";
      if (chart) {
        html.push(chart);
        codeLines = [];
        inCode = false;
        codeLang = "";
        codeFenceChar = "";
        codeFenceLength = 0;
        return;
      }
      html.push(
        `<pre class="md-code"><div class="md-code-head"><span>${escapeHtml(codeLang || "code")}</span><button class="md-copy" data-code="${encodeURIComponent(codeLines.join("\n"))}">复制</button></div><code>${escapeHtml(codeLines.join("\n"))}</code></pre>`
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
    if (!quoteLines) return;
    // 引用块内部仍是 Markdown：剥掉一层 `>` 后递归渲染，
    // 这样列表、围栏代码、表格、图表和更深层引用都能正常显示。
    const inner = quoteLines
      .map((line) => line.replace(/^\s*>\s?/, ""))
      .join("\n");
    const outerTex = TEX_STORE;
    const rendered = renderMarkdown(inner);
    // 递归会重置公式占位表：恢复外层引用，外层公式才能在整体还原时命中。
    TEX_STORE = outerTex;
    html.push('<blockquote class="md-quote">' + rendered + "</blockquote>");
    quoteLines = null;
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
    // 引用行先缓冲，整段交给递归渲染：`> >` 嵌套、`> - 列表`、`> ```代码` 都能正确成块。
    const quote = line.match(/^\s*>\s?(.*)$/);
    if (quote) {
      flushList();
      flushTable();
      if (!quoteLines) quoteLines = [];
      quoteLines.push(line);
      continue;
    }
    flushQuote();
    const heading = line.match(/^(#{1,6})\s+(.*)$/);
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
    // 列表项：保留缩进以体现层级（嵌套列表用内边距表达，不强行重排 DOM），
    // 支持 `- [ ]` / `- [x]` 任务列表与有序列表起始号。
    const unorderedItem = line.match(/^(\s*)[-*+]\s+(.*)$/);
    const orderedItem = line.match(/^(\s*)(\d+)\.\s+(.*)$/);
    const listItem = unorderedItem
      ? { tag: "ul", text: unorderedItem[2], indent: unorderedItem[1].length, start: 1 }
      : orderedItem
        ? { tag: "ol", text: orderedItem[3], indent: orderedItem[1].length, start: Number(orderedItem[2]) }
        : null;
    if (listItem) {
      flushTable();
      if (inList && inList !== listItem.tag) flushList();
      if (!inList) {
        const startAttr = listItem.tag === "ol" && listItem.start !== 1 ? ` start="${listItem.start}"` : "";
        html.push("<" + listItem.tag + ' class="md-list"' + startAttr + ">");
        inList = listItem.tag;
      }
      const depth = Math.min(3, Math.floor(listItem.indent / 2));
      const depthClass = depth ? ` md-li-depth-${depth}` : "";
      const task = listItem.text.match(/^\[( |x|X)\]\s+(.*)$/);
      const content = task
        ? `<span class="md-task${task[1].toLowerCase() === "x" ? " is-done" : ""}" aria-hidden="true">${task[1].toLowerCase() === "x" ? "✓" : "○"}</span> ${inlineMarkdown(task[2])}`
        : inlineMarkdown(listItem.text);
      html.push(`<li class="md-li${depthClass}">${content}</li>`);
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
  // 流式输出时围栏可能还没闭合：把已到达的代码先渲染出来，不能等到结束才显示。
  flushCode();
  flushList();
  flushQuote();
  flushTable();
  return restoreTex(groupMedia(html.join("\n")), TEX_STORE);
}

/// 连续独占段落的图片合成一格栅（ChatGPT 式图集）；
/// 表格外包一层滚动容器，窄屏下不撑破气泡。
function groupMedia(html) {
  // 图片各占一段时先把「连续的纯图片段落」并成一段（每次合并相邻两段，直到稳定）。
  // 不能用「只含一张图」的段落做配对：3 张以上时前面已合并成多图段落，
  // 旧实现会把第三张留在外面，或把 4 张拆成两个双图网格。
  const imageParagraph = '<div class="md-p">\\s*((?:<img class="md-img"[^>]*>\\s*)+)<\\/div>';
  const adjacent = new RegExp(`${imageParagraph}\\s*${imageParagraph}`, "g");
  let merged = html;
  for (;;) {
    const next = merged.replace(
      adjacent,
      (match, first, second) => `<div class="md-p">${first}${second}</div>`
    );
    if (next === merged) break;
    merged = next;
  }
  const grid = merged.replace(
    /<div class="md-p">\s*((?:<img class="md-img"[^>]*>\s*){2,})<\/div>/g,
    (match, images) => `<div class="md-img-grid">${images.trim()}</div>`
  );
  return grid.replace(
    /<table class="md-table">([\s\S]*?)<\/table>/g,
    (match, body, offset, source) => {
      // 引用块是递归渲染的：内层已经包过滚动容器，外层不能再套一层。
      if (source.slice(Math.max(0, offset - 32), offset).endsWith('<div class="md-table-wrap">')) {
        return match;
      }
      return `<div class="md-table-wrap"><table class="md-table">${body}</table></div>`;
    }
  );
}

/// 从已渲染的 HTML 里收集外链，生成"来源"卡片（ChatGPT 式的引用区）。
/// 只收 http(s)，按 href 去重，最多 `max` 条；没有外链时返回空串。
function collectSources(html, limit) {
  const max = Math.max(1, Math.min(20, Number(limit || 8)));
  const found = [];
  const seen = new Set();
  // 站内链接（相对路径会被 safeMarkdownHref 规范化成同源绝对地址）不算"来源"。
  let sameOrigin = "";
  try {
    sameOrigin = new URL(window.location.href).origin;
  } catch (_) {
    sameOrigin = "";
  }
  const pattern = /<a\s[^>]*href="([^"]+)"[^>]*>([\s\S]*?)<\/a>/g;
  let match;
  while ((match = pattern.exec(String(html || ""))) !== null) {
    const href = match[1];
    if (!/^https?:\/\//i.test(href) || seen.has(href)) continue;
    if (sameOrigin && href.startsWith(sameOrigin + "/")) continue;
    seen.add(href);
    let host = href;
    try {
      host = new URL(href).host;
    } catch (_) {
      // 解析失败就退回原 URL。
    }
    // 正文里的标签已被转义成实体：先解回字符再剥标签，避免卡片显示 `&lt;img`。
    const plain = match[2]
      .replace(/&lt;/g, "<")
      .replace(/&gt;/g, ">")
      .replace(/&quot;/g, '"')
      .replace(/&#39;/g, "'")
      .replace(/&amp;/g, "&");
    const label = plain.replace(/<[^>]+>/g, "").trim() || host;
    found.push({ href, host, label: label.slice(0, 100) });
    if (found.length >= max) break;
  }
  return found;
}

function renderSourceCards(html, options) {
  const found = collectSources(html, options && options.max);
  if (!found.length) return "";
  const items = found
    .map(
      (source, index) =>
        `<a class="md-source" href="${escapeAttribute(source.href)}" target="_blank" rel="noopener">` +
        `<span class="md-source-index">${index + 1}</span>` +
        `<span class="md-source-body">` +
        `<span class="md-source-label">${escapeHtml(source.label)}</span>` +
        `<span class="md-source-host">${escapeHtml(source.host)}</span>` +
        "</span></a>"
    )
    .join("");
  return `<div class="md-sources"><div class="md-sources-title">来源</div><div class="md-sources-list">${items}</div></div>`;
}

/// 把正文里的 `[1] [2]` 变成指向来源卡片的锚点（ChatGPT 式行内引用）。
/// 只处理标签之外的文本，且跳过 code/pre/a 内部；没有对应来源时保持原样。
function linkCitations(html, sources) {
  if (!Array.isArray(sources) || !sources.length) return html;
  const byIndex = new Map();
  sources.forEach((source, index) => byIndex.set(String(index + 1), source));
  let insideCode = 0;
  let insideAnchor = 0;
  return String(html || "")
    .split(/(<[^>]+>)/)
    .map((part) => {
      if (!part) return part;
      if (part.startsWith("<")) {
        const name = part.replace(/^<\/?/, "").split(/[\s/>]/)[0].toLowerCase();
        const closing = part.startsWith("</");
        const selfClosing = /\/>$/.test(part);
        if (name === "code" || name === "pre") {
          insideCode += selfClosing ? 0 : closing ? -1 : 1;
          if (insideCode < 0) insideCode = 0;
        } else if (name === "a") {
          insideAnchor += selfClosing ? 0 : closing ? -1 : 1;
          if (insideAnchor < 0) insideAnchor = 0;
        }
        return part;
      }
      if (insideCode > 0 || insideAnchor > 0) return part;
      return part.replace(/\[(\d{1,2})\]/g, (match, digits) => {
        const source = byIndex.get(digits);
        if (!source) return match;
        return (
          `<a class="md-cite" href="${escapeAttribute(source.href)}" ` +
          `target="_blank" rel="noopener" title="${escapeAttribute(source.label)}">${digits}</a>`
        );
      });
    })
    .join("");
}

/// 助手回答的完整渲染：Markdown + 行内引用 + 来源卡片（流式增量仍用 renderMarkdown，避免卡片抖动）。
function renderMessage(text) {
  const html = renderMarkdown(text);
  const sources = collectSources(html);
  return linkCitations(html, sources) + renderSourceCards(html);
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
    renderSourceCards,
    renderMessage,
    linkCitations,
    renderChart,
    renderPie,
    collectSources,
  });
})(typeof window !== "undefined" ? window : globalThis);
