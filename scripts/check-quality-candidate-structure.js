// Observational structure audit, not a language-quality or universal quota oracle.
const fs = require('fs');
const path = require('path');
const crypto = require('crypto');
const root = path.resolve(process.argv[2] || 'build/quality-review-20261029');
const output = path.resolve(process.argv[3] || 'docs/quality-review/2026-10-29');
const runs = fs.readFileSync(path.join(root, 'rounds.jsonl'), 'utf8').trim().split(/\r?\n/).map(JSON.parse);
const issues = new Map();
const layouts = new Map();
let observations = 0, currentRuns = 0;
function add(test, kind, evidence, run, runtime, pageSize, diagnostic = false) {
  const key = JSON.stringify([test.input, test.context, test.previous, test.correction, pageSize, kind]);
  const collection = diagnostic ? layouts : issues;
  if (!collection.has(key)) collection.set(key, { id: 'structure-' + crypto.createHash('sha256').update(key).digest('hex').slice(0, 12), kind, input: test.input, expected: test.expected, status: diagnostic ? '规则核对（非缺陷断言）' : '待批阅', observations: [] });
  collection.get(key).observations.push({ round: run.round, backend: run.backend, runtime, report: run.report, case_id: test.id, page_size: pageSize, evidence });
}
for (const run of runs) {
  const rows = fs.readFileSync(path.join(run.report, 'cases.jsonl'), 'utf8').trim().split(/\r?\n/).map(JSON.parse);
  const manifest = rows[0];
  if (manifest.record !== 'manifest') throw Error('Missing manifest');
  const runtime = manifest.bridge ? (/runtime-p1-x86_64/.test(manifest.bridge.path) ? 'release-legacy' : 'current') : 'base';
  if (runtime === 'release-legacy') continue;
  currentRuns++;
  const pageSize = +manifest.effective_config.match(/^candidate_page_size=(\d+)$/m)[1];
  const partition = /^candidate_partition_enabled=true$/m.test(manifest.effective_config);
  for (const test of rows.filter(row => row.record === 'case' && !row.runtime_error)) {
    observations++;
    for (const [label, candidates] of [['base', test.base_candidates], ['final', test.final_first_page]]) {
      const seen = new Map();
      const duplicates = [], malformed = [];
      for (const item of candidates) {
        if (!item.text || item.consumed < 1 || item.consumed > test.input.length || !(partition ? [0, 1, 2] : [0, 1, 2, 3]).includes(item.group) || item.index < 1 || item.index > pageSize || item.page < 0) malformed.push(item);
        const tuple = JSON.stringify([item.text, item.consumed, item.group]);
        if (seen.has(tuple)) duplicates.push({ before: seen.get(tuple), after: item });
        else seen.set(tuple, item);
      }
      if (duplicates.length) add(test, label === 'base' ? '扫描分页出现重复候选元组' : '最终首页出现重复候选元组', { view: label, duplicates }, run, runtime, pageSize);
      if (malformed.length) add(test, '候选字段范围或分组标识异常', { view: label, items: malformed }, run, runtime, pageSize);
      if (partition) {
        const firstPage = candidates.filter(item => item.page === 0);
        const groups = firstPage.map(item => item.group);
        let otherSeen = false;
        if (groups.some(group => group === 0 ? otherSeen : (otherSeen = true, false))) add(test, '首页组0出现在其他候选组后（规则待评）', { view: label, first_page: firstPage }, run, runtime, pageSize, true);
        if (firstPage.filter(item => item.group === 0).length > 2) add(test, '首页组0超过两个（含合并词句区）', { view: label, first_page: firstPage }, run, runtime, pageSize, true);
      }
    }
  }
}
const all = [...issues.values()].sort((a, b) => a.kind.localeCompare(b.kind) || a.input.localeCompare(b.input));
fs.mkdirSync(output, { recursive: true });
fs.writeFileSync(path.join(output, 'structure-issues.jsonl'), all.map(x => JSON.stringify(x)).join('\n') + (all.length ? '\n' : ''));
fs.writeFileSync(path.join(output, 'structure-layout-observations.jsonl'), [...layouts.values()].map(x => JSON.stringify(x)).join('\n') + (layouts.size ? '\n' : ''));
const counts = Object.fromEntries([...new Set(all.map(x => x.kind))].map(kind => [kind, all.filter(x => x.kind === kind).length]));
const summary = { kind: 'observational_candidate_structure_audit', current_and_base_runs: currentRuns, observations, issues: all.length, classes: counts, layout_rule_observations_not_defects: layouts.size, note: '重复按文字+消费范围+分组判断；跨请求扫描可能受候选池扩容影响，不能直接当作同一静态池错误。组0包含合并词句区，不等同于纯句子。CLI在无分组时用3表示未分区，合法。未执行GUI。' };
fs.writeFileSync(path.join(output, 'structure-summary.json'), JSON.stringify(summary, null, 2));
const escape = value => String(value).replaceAll('|', '\\|').replace(/[\r\n]/g, ' ');
const markdown = ['# 候选结构巡检（待批阅）', '', `检查当前桥接及基础候选 ${currentRuns} 个后端轮次、${observations} 条观测，得到 ${all.length} 个去重条目。`, '', summary.note, '', '|类型|条目数|', '|---|---:|', ...Object.entries(counts).map(([kind, count]) => `|${kind}|${count}|`), '', '## 示例（前30项，完整证据见 structure-issues.jsonl）', '', '|ID|输入|类型|观测数|', '|---|---|---|---:|', ...all.slice(0, 30).map(issue => `|${issue.id}|${escape(issue.input)}|${issue.kind}|${issue.observations.length}|`), '', `另有 ${layouts.size} 项布局规则观察写入 structure-layout-observations.jsonl，不计为缺陷条目。`, '例如 bgs 的办公室等字典词可能共用组0；安全可以优先于更长的“按去按”组合。因此不能仅凭组0数量或位置认定违反“长句最多两个”规则。', '不硬编码字典/单字区配额，因为补位、单字优先和词句合并会改变合法布局。'];
fs.writeFileSync(path.join(output, 'structure-issues.md'), markdown.join('\n') + '\n');
console.log(JSON.stringify(summary));
