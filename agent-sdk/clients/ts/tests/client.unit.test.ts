import test from "node:test";
import assert from "node:assert/strict";
import { createClient, type TurnEvent } from "../src/index.js";
import type { components, operations, paths } from "../src/schema.js";

test("runTurn 复用自定义请求头并处理无换行的最后一个 SSE 事件", async () => {
  const originalFetch = globalThis.fetch;
  let requestUrl = "";
  let requestHeaders: Headers | undefined;

  globalThis.fetch = async (input, init) => {
    requestUrl = String(input);
    requestHeaders = new Headers(init?.headers);
    return new Response('data: {"type":"final","text":"完成"}\n\ndata: {"type":"turn_stats","completion_status":"accepted","completion_record":{"task_id":"session/1","attempt_id":"turn-1","status":"accepted","evidence_receipt_ids":["receipt-1"],"candidate_version_sha256":"sha256:abc","decided_at":"2026-10-05T00:00:00Z"}}', {
      status: 200,
      headers: { "Content-Type": "text/event-stream" },
    });
  };

  try {
    const events: TurnEvent[] = [];
    const client = createClient({
      baseUrl: "http://127.0.0.1:4096/",
      headers: { Authorization: "Bearer test-token", "X-Client": "unit" },
    });

    const completion = await client.runTurn(
      { id: "session/1", prompt: "测试" },
      { onEvent: (event) => events.push(event) },
    );

    assert.equal(
      requestUrl,
      "http://127.0.0.1:4096/session/session%2F1/turn",
    );
    assert.equal(requestHeaders?.get("Authorization"), "Bearer test-token");
    assert.equal(requestHeaders?.get("X-Client"), "unit");
    assert.equal(requestHeaders?.get("Content-Type"), "application/json");
    assert.equal(completion.completionStatus, "accepted");
    assert.deepEqual(completion.stats.completion_record, {
      task_id: "session/1",
      attempt_id: "turn-1",
      status: "accepted",
      evidence_receipt_ids: ["receipt-1"],
      candidate_version_sha256: "sha256:abc",
      decided_at: "2026-10-05T00:00:00Z",
    });
    assert.deepEqual(events[1].completion_record, completion.stats.completion_record);
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("pre-aborted run does not submit or cancel any turn", async () => {
  const originalFetch = globalThis.fetch;
  const requestedUrls: string[] = [];
  globalThis.fetch = async (input) => {
    requestedUrls.push(String(input));
    return new Response("", { status: 499 });
  };
  try {
    const controller = new AbortController(); controller.abort();
    const client = createClient({ baseUrl: "http://127.0.0.1:4096" });
    await assert.rejects(client.runTurn({ id: "session", prompt: "cancel" },
      { onEvent: () => undefined, signal: controller.signal }), { name: "AbortError" });
    assert.deepEqual(requestedUrls, []);
  } finally { globalThis.fetch = originalFetch; }
});

test("runTurn 在 SSE 没有 final 事件时失败", async () => {
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async () =>
    new Response('data: {"type":"progress","message":"处理中"}', {
      status: 200,
      headers: { "Content-Type": "text/event-stream" },
    });

  try {
    const client = createClient({ baseUrl: "http://127.0.0.1:4096" });
    await assert.rejects(
      client.runTurn(
        { id: "session", prompt: "截断" },
        { onEvent: () => undefined },
      ),
      /未收到 final 事件/,
    );
  } finally {
    globalThis.fetch = originalFetch;
  }
});

// ---------------------------------------------------------------------------
// R0 契约同步（第一路）：候选治理字段 + retry 冻结契约的 TS 类型级断言。
// 本用例不发起网络请求——任何字段漂移都会先在 tsc 编译期失败。
// ---------------------------------------------------------------------------

test("ModelCandidate 治理字段契约（provider_ref / provider / calibration_summary / gates）", () => {
  type Candidate = components["schemas"]["ModelCandidate"];
  type ProviderRef = components["schemas"]["CandidateProviderRef"];
  type Calibration = components["schemas"]["CalibrationReport"];
  type RegisterBody = NonNullable<
    operations["modelCandidateRegister"]["requestBody"]
  >["content"]["application/json"];
  type PromoteResponse = operations["modelCandidatePromote"]["responses"][200]["content"]["application/json"];

  // 注册请求侧：provider_ref（声明 ≠ 接线；external 与 metadata_only 双形态）。
  const registerBody: RegisterBody = {
    model_id: "wm-rule",
    model_version: "1.0.0",
    provider_ref: { type: "external", kind: "wm1-http", locator: "http://127.0.0.1:9/v1" },
  };
  assert.equal(registerBody.provider_ref?.type, "external");
  const metadataOnly: NonNullable<RegisterBody["provider_ref"]> = { type: "metadata_only" };
  assert.equal(metadataOnly.type, "metadata_only");

  // 候选响应侧：wire 字段名是 provider（非 provider_ref）+ calibration_summary（可空）。
  const candidate: Candidate = {
    candidate_id: "c-1",
    model_id: "wm-rule",
    model_version: "1.0.0",
    source: "手动注册（shadow 起步）",
    status: "shadow",
    created_at: "2026-08-27T00:00:00Z",
    promoted_at: null,
    promote_reason: null,
    provider: { type: "metadata_only" },
    calibration_summary: null,
  };
  const wired: Extract<ProviderRef, { type: "external" }> = {
    type: "external",
    kind: "local-onnx",
    locator: "file:///models/wm.onnx",
  };
  candidate.provider = wired;
  const calibration: Calibration = {
    samples: 16,
    success_hit_rate: 0.9,
    mean_calibration_error: 0.1,
    mean_delta_jaccard: 0.2,
    uncertainty_buckets: [{ label: "0.0-0.25", samples: 4, hit_rate: 0.75 }],
  };
  candidate.status = "active";
  candidate.calibration_summary = calibration;

  // 晋升响应侧：gates 明细（min_shadow_samples / provider_wired / calibration_summary / regression_check）。
  const promoted: PromoteResponse = {
    candidate,
    active: "c-1",
    previous_active: null,
    samples: 16,
    gates: {
      min_shadow_samples: 16,
      provider_wired: { kind: "wm1-http", locator: "http://127.0.0.1:9/v1" },
      calibration_summary: calibration,
      regression_check: null,
    },
  };
  assert.equal(promoted.gates.min_shadow_samples, 16);
  assert.equal(promoted.gates.calibration_summary.samples, 16);
});

test("steer retry 冻结契约（command 枚举含 retry，step_id + note）", () => {
  type SteerBody = NonNullable<
    operations["workswarmSteerTeam"]["requestBody"]
  >["content"]["application/json"];
  const retryBody: SteerBody = {
    command: "retry",
    step_id: "builder",
    note: "修复输入后重试",
  };
  assert.equal(retryBody.command, "retry");
  assert.equal(retryBody.step_id, "builder");
  const continueBody: SteerBody = { command: "continue" };
  assert.equal(continueBody.command, "continue");
});

test("R2 additive：interrupted 布尔位出现在 teams 列表 / 详情 / steer / events 快照", () => {
  type SteerResponse =
    operations["workswarmSteerTeam"]["responses"][200]["content"]["application/json"];
  type TeamDetail =
    operations["workswarmGetTeam"]["responses"][200]["content"]["application/json"];
  type TeamList =
    operations["workswarmListTeams"]["responses"][200]["content"]["application/json"];
  type EventsSnapshot =
    operations["workswarmTeamEvents"]["responses"][200]["content"]["application/json"];

  const steer: SteerResponse = { team_id: "t", status: "Running", interrupted: false };
  assert.equal(typeof steer.interrupted, "boolean");

  const detail: TeamDetail = {
    team: {},
    interrupted: true,
    tasks: {},
    audit_tail: [{ ts: "2026-08-27T00:00:00Z", event: "team.interrupted", detail: "进程重启中断识别" }],
  };
  assert.equal(detail.interrupted, true);

  const list: TeamList = { teams: [{ active: false, interrupted: false }] };
  assert.equal(typeof list.teams[0].interrupted, "boolean");

  const snapshot: EventsSnapshot = {
    team_id: "t",
    status: "Running",
    active: false,
    interrupted: true,
    audit: [],
  };
  assert.equal(snapshot.interrupted, true);
});

// ---------------------------------------------------------------------------
// V1 三日 ProductEval 冻结契约（第四路）：四路由 + 六态 + 拓扑枚举的类型级断言。
// 本用例不发起网络请求——任何字段漂移都会先在 tsc 编译期失败。
// ---------------------------------------------------------------------------

test("ProductEval 冻结契约（创建/列表/详情/取消 + 六态 + workswarm≡multi 口径）", () => {
  type CreateBody = NonNullable<
    operations["createProductEvalRun"]["requestBody"]
  >["content"]["application/json"];
  type Accepted = operations["createProductEvalRun"]["responses"][202]["content"]["application/json"];
  type RunSummary = components["schemas"]["ProductEvalRunSummary"];
  type Detail = operations["getProductEvalRun"]["responses"][200]["content"]["application/json"];
  type Cancelled = operations["cancelProductEvalRun"]["responses"][200]["content"]["application/json"];

  // 冻结请求体：suite 仅 v1；execution reference|live；modes 单拓扑子集；repetitions 1..=20。
  const body: CreateBody = {
    suite: "v1",
    execution: "reference",
    modes: ["single", "workswarm"],
    repetitions: 1,
    category: null,
    only: null,
  };
  assert.equal(body.suite, "v1");
  assert.equal(body.execution, "reference");
  assert.deepEqual(body.modes, ["single", "workswarm"]);

  // 202 受理：eval-… run_id + queued。
  const accepted: Accepted = { run_id: "eval-abc123", status: "queued" };
  assert.match(accepted.run_id, /^eval-/);
  assert.equal(accepted.status, "queued");

  // 运行摘要：六态 + 进度 {done,total}；modes 保留请求字面量 single/workswarm。
  const summary: RunSummary = {
    run_id: accepted.run_id,
    suite: "v1",
    execution: "reference",
    modes: ["single", "workswarm"],
    repetitions: 1,
    status: "completed",
    created_at: "2026-08-27T09:00:00Z",
    planned_total: 4,
    progress: { done: 4, total: 4 },
  };
  assert.equal(summary.status, "completed");
  assert.equal(summary.progress.done, summary.progress.total);

  // 详情：summary 基底 + report（ProductEvalReport 原样，可为 null）。
  const detail: Detail = {
    ...summary,
    report: {
      schema_version: 1,
      suite_name: "v1-r1-product-suite",
      suite_hash: "sha256:deadbeef",
      execution: "reference",
      model: null,
      generated_at: "2026-08-27T09:01:00Z",
      runs: [
        {
          key: { case_id: "document-draft-note", agent_mode: "multi", repetition: 1 },
          category: "document",
          status: "passed",
          wall_ms: 12,
          model_calls: 0,
          tool_calls: null,
          prompt_tokens: null,
          completion_tokens: null,
          total_tokens: null,
          cost_usd: null,
          failed_steps: [],
          retries: 0,
          cancellations: 0,
          artifact_refs: ["artifacts/note.md"],
          tool_log: [],
          model: null,
          started_at: "2026-08-27T09:00:30Z",
          finished_at: "2026-08-27T09:00:31Z",
          error: null,
        },
      ],
      pending: [],
      metrics: {
        runs_total: 4,
        passed: 2,
        failed: 2,
        errors: 0,
        timeouts: 0,
        cancelled: 0,
        success_rate: 0.5,
        mean_wall_ms: 12,
        total_model_calls: 0,
        total_tool_calls: null,
        total_tokens: null,
        estimated_cost_usd: null,
      },
      per_case: [
        {
          case_id: "document-draft-note",
          category: "document",
          agent_mode: "multi",
          runs_total: 1,
          passed: 1,
          success_rate: 1,
          mean_wall_ms: 12,
          mean_model_calls: 0,
          mean_tool_calls: null,
          total_tokens: null,
        },
      ],
    },
  };
  assert.equal(detail.report?.metrics.runs_total, 4);
  assert.deepEqual(detail.report?.runs[0].key, {
    case_id: "document-draft-note",
    agent_mode: "multi",
    repetition: 1,
  });
  const detailWithoutReport: Detail = { ...summary, report: null };
  assert.equal(detailWithoutReport.report, null);

  // 取消：幂等返回六态之一。
  const cancelled: Cancelled = { run_id: accepted.run_id, status: "cancelled" };
  assert.equal(cancelled.status, "cancelled");
  const completedNoOp: Cancelled = { run_id: accepted.run_id, status: "completed" };
  assert.equal(completedNoOp.status, "completed");

  // workswarm ≡ multi：结果 wire 的 agent_mode 用核心小写词（single/multi）。
  type ReportRun = NonNullable<Detail["report"]>["runs"][number];
  const multiMode: ReportRun["key"]["agent_mode"] = "multi";
  assert.equal(multiMode, "multi");
  const singleMode: ReportRun["key"]["agent_mode"] = "single";
  assert.equal(singleMode, "single");
});

// ---------------------------------------------------------------------------
// V1 四期 Artifact 评审闭环冻结契约（第三路）：review/history 路由 +
// 不可变评审记录 + 版本链 + approved head 的类型级断言。
// 本用例不发起网络请求——任何字段漂移都会先在 tsc 编译期失败。
// ---------------------------------------------------------------------------

test("Artifact 评审冻结契约（三决定 + 幂等重放 + 版本链 + approved head）", () => {
  type ReviewBody = NonNullable<
    operations["artifactSubmitReview"]["requestBody"]
  >["content"]["application/json"];
  type ReviewCreated =
    operations["artifactSubmitReview"]["responses"][201]["content"]["application/json"];
  type ReviewReplay =
    operations["artifactSubmitReview"]["responses"][200]["content"]["application/json"];
  type History =
    operations["artifactReviewHistory"]["responses"][200]["content"]["application/json"];
  type ReviewRecord = components["schemas"]["ArtifactReviewRecord"];

  // 请求体：decision 三枚举 + expected_version 乐观并发 + idempotency_key 幂等。
  const approveBody: ReviewBody = {
    team_id: "team-1",
    decision: "approve",
    reviewer: "critic",
    comment: "结构和证据通过",
    expected_version: 2,
    idempotency_key: "idem-1",
  };
  assert.equal(approveBody.decision, "approve");
  const requestChanges: ReviewBody["decision"] = "request_changes";
  const reject: ReviewBody["decision"] = "reject";
  assert.equal(requestChanges, "request_changes");
  assert.equal(reject, "reject");

  // 不可变评审记录：决策/取证锚点/幂等键齐备。
  const record: ReviewRecord = {
    review_id: "rev-0a1b2c3d",
    artifact_id: "team-1:builder:v2",
    artifact_version: 2,
    team_id: "team-1",
    decision: "approve",
    reviewer: "critic",
    comment: "结构和证据通过",
    idempotency_key: "idem-1",
    content_ref: "cas://sha256:abc",
    created_at: "2026-08-28T10:00:00Z",
  };
  assert.equal(record.artifact_version, 2);

  // 201 首次提交：replayed=false + approved head 指向被批准版本。
  const created: ReviewCreated = {
    replayed: false,
    review: record,
    artifact: {},
    approved_head: {
      artifact_id: "team-1:builder:v2",
      kind: "document",
      version: 2,
      review_state: "approved",
    },
  };
  assert.equal(created.replayed, false);
  assert.equal(created.approved_head?.artifact_id, "team-1:builder:v2");

  // 200 幂等重放：零副作用（同记录 + head 可为 null）。
  const replay: ReviewReplay = {
    replayed: true,
    review: record,
    artifact: {},
    approved_head: null,
  };
  assert.equal(replay.replayed, true);

  // history：评审历史升序 + 版本链两端 + 五态枚举。
  const history: History = {
    artifact_id: "team-1:builder:v2",
    kind: "document",
    version: 2,
    producer: "m-builder",
    review_state: "approved",
    supersedes_artifact_id: "team-1:builder:v1",
    superseded_by: null,
    reviews: [record],
    approved_head: null,
  };
  assert.equal(history.supersedes_artifact_id, "team-1:builder:v1");
  assert.equal(history.superseded_by, null);
  const states: History["review_state"][] = [
    "draft",
    "pending_review",
    "approved",
    "rejected",
    "superseded",
  ];
  assert.equal(states.length, 5);
});

// ---------------------------------------------------------------------------
// 五期冻结契约（第二/三路实现，第四路接线）：返工 / 交付物 / 团队指标 / 脱敏诊断。
// 类型级断言——字段漂移先在 tsc 编译期失败。
// ---------------------------------------------------------------------------

test("五期契约（返工 + 交付物 + 指标 + 诊断）", () => {
  // POST /artifacts/{id}/rework：同评审幂等（200 replayed / 201 created）。
  type ReworkBody = NonNullable<
    operations["artifactSubmitRework"]["requestBody"]
  >["content"]["application/json"];
  const rework: ReworkBody = {
    team_id: "team-1",
    review_id: "rev-1",
    instruction: "修正 JSON scope 字段并保持 schema 不变",
  };
  assert.equal(rework.review_id, "rev-1");
  // 201 created / 200 idempotent replay 响应键在场（类型级存在性断言：缺失时解析为 never）。
  type Present<R> = [R] extends [never] ? false : true;
  type ReworkCreatedPresent = Present<operations["artifactSubmitRework"]["responses"][201]>;
  type ReworkReplayPresent = Present<operations["artifactSubmitRework"]["responses"][200]>;
  const reworkCreatedPresent: ReworkCreatedPresent = true;
  const reworkReplayPresent: ReworkReplayPresent = true;
  assert.equal(reworkCreatedPresent, true);
  assert.equal(reworkReplayPresent, true);

  // GET /projects/{id}/deliverables：approved/pending/rejected_or_superseded 三桶。
  type Deliverables =
    operations["projectDeliverables"]["responses"][200]["content"]["application/json"];
  const dlv: Deliverables = {
    project_id: "proj-team-1",
    approved: [{ artifact_id: "team-1:critic:v2", review_state: "approved" }],
    pending: [{ artifact_id: "team-1:critic:v1", review_state: "pending_review" }],
    rejected_or_superseded: [],
  };
  assert.equal(dlv.approved.length, 1);
  assert.equal(dlv.pending.length, 1);

  // GET /teams/{id}/metrics：workers + summary。
  type Metrics =
    operations["teamMetrics"]["responses"][200]["content"]["application/json"];
  const metrics: Metrics = {
    team_id: "team-1",
    workers: [
      {
        worker: "m-critic",
        role: "critic",
        duration_ms: 1200,
        model_calls: 3,
        terminal: "Succeeded",
      },
    ],
    summary: { wall_clock_ms: 5000, total_model_calls: 3 },
  };
  assert.equal(metrics.workers?.[0]?.model_calls, 3);

  // GET /teams/{id}/diagnostic：脱敏 JSON（additionalProperties 开放对象；200 在场为类型级断言）。
  type DiagnosticResponse =
    operations["teamDiagnostic"]["responses"][200]["content"]["application/json"];
  const diag: DiagnosticResponse = { team_id: "team-1", redacted: true };
  assert.equal(diag.team_id, "team-1");
});

// ---------------------------------------------------------------------------
// 六期冻结契约（第二/三路实现，第四路接线）：工作区绑定 / 模板目录 / POST /teams workspace。
// 类型级断言——字段漂移先在 tsc 编译期失败。
// ---------------------------------------------------------------------------

test("六期契约（工作区绑定 + 模板目录 + teams workspace 字段）", () => {
  type Present<R> = [R] extends [never] ? false : true;

  // POST /teams 请求体 additive workspace（六期冻结：root 必填、read_only 缺省 true）。
  type CreateTeamBody = NonNullable<
    operations["workswarmCreateTeam"]["requestBody"]
  >["content"]["application/json"];
  const createBody: CreateTeamBody = {
    objective: "修复登录超时并补充回归测试",
    strategy: "auto",
    workspace: {
      root: "T:\\demo",
      read_only: true,
      write_allowed_paths: ["src/", "tests/"],
      tree_depth: 2,
    },
  };
  assert.equal(createBody.workspace?.root, "T:\\demo");
  assert.equal(createBody.workspace?.read_only, true);

  // PUT /projects/{id}/workspace 绑定 → 200 绑定回显。
  type BindBody = NonNullable<
    operations["projectBindWorkspace"]["requestBody"]
  >["content"]["application/json"];
  const bind: BindBody = { root: "T:\\demo", read_only: true, tree_depth: 2 };
  assert.equal(bind.root, "T:\\demo");
  type BindOk = Present<operations["projectBindWorkspace"]["responses"][200]>;
  const bindOk: BindOk = true;
  assert.equal(bindOk, true);

  // GET /projects/{id}/workspace：body = {workspace:{...}}（二路实现包装形状）。
  type Workspace = operations["projectGetWorkspace"]["responses"][200]["content"]["application/json"];
  const ws: Workspace = {
    workspace: {
      project_id: "proj-team-1",
      team_id: "team-1",
      root: "T:\\demo",
      read_only: true,
      write_allowed_paths: ["src/"],
      tree_depth: 2,
    },
  };
  assert.equal(ws.workspace.root, "T:\\demo");
  assert.equal(ws.workspace.read_only, true);

  // GET /projects/{id}/workspace/tree：扁平 entries（root/depth/truncated 附加）。
  type Tree =
    operations["projectWorkspaceTree"]["responses"][200]["content"]["application/json"];
  const tree: Tree = {
    root: "T:\\demo",
    depth: 2,
    truncated: false,
    entries: [
      { path: "src", type: "dir" },
      { path: "src/main.rs", type: "file", size: 1024 },
    ],
  };
  assert.equal(tree.entries[0]?.type, "dir");

  // GET /projects/{id}/workspace/git-status：porcelain 行字符串数组（非 Git 仓库为空数组）。
  type Git =
    operations["projectWorkspaceGitStatus"]["responses"][200]["content"]["application/json"];
  const git: Git = {
    root: "T:\\demo",
    git: true,
    porcelain: " M a.rs\n?? b.txt",
    entries: [" M a.rs", "?? b.txt"],
  };
  assert.equal(git.entries.length, 2);

  // GET /teams/templates/catalog：候选区（installed 标记）。
  type Catalog =
    operations["teamTemplateCatalog"]["responses"][200]["content"]["application/json"];
  const cat: Catalog = {
    catalog: [
      { template_id: "code-change-v1", version: 1, installed: true, builtin: true },
      { template_id: "research-brief-v1", version: 1, installed: false, builtin: true },
    ],
  };
  assert.equal(cat.catalog.length, 2);

  // POST /teams/templates/catalog/{id}/install：幂等（replayed）。
  type InstallOk = Present<
    operations["teamTemplateCatalogInstall"]["responses"][200]
  >;
  const installOk: InstallOk = true;
  assert.equal(installOk, true);

  // 404 语义在场：未知 project / 未知模板 id。
  type WsNotFound = Present<operations["projectGetWorkspace"]["responses"][404]>;
  type InstallNotFound = Present<
    operations["teamTemplateCatalogInstall"]["responses"][404]
  >;
  const wsNotFound: WsNotFound = true;
  const installNotFound: InstallNotFound = true;
  assert.equal(wsNotFound, true);
  assert.equal(installNotFound, true);
});

// ---------------------------------------------------------------------------
// 七期冻结契约（二/三路 wire，第四路接线）：/teams/{id} additive
// worker_profiles / write_lease / changes + /projects/{id}/artifacts additive
// validation / sha256 / size_bytes / evidence_refs + 3 条新路由
//（artifactContent / artifactMetadata / projectDeliveryManifest）。
// 类型级断言——二/三路 wire 形状漂移先在 tsc 编译期失败。
// 接线前 3 条路由 404（未注册），UI 容错读取；字段均为 nullable/可缺省。
// ---------------------------------------------------------------------------

test("七期契约（teams/{id} 新字段 + artifacts 新字段 + 3 条新路由）", () => {
  type Present<R> = [R] extends [never] ? false : true;

  // GET /teams/{id} 200：additive worker_profiles / write_lease / changes
  //（不在 required 内，旧记录缺字段仍可解析；也可能位于 team 对象内，UI 双路径容错）。
  type Team200 =
    operations["workswarmGetTeam"]["responses"][200]["content"]["application/json"];
  const team: Team200 = {
    team: {},
    interrupted: false,
    tasks: {},
    audit_tail: [],
    worker_profiles: [
      {
        role: "implementer",
        visible_tools: ["read", "edit", "run_command"],
        read_only: false,
        can_use_browser: false,
        can_run_command: true,
        write_allowed_paths: ["src/"],
        max_turns: 8,
      },
      {
        role: "researcher",
        visible_tools: ["read", "browser"],
        read_only: true,
        can_use_browser: true,
        can_run_command: false,
      },
    ],
    write_lease: {
      holder_role: "implementer",
      holder_step_id: "step-3",
      acquired_at_ms: 1720000000000,
      released_at_ms: null,
    },
    changes: [
      {
        path: "src/main.rs",
        state: "modified",
        added_lines: 12,
        deleted_lines: 3,
        diff: "@@ -1,4 +1,14 @@",
      },
      { path: "src/legacy.rs", state: "deleted" },
    ],
  };
  assert.equal(team.worker_profiles?.[0]?.role, "implementer");
  assert.equal(team.worker_profiles?.[1]?.can_use_browser, true);
  assert.equal(team.write_lease?.holder_role, "implementer");
  assert.equal(team.changes?.[0]?.state, "modified");
  assert.equal(team.changes?.[1]?.state, "deleted");
  // 旧记录形状：3 个新字段整体缺省（required 未变）必须仍合法。
  const teamLegacy: Team200 = {
    team: {},
    interrupted: false,
    tasks: {},
    audit_tail: [],
  };
  assert.equal(teamLegacy.worker_profiles, undefined);
  assert.equal(teamLegacy.write_lease, undefined);
  assert.equal(teamLegacy.changes, undefined);

  // GET /projects/{id}/artifacts 200：items additive
  // validation / sha256 / size_bytes / evidence_refs（全部可缺省）。
  type ArtList =
    operations["workswarmListArtifacts"]["responses"][200]["content"]["application/json"];
  const arts: ArtList = {
    project_id: "proj-team-1",
    artifacts: [
      {
        artifact_id: "art-1",
        kind: "report",
        review_state: "pending",
        version: 2,
        sha256: "ab12",
        size_bytes: 4096,
        evidence_refs: ["evidence://audit/1"],
        validation: { format: "markdown", valid: true },
      },
    ],
  };
  assert.equal(arts.artifacts[0]?.sha256, "ab12");
  assert.equal(arts.artifacts[0]?.validation?.valid, true);
  assert.equal(arts.artifacts[0]?.evidence_refs?.length, 1);

  // GET /artifacts/{id}/content：下载/预览载荷（content = 原始文本，非 JSON 编码）。
  type ArtContent =
    operations["artifactContent"]["responses"][200]["content"]["application/json"];
  const content: ArtContent = {
    artifact_id: "art-1",
    format: "markdown",
    sha256: "ab12",
    size_bytes: 4096,
    content: "# 报告\n正文",
  };
  assert.equal(content.content.startsWith("# "), true);
  type ArtContent404 = Present<operations["artifactContent"]["responses"][404]>;
  const artContent404: ArtContent404 = true;
  assert.equal(artContent404, true);

  // GET /artifacts/{id}/metadata：元数据 + validation + evidence_refs + handoff（可空）。
  type ArtMeta =
    operations["artifactMetadata"]["responses"][200]["content"]["application/json"];
  const meta: ArtMeta = {
    artifact_id: "art-1",
    team_id: "team-1",
    kind: "report",
    format: "markdown",
    version: 2,
    sha256: "ab12",
    size_bytes: 4096,
    validation: { format: "markdown", valid: true, reason: null },
    evidence_refs: ["evidence://audit/1"],
    handoff: null,
  };
  assert.equal(meta.handoff, null);
  type ArtMeta404 = Present<operations["artifactMetadata"]["responses"][404]>;
  const artMeta404: ArtMeta404 = true;
  assert.equal(artMeta404, true);

  // GET /projects/{id}/delivery-manifest：交付清单（approved 概览 + content_url 相对路径）。
  type Manifest =
    operations["projectDeliveryManifest"]["responses"][200]["content"]["application/json"];
  const manifest: Manifest = {
    project_id: "proj-team-1",
    generated_at: "2026-07-28T00:00:00Z",
    manifest: [
      {
        artifact_id: "art-1",
        kind: "report",
        format: "markdown",
        version: 2,
        sha256: "ab12",
        size_bytes: 4096,
        approved: true,
        content_url: "/artifacts/art-1/content",
      },
    ],
  };
  assert.equal(manifest.manifest[0]?.content_url, "/artifacts/art-1/content");
  type Manifest404 = Present<
    operations["projectDeliveryManifest"]["responses"][404]
  >;
  const manifest404: Manifest404 = true;
  assert.equal(manifest404, true);
});

test("七期契约（二路交接：GET /projects/{id}/workspace/changes 变更追踪）", () => {
  type Present<R> = [R] extends [never] ? false : true;

  // 路径面：operationId 冻结为 projectWorkspaceChanges，GET 方法挂在项目工作区路径下。
  type ChangesRoute = paths["/projects/{id}/workspace/changes"]["get"];
  type SameSource = [operations["projectWorkspaceChanges"]] extends [
    ChangesRoute,
  ]
    ? true
    : false;
  const sameSource: SameSource = true;
  assert.equal(sameSource, true, "路径 get 与 operations 同源");

  // 200：六个顶层字段全部 required（team_id/git/changed_files/diff_summary/has_violation/records）。
  type Changes200 =
    operations["projectWorkspaceChanges"]["responses"][200]["content"]["application/json"];
  const payload: Changes200 = {
    team_id: "team-1",
    git: true,
    changed_files: ["src/calc.rs", "out/fix-report.md"],
    diff_summary: " src/calc.rs | 2 +-",
    has_violation: false,
    records: [
      {
        role: "implementer",
        step: "s-2",
        at: 1753680000000,
        git: true,
        changed_files: ["src/calc.rs"],
        diff_summary: " src/calc.rs | 2 +-",
        diff_ref: "team-1-changes/s-2.patch",
        violation: null,
      },
      {
        role: "finalizer",
        step: "s-3",
        at: 1753680001000,
        git: false,
        changed_files: [],
        diff_summary: "",
        diff_ref: null,
        violation: "越界写入 /etc/passwd（不在允许路径）",
      },
    ],
  };
  assert.equal(payload.records.length, 2);
  assert.equal(payload.records[1]?.violation, "越界写入 /etc/passwd（不在允许路径）");
  // 记录元素为开放对象（additionalProperties）：未知字段可读不报错。
  const open: Record<string, unknown> = payload.records[0] ?? {};
  assert.equal(typeof open["role"], "string");

  // 容错语义：字段全部可选（additionalProperties: true 的 items），缺省读取不报类型错。
  const sparse: Changes200 = {
    team_id: "team-2",
    git: false,
    changed_files: [],
    diff_summary: "",
    has_violation: false,
    records: [],
  };
  assert.deepEqual(sparse.changed_files, []);

  // 404：未知项目/团队。
  type Changes404 = Present<
    operations["projectWorkspaceChanges"]["responses"][404]
  >;
  const changes404: Changes404 = true;
  assert.equal(changes404, true);
});

// ============================================================================
// 八期冻结契约（二/三路 wire，第四路接线）：ChangeSet 审批闭环 + Human Inbox
// ============================================================================

test("八期契约（ChangeSet 五路由：列表/详情/accept/reject/revert）", () => {
  // 路径面：operationId 与 operations 同源。
  type ListRoute = paths["/teams/{id}/change-sets"]["get"];
  const listSameSource: [operations["teamChangeSets"]] extends [ListRoute] ? true : false = true;
  assert.equal(listSameSource, true);

  // 200：{team_id, change_sets[]}，change_set 元素状态枚举冻结。
  type CsList = operations["teamChangeSets"]["responses"][200]["content"]["application/json"];
  const payload: CsList = {
    team_id: "team-1",
    change_sets: [
      {
        change_set_id: "cs-1",
        team_id: "team-1",
        step_id: "s-impl",
        role: "implementer",
        changed_files: ["src/calc.rs"],
        diff_ref: null,
        status: "pending_review",
        created_at: "2026-08-30T04:00:00Z",
        resolved_at: null,
      },
    ],
  };
  assert.equal(payload.change_sets[0]?.status, "pending_review");
  assert.deepEqual(payload.change_sets[0]?.changed_files, ["src/calc.rs"]);

  // 动作端点 200：{change_set, replayed}（幂等重放零副作用）。
  type CsAccept = operations["changeSetAccept"]["responses"][200]["content"]["application/json"];
  const accepted: CsAccept = { change_set: payload.change_sets[0] ?? {}, replayed: true };
  assert.equal(accepted.replayed, true);

  // 409：文件被用户再次修改 / 已终态跨动作；404：未知 ChangeSet。
  type CsAccept409 = [operations["changeSetAccept"]["responses"][409]] extends [never] ? false : true;
  type CsAccept404 = [operations["changeSetAccept"]["responses"][404]] extends [never] ? false : true;
  const csAccept409: CsAccept409 = true;
  const csAccept404: CsAccept404 = true;
  assert.equal(csAccept409, true);
  assert.equal(csAccept404, true);
});

test("八期契约（Human Inbox 五路由：列表/详情/claim/release/resolve）", () => {
  type ListRoute = paths["/human/inbox"]["get"];
  const listSameSource: [operations["humanInboxList"]] extends [ListRoute] ? true : false = true;
  assert.equal(listSameSource, true);

  // 200：{items, counts}；item.kind 四类枚举冻结；item.status 三态。
  type InboxList = operations["humanInboxList"]["responses"][200]["content"]["application/json"];
  const payload: InboxList = {
    items: [
      {
        item_id: "i1",
        kind: "artifact_review",
        status: "claimed",
        assignee: "本地用户",
        team_id: "team-1",
        project_id: "proj-1",
        target_id: "team-1:builder:v1",
        summary: "待评审产物",
        created_at: "2026-08-30T04:00:00Z",
        claimed_at: "2026-08-30T04:05:00Z",
        resolved_at: null,
        detail: {},
      },
    ],
    counts: { artifact_review: 1 },
  };
  assert.equal(payload.items[0]?.kind, "artifact_review");
  assert.equal(payload.items[0]?.status, "claimed");

  // resolve 200：{resolved, replayed, item_id, kind, result}（按 kind 分派到既有领域端点）。
  type Resolve200 = operations["humanInboxResolve"]["responses"][200]["content"]["application/json"];
  const resolved: Resolve200 = {
    resolved: true,
    replayed: false,
    item_id: "i1",
    kind: "artifact_review",
    result: {},
  };
  assert.equal(resolved.resolved, true);
  assert.equal(resolved.replayed, false);

  // claim 请求体：{user} 必填；409 已被他人领取；404 未知待办。
  type ClaimBody = NonNullable<operations["humanInboxClaim"]["requestBody"]>["content"]["application/json"];
  const claim: ClaimBody = { user: "本地用户" };
  assert.equal(claim.user, "本地用户");
  type Claim409 = [operations["humanInboxClaim"]["responses"][409]] extends [never] ? false : true;
  type Claim404 = [operations["humanInboxClaim"]["responses"][404]] extends [never] ? false : true;
  const claim409: Claim409 = true;
  const claim404: Claim404 = true;
  assert.equal(claim409, true);
  assert.equal(claim404, true);

  // resolve 幂等重放 200；409 未领取/领域冲突。
  type ResolveReplay = operations["humanInboxResolve"]["responses"][200]["content"]["application/json"];
  const replay: ResolveReplay = {
    resolved: true,
    replayed: true,
    item_id: "i1",
    kind: "artifact_review",
    result: {},
  };
  assert.equal(replay.replayed, true);
});

// ---------------------------------------------------------------------------
// 十期冻结契约（一路）：/health additive build 字段——版本/构建信息单源。
// 类型级断言——字段漂移先在 tsc 编译期失败。
// ---------------------------------------------------------------------------

test("十期契约（/health additive build：BuildInfo 三字段 + 旧形状仍合法）", () => {
  type Health200 =
    operations["health"]["responses"][200]["content"]["application/json"];
  type Build = NonNullable<NonNullable<Health200["build"]>>;

  // 新形状：build-info.json 存在时 build 三字段齐备（commit/dirty/built_at）。
  const withBuild: Health200 = {
    healthy: true,
    version: "0.1.0",
    api_version: "0.7",
    auto_approve: false,
    build: { commit: "8412021", dirty: true, built_at: "2026-08-30T04:00:00Z" },
  };
  assert.equal(withBuild.build?.commit, "8412021");
  assert.equal(withBuild.build?.dirty, true);
  assert.equal(typeof withBuild.build?.built_at, "string");

  // 旧形状：build 缺省（skip_serializing_if）仍合法——required 未变。
  const legacy: Health200 = {
    healthy: true,
    version: "0.1.0",
    api_version: "0.7",
    auto_approve: true,
  };
  assert.equal(legacy.build, undefined);

  // BuildInfo 形状面：dirty 必为布尔，commit/built_at 必为字符串。
  const build: Build = { commit: "HEAD", dirty: false, built_at: "" };
  assert.equal(build.dirty, false);
});

// ---------------------------------------------------------------------------
// R3（§8.1）契约：/diagnostics/requests 六字段白名单 ledger（验收取证唯一事实源）。
// ---------------------------------------------------------------------------

test("R3 契约（/diagnostics/requests：ledger 六字段白名单 + 三桶聚合）", () => {
  type Report200 =
    operations["diagnosticsRequests"]["responses"][200]["content"]["application/json"];
  type Record200 = Report200["records"][number];
  type Aggregates = Report200["aggregates"];

  // 恰好六字段：多余键（如 path/body/authorization）在类型层即不可构造。
  const record: Record200 = {
    method: "GET",
    route_template: "/session/{id}/diff",
    started_at: "2026-09-17T08:41:33.120Z",
    duration_ms: 7,
    status: 404,
    source: "web",
  };
  assert.equal(Object.keys(record).length, 6);
  assert.ok(
    !("path" in record) &&
      !("query" in record) &&
      !("authorization" in record) &&
      !("body" in record),
    "ledger 禁止记录原始路径/查询串/凭据/请求体",
  );
  assert.ok(
    !("real_id" in record),
    "route_template 恒为模板，真实资源 id 不落 ledger",
  );

  const report: Report200 = {
    total: 3,
    returned: 3,
    cap: 512,
    aggregates: { health: 1, auth_token: 0, business: 2 } satisfies Aggregates,
    records: [record],
  };
  assert.equal(
    report.aggregates.health + report.aggregates.auth_token + report.aggregates.business,
    report.returned,
    "三桶必须恰好覆盖窗口内记录",
  );
});

test("expression uses only the authenticated bounded route and propagates cancellation", async () => {
  const originalFetch = globalThis.fetch;
  const controller = new AbortController();
  const signal = controller.signal;
  globalThis.fetch = async (input, init) => {
    assert.equal(String(input), "http://127.0.0.1:4096/expression");
    assert.equal(init?.signal, signal);
    assert.equal(new Headers(init?.headers).get("authorization"), "Bearer fixture");
    assert.deepEqual(JSON.parse(String(init?.body)), { prompt: "改写你好" });
    return new Response(JSON.stringify({ text: "您好" }), { status: 200 });
  };
  try {
    const client = createClient({ baseUrl: "http://127.0.0.1:4096/", headers: { authorization: "Bearer fixture" } });
    assert.deepEqual(await client.expression({ prompt: "改写你好" }, signal), { text: "您好" });
    globalThis.fetch = async () => new Response(JSON.stringify({ text: "中".repeat(4097) }));
    await assert.rejects(client.expression({ prompt: "改写你好" }), /Invalid expression/);
  } finally { globalThis.fetch = originalFetch; }
});


test("expression does not retry a source rejection", async () => {
  const originalFetch = globalThis.fetch;
  let calls = 0;
  globalThis.fetch = async () => {
    calls++;
    return new Response("expression/invalid_source", { status: 403 });
  };
  try {
    await assert.rejects(createClient({ baseUrl: "http://127.0.0.1:4096" }).expression({ prompt: "synthetic" }), /403/);
    assert.equal(calls, 1);
  } finally { globalThis.fetch = originalFetch; }
});
