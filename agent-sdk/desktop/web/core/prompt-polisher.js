/* 提示词优化器（2026-11-13）。
 *
 * 设计约束：
 * - 本地、确定性、零模型调用：不把用户草稿发到任何端点，也不消耗额度。
 * - 只做“编排与补全”，不改写事实：原句逐字保留，新增内容只来自意图模板与
 *   通用质量约束；缺少的关键背景以一句“先提问”兜底，不凭空编造。
 * - 语言跟随原文：中文输入只加中文骨架，英文输入只加英文骨架。
 * - 纯逻辑、无 DOM，供 core/prompt-polish-ui.js 与单元测试直接复用。
 */
(function (global) {
  "use strict";

  const MIN_ZH_CHARS = 6;
  const MIN_EN_WORDS = 3;
  const COMPLETE_SCORE = 55;

  // 完整度维度权重：目标 22 + 背景 16 + 约束 16 + 输出 14 + 验收 14
  // + 具体信息 10 + 结构 8 = 100。阈值用于区分“可优化”和“已完善”。
  const SCORE_SIGNALS = {
    zh: {
      goal: /(实现|修复|编写|撰写|写|生成|分析|总结|翻译|设计|重构|调研|对比|比较|解释|优化|整理|创建|开发|排查|制定|评估|给出|列出|说明|检查|测试|部署|搭建|画|做|改|润色|扩写|改写|提取|转换|推荐|规划|审查|审阅|归纳)/g,
      context: /(背景|现状|目前|现在|已有|基于|针对|面向|用于|场景|项目|仓库|代码库|文件|目录|数据|用户|读者|受众|客户|学生|同事|团队|输入|给定|已有材料)/g,
      constraints: /(必须|不要|不得|不能|只能|仅限|约束|限制|保持|兼容|避免|不超过|不少于|至少|最多|字数|语气|风格|遵循|遵守|符合|不(?:引入|使用|修改|删除|改变|影响))/g,
      format: /(输出|返回|格式|列表|表格|JSON|Markdown|分点|要点|步骤|大纲|代码块|代码|用例|命令|示例|模板|结构|篇幅|字数|段落|报告|文案|摘要)/gi,
      criteria: /(验收|标准|确保|保证|通过|测试|成功|正确|可运行|无错误|覆盖|预期|边界|异常)/g,
      must: /(必须|不要|不得|不能|只能|仅限)/g,
    },
    en: {
      goal: /\b(implement|fix|write|create|build|generate|analy[sz]e|summari[sz]e|translate|design|refactor|research|compare|explain|optimi[sz]e|organi[sz]e|develop|debug|review|evaluate|list|describe|test|deploy|draft|rewrite|polish)\b/gi,
      context: /\b(context|background|current|existing|based on|audience|reader|customer|user|project|repo|repository|codebase|file|directory|data|dataset|for (?:a|an|the) [a-z]+)\b/gi,
      constraints: /\b(must|must not|should|should not|don't|do not|avoid|only|limit|within|at most|at least|no more than|keep|compatible|without|constraint|require[sd]?)\b/gi,
      format: /\b(output|return|format|json|markdown|table|list|steps?|outline|code block|code|tests?|command|example|template|structure|length|words?|paragraphs?|report|summary|draft)\b/gi,
      criteria: /\b(acceptance|criteria|ensure|verify|pass|test|correct|expected|edge case|works?|success)\b/gi,
      must: /\b(must|must not|don't|do not|only|never)\b/gi,
    },
  };

  const INTENT_PATTERNS = {
    zh: {
      translation: /翻译|译成|译文|本地化/,
      image: /图片|海报|插画|图标|封面|配图|摄影|画面|生成图/,
      code: /代码|函数|脚本|程序|报错|异常|接口|数据库|重构|单元测试|编译|部署|仓库|实现|开发|调试|bug/i,
      analysis: /分析|研究|调研|对比|比较|评估|数据|趋势|原因|论证|洞察/,
      planning: /计划|方案|规划|流程|步骤|路线|里程碑|策略|安排|排期/,
      writing: /写|文案|文章|报告|邮件|总结|润色|扩写|改写|标题|简介|小说|演讲稿|说明文/,
      explain: /解释|讲解|说明|是什么|为什么|原理|区别|科普/,
    },
    en: {
      translation: /\btranslat|localiz/i,
      image: /\b(image|poster|illustration|icon|cover|photo|picture|visual)\b/i,
      code: /\b(code|function|script|program|bug|error|api|database|refactor|unit test|compile|deploy|repo|implement|debug)\b/i,
      analysis: /\b(analy[sz]e|research|compare|evaluate|data|trend|cause|evidence|insight)\b/i,
      planning: /\b(plan|proposal|roadmap|workflow|steps?|milestone|strategy|schedule)\b/i,
      writing: /\b(write|article|essay|report|email|summari[sz]e|polish|rewrite|copy|blog|draft|title)\b/i,
      explain: /\b(explain|what is|why|how does|concept|difference)\b/i,
    },
  };

  // 每种意图只补“输出形式”和“验收方式”两类高价值信息；不补领域事实。
  const INTENT_TEXT = {
    zh: {
      translation: {
        format: "只输出译文；保留原文的段落、标题、列表与术语格式。",
        criteria: "不增删信息，术语前后一致，符合目标语言习惯。",
        guard: "保持原文语气与专有名词，不做润色式改写。",
      },
      image: {
        format: "按主体、场景、构图、光线、风格、色调、画面比例组织描述，并单列需要避免的元素。",
        criteria: "画面元素不冲突，主体清晰，风格与用途一致，比例适合目标载体。",
        guard: "不堆砌互相冲突的风格词；不用抽象口号代替可见细节。",
      },
      code: {
        format: "给出改动点、关键代码（标注文件路径）和可复现的验证命令；不要贴无关代码。",
        criteria: "代码可运行，现有测试通过；说明边界情况、失败路径与回滚方式。",
        guard: "保持现有技术栈与代码风格，不做与目标无关的重构。",
      },
      analysis: {
        format: "结论先行，再给依据、数据来源与不确定性；需要时用表格对比。",
        criteria: "每个结论都有证据支撑，明确区分事实、推断与假设。",
        guard: "不编造数据或来源；证据不足时直接说明。",
      },
      planning: {
        format: "输出目标、阶段或里程碑、依赖、风险与下一步行动，按优先级排序。",
        criteria: "步骤可执行，标明前置条件与完成标志；风险有对应缓解办法。",
        guard: "不写空泛口号；每个阶段都要有可检查的产出。",
      },
      writing: {
        format: "先给结构大纲，再给完整成稿；按段落组织，必要时配标题。",
        criteria: "信息准确、无空话套话；语气与目标读者匹配，篇幅符合要求。",
        guard: "不编造事实、数据或引用；需要引用时标明来源。",
      },
      explain: {
        format: "先给一句话结论，再分层解释；必要时给例子或类比。",
        criteria: "概念准确，例子可验证，避免循环定义和过度类比。",
        guard: "面向目标读者的已有认知解释，不堆砌术语。",
      },
      general: {
        format: "先给结论，再给依据和可执行的下一步；需要时用列表或表格。",
        criteria: "逐条覆盖原话中的要求，明确假设与不确定性，给出可检查的交付物。",
        guard: "不改变原意，不扩大或缩小任务范围。",
      },
    },
    en: {
      translation: {
        format: "Return only the translation; preserve paragraphs, headings, lists and terminology formatting.",
        criteria: "No information added or removed; terminology is consistent and idiomatic in the target language.",
        guard: "Preserve the original tone and proper nouns; do not rewrite for style.",
      },
      image: {
        format: "Describe subject, setting, composition, lighting, style, palette and aspect ratio; list elements to avoid separately.",
        criteria: "Elements do not conflict, the subject reads clearly, and the style and aspect ratio fit the intended use.",
        guard: "Avoid contradictory style words; replace abstract slogans with visible details.",
      },
      code: {
        format: "Provide the change points, key code with file paths, and reproducible verification commands; omit unrelated code.",
        criteria: "The code runs, existing tests pass, and edge cases, failure paths and rollback are explained.",
        guard: "Keep the existing stack and code style; do not refactor beyond the task.",
      },
      analysis: {
        format: "Lead with the conclusion, then evidence, sources and uncertainty; use a table for comparisons when useful.",
        criteria: "Every claim is supported by evidence, with facts, inferences and assumptions clearly separated.",
        guard: "Do not invent data or sources; say so when evidence is insufficient.",
      },
      planning: {
        format: "Give the goal, phases or milestones, dependencies, risks and next actions, ordered by priority.",
        criteria: "Steps are actionable with prerequisites and done criteria; each risk has a mitigation.",
        guard: "Avoid vague slogans; every phase needs a checkable output.",
      },
      writing: {
        format: "Start with an outline, then provide the full draft in paragraphs, with headings where useful.",
        criteria: "Accurate and free of filler; tone fits the audience and the length matches the request.",
        guard: "Do not invent facts, data or citations; cite sources when claims require them.",
      },
      explain: {
        format: "Open with a one-sentence answer, then explain in layers with examples or analogies where useful.",
        criteria: "Concepts are accurate, examples are verifiable, and definitions are not circular.",
        guard: "Build on the audience's existing knowledge; avoid unexplained jargon.",
      },
      general: {
        format: "Lead with the conclusion, then evidence and concrete next steps; use lists or tables when helpful.",
        criteria: "Cover every requirement in the original request, state assumptions and uncertainty, and deliver something checkable.",
        guard: "Do not change the intent or expand/shrink the scope.",
      },
    },
  };

  const LABELS = {
    zh: {
      goal: "目标明确",
      context: "背景充分",
      constraints: "约束清楚",
      format: "输出明确",
      criteria: "验收清楚",
      preserved: "保留原句与语言",
      ask: "缺失信息先提问",
      formatNote: "补充输出格式",
      criteriaNote: "补充验收标准",
      scopeNote: "补充执行边界",
      templateNote: "给出可填写的起步模板",
      noRewriteNote: "未扩写，避免套话",
    },
    en: {
      goal: "Clear goal",
      context: "Enough context",
      constraints: "Clear constraints",
      format: "Defined output",
      criteria: "Clear acceptance",
      preserved: "Kept wording and language",
      ask: "Asks before assuming",
      formatNote: "Added an output format",
      criteriaNote: "Added acceptance criteria",
      scopeNote: "Added execution boundaries",
      templateNote: "Provided a fill-in starter",
      noRewriteNote: "Not padded with generic prose",
    },
  };

  function asText(value) {
    return String(value == null ? "" : value).replace(/\r\n?/g, "\n");
  }

  function count(text, re) {
    const matches = text.match(re);
    return matches ? matches.length : 0;
  }

  function detectLanguage(compact) {
    if (!compact) return "zh";
    const cjk = count(compact, /[\u3400-\u9fff]/g);
    const latin = count(compact, /[A-Za-z][A-Za-z0-9'’-]*/g);
    if (cjk && latin) return cjk >= latin ? "zh" : "en";
    if (cjk) return "zh";
    return "en";
  }

  function detectIntent(compact, language) {
    const patterns = INTENT_PATTERNS[language] || INTENT_PATTERNS.zh;
    const order = ["translation", "image", "code", "analysis", "planning", "writing", "explain"];
    for (const key of order) {
      if (patterns[key] && patterns[key].test(compact)) return key;
    }
    return "general";
  }

  function analyze(text) {
    const original = asText(text);
    const trimmed = original.trim();
    const compact = trimmed.replace(/\s+/g, " ");
    const language = detectLanguage(compact);
    const signals = SCORE_SIGNALS[language] || SCORE_SIGNALS.zh;
    const chars = compact.length;
    const words = count(compact, /[A-Za-z][A-Za-z0-9'’-]*/g);
    const empty = compact.length === 0;
    // 除了字数，纯口令式短语（“帮我看看这个”“优化一下”）也算信息不足：
    // 它们没有可保留的对象，直接扩写只会得到套话。
    const vagueOnly = chars < 12 &&
      /^(?:帮我|请|麻烦)?\s*(?:看看|看一下|改改|改一下|搞一下|弄一下|优化一下|处理一下|整理一下)/.test(compact);
    const tooShort = !empty && (language === "en" ? words < MIN_EN_WORDS : chars < MIN_ZH_CHARS || vagueOnly);
    const intent = empty ? "general" : detectIntent(compact, language);

    const hasDigits = /[\d０-９]/.test(compact);
    const hasPath = /(?:[A-Za-z]:\\|\/[\w.-]+\/|\.(?:ts|tsx|js|jsx|py|rs|json|md|txt|csv|xlsx|docx|pdf|html|css|ya?ml|toml))\b/i.test(compact);
    const hasQuote = /["“”'‘’][^"“”'‘’]{2,}["“”'‘’]/.test(compact);
    const hasTechnical = /\b[A-Z]{2,}\b/.test(compact) || /`[^`]{1,40}`/.test(compact);
    const specificity = Math.min(10,
      (hasDigits ? 4 : 0) +
      (hasPath ? 4 : 0) +
      (hasQuote ? 2 : 0) +
      (hasTechnical ? 3 : 0));

    const goalHits = count(compact, signals.goal);
    const goal = Math.min(22,
      (goalHits ? 12 : 0) +
      (chars >= 14 ? 4 : 0) +
      (chars >= 40 ? 3 : 0) +
      (specificity >= 4 ? 3 : 0));

    const contextHits = count(compact, signals.context);
    const context = Math.min(16,
      Math.min(10, contextHits * 4) +
      (chars >= 50 ? 3 : 0) +
      (hasPath ? 3 : 0));

    const constraintHits = count(compact, signals.constraints);
    const mustHits = count(compact, signals.must);
    const constraints = Math.min(16,
      Math.min(12, constraintHits * 5) +
      (mustHits ? 4 : 0));

    const formatHits = count(compact, signals.format);
    const format = Math.min(14,
      Math.min(12, formatHits * 5) +
      (/(输出|output|format)/i.test(compact) ? 2 : 0));

    const criteriaHits = count(compact, signals.criteria);
    const criteria = Math.min(14, criteriaHits * 7);

    const headingHits = count(trimmed, /(?:^|\n)#{1,4}\s/g);
    const listHits = count(trimmed, /(?:^|\n)\s*(?:[-*]|\d+[.)])\s/g);
    const structured = headingHits > 0 || listHits >= 2;
    const structure = Math.min(8, (headingHits ? 4 : 0) + (listHits >= 2 ? 4 : 0));

    const total = Math.max(0, Math.min(100, goal + context + constraints + format + criteria + specificity + structure));
    const present = {
      goal: goal >= 10,
      context: context >= 6,
      constraints: constraints >= 6,
      format: format >= 7,
      criteria: criteria >= 7,
    };
    const missing = Object.keys(present).filter((key) => !present[key]);
    const labels = LABELS[language] || LABELS.zh;
    const strengths = ["goal", "context", "constraints", "format", "criteria"]
      .filter((key) => present[key])
      .map((key) => labels[key]);
    // “已完善”不是单看长度：目标、输出、验收三件套必须在场，且至少交代了
    // 背景或约束之一；否则再长的草稿也只是“写得长”，仍然值得补全。
    const complete = !empty && !tooShort && total >= COMPLETE_SCORE && chars >= 40 &&
      present.goal && present.format && present.criteria && (present.context || present.constraints);

    return {
      text: trimmed,
      compact,
      language,
      intent,
      chars,
      words,
      empty,
      tooShort,
      score: total,
      present,
      missing,
      strengths,
      structured,
      complete,
      dimensions: { goal, context, constraints, format, criteria, specificity, structure },
    };
  }

  function scaffold(language) {
    if (language === "en") {
      return [
        "Goal: [specific outcome you want]",
        "Context: [audience, current state, existing material]",
        "Constraints: [must / must not / tools or format to keep]",
        "Output: [format, length, who will read it]",
        "Acceptance: [how we know it is done]",
      ].join("\n");
    }
    return [
      "我要完成：〔具体目标，例如“修复登录页在窄窗口下错位”〕",
      "背景：〔对象、现状、已有材料〕",
      "要求：〔必须满足 / 不要出现 / 要保留的格式〕",
      "输出：〔形式、篇幅、给谁看〕",
      "验收：〔怎样算完成〕",
    ].join("\n");
  }

  function compose(analysis) {
    const language = analysis.language;
    const intent = INTENT_TEXT[language][analysis.intent] || INTENT_TEXT[language].general;
    const labels = LABELS[language];
    const lines = [analysis.text, ""];
    if (language === "en") {
      lines.push("## Requirements");
      lines.push("- " + intent.guard);
      lines.push("- If critical information is missing, ask up to 3 clarifying questions before starting.");
      if (!analysis.present.format) {
        lines.push("", "## Output");
        lines.push("- " + intent.format);
      }
      if (!analysis.present.criteria) {
        lines.push("", "## Acceptance");
        lines.push("- " + intent.criteria);
      }
      if (analysis.present.format && analysis.present.criteria) {
        lines.push("", "## Done when");
        lines.push("- Every requirement in the original request is covered; restate the goal and deliverable before executing.");
      }
    } else {
      lines.push("## 执行要求");
      lines.push("- " + intent.guard);
      lines.push("- 如果缺少会影响结果的关键信息，先提出不超过 3 个澄清问题，再开始。");
      if (!analysis.present.format) {
        lines.push("", "## 输出");
        lines.push("- " + intent.format);
      }
      if (!analysis.present.criteria) {
        lines.push("", "## 验收");
        lines.push("- " + intent.criteria);
      }
      if (analysis.present.format && analysis.present.criteria) {
        lines.push("", "## 完成标准");
        lines.push("- 逐条覆盖原话中的要求；执行前先用一句话复述目标与交付物，确认无歧义。");
      }
    }
    return lines.join("\n");
  }

  function augment(analysis) {
    const language = analysis.language;
    const intent = INTENT_TEXT[language][analysis.intent] || INTENT_TEXT[language].general;
    const parts = [];
    if (language === "en") {
      if (!analysis.present.format) parts.push("- Output: " + intent.format);
      if (!analysis.present.criteria) parts.push("- Acceptance: " + intent.criteria);
      if (!parts.length) parts.push("- Restate the goal and deliverable before executing to confirm alignment.");
      parts.push("- Ask up to 3 questions when critical context is missing; do not invent details.");
      return analysis.text + "\n\n## Additional requirements\n" + parts.join("\n");
    }
    if (!analysis.present.format) parts.push("- 输出：" + intent.format);
    if (!analysis.present.criteria) parts.push("- 验收：" + intent.criteria);
    if (!parts.length) parts.push("- 执行前先用一句话复述目标与交付物，确认理解一致。");
    parts.push("- 缺少关键背景时先提问，最多 3 个；不要编造细节。");
    return analysis.text + "\n\n## 补充要求\n" + parts.join("\n");
  }

  function optimize(text, options) {
    const opts = options || {};
    const analysis = analyze(text);
    const labels = LABELS[analysis.language] || LABELS.zh;
    const base = {
      language: analysis.language,
      intent: analysis.intent,
      original: analysis.text,
      scoreBefore: analysis.score,
      analysis,
    };

    if (analysis.empty) {
      return Object.assign({}, base, {
        status: "empty",
        prompt: "",
        notes: [],
        scoreAfter: 0,
      });
    }

    if (analysis.tooShort) {
      const prompt = scaffold(analysis.language);
      return Object.assign({}, base, {
        status: "too_short",
        prompt,
        template: true,
        notes: [labels.noRewriteNote, labels.templateNote],
        scoreAfter: analyze(prompt).score,
      });
    }

    if (analysis.complete && !opts.force) {
      return Object.assign({}, base, {
        status: "complete",
        prompt: analysis.text,
        notes: analysis.strengths.slice(0, 4),
        scoreAfter: analysis.score,
      });
    }

    const prompt = analysis.structured ? augment(analysis) : compose(analysis);
    const notes = [labels.preserved];
    if (!analysis.present.format) notes.push(labels.formatNote);
    if (!analysis.present.criteria) notes.push(labels.criteriaNote);
    if (!analysis.present.constraints) notes.push(labels.scopeNote);
    if (notes.length < 4) notes.push(labels.ask);
    return Object.assign({}, base, {
      status: "ok",
      prompt,
      notes: notes.slice(0, 4),
      scoreAfter: analyze(prompt).score,
      missing: analysis.missing,
    });
  }

  const api = {
    analyze,
    optimize,
    scaffold,
    MIN_ZH_CHARS,
    MIN_EN_WORDS,
    COMPLETE_SCORE,
  };

  global.OwoPromptPolisher = api;
  if (typeof module !== "undefined" && module.exports) module.exports = api;
})(typeof window !== "undefined" ? window : globalThis);
