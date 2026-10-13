# OwO 团队协作只读审查（2026-10-13）

结论：团队协作已有真实实现与可通过的基础流程，但审批、撤销、人工恢复、替换和返工仍有缺陷，不能判定达到可靠协作闭环预期。

当前 branch=agent-frame，HEAD=79fbc6f671915054796619505ca280743dfd10b1。现有 EXE 自报同提交、dirty=true；编译期时间 2026-10-12T15:22:02Z。未重新编译 SDK。仅同提交与 dirty 标记不能证明二进制和当前工作树逐字一致；实测结论均用当前源码调用链交叉核对。

## 范围与验证口径

- 检查 Team/WorkSwarm 建队、策略与任务图、角色画像、上下文、调度、预算、工具/写租约、人工接力、steer/cancel/retry/replace、恢复、Artifact/评审/返工/交付、Human Inbox、模板、指标/SSE、CLI/桌面消费，以及团队技能包共享。
- API 测试使用隔离 Daemon、echo/sleep Worker、合成数据与故障注入；OWO_CLOUD_ENABLED=false，未调用付费模型、未下载 Ollama 或依赖。
- 正常 echo 接力、正常人工唤醒、sleep 取消、Artifact 正文哈希、事件 JSON 快照、指标/诊断/交付清单、共享事实发布及 revision 冲突、模板采纳均取得相应预期结果。只读路由 200 仅证明此次可访问，不证明所有内容与故障分支正确。
- 7 个 Node 测试文件累计 126 项通过，涉及团队面板、挂载生命周期、交付文件、Artifact 评审与工作区路由。未启动真实 Electron GUI。
- 对原样提取的生产 Git 解析函数单独编译运行：3 项中 ASCII 控制通过、中文引用及箭头文件名失败；这不是 Rust workspace 测试通过。测试 EXE/PDB 仅生成在 env，未安装软件。
- 完整 Rust SDK 原生重链未运行：检查时构建卷约 8.85 GiB，可用内存约 12.34 GiB；仓库完整/原生重链要求至少 20 GiB 磁盘。没有降低门槛或清理旧产物。
- 真实 LLM 生成的动态任务图、真实并行命令、预算用量/计费与大型并发、Electron 可见交互、全部 Rust 契约测试未验收；静态判定单列，不宣称所有功能或错误路径均已验证。

## 缺陷、根因与解决方案

### F01 [P1] 撤销可写出绑定工作区
- 证据等级：已复现：HTTP/Junction 故障注入
- 缺陷/是否达到预期：未达到该条件下的预期。构造合法相对路径、让目录成为指向工作区外的 Junction；reject 返回 200，外侧测试文件被恢复写回。外侧文件仍位于本次 env 测试目录。
- 根因：restore_change_set 的 file_hash 和写回/删除直接 root.join(relative)，未重新解析真实目标并验证根/白名单。
- 解决方案：对恢复路径逐组件验证真实父目录、拒绝重解析点和越界；校验后至写入期间防止目标再次被替换。
- 源码：[源码位置](D:/src/OwO/agent-sdk/crates/owo-agent-extensions/src/change_set.rs:401)

### F02 [P1] 接受与拒绝竞争造成文件、状态不一致
- 证据等级：已复现：HTTP 并发/合成 ChangeSet
- 缺陷/是否达到预期：未达到该条件下的预期。accept=200，reject=409，最终 accepted，但 1200/1200 文件已恢复为基线。
- 根因：decide 在锁外读决定，只有 reject/revert 的文件恢复持有工作区租约；恢复后释放租约，再在另一锁内 apply_decision；accept 不参与同一事务。
- 解决方案：同一 ChangeSet 的验证、动作占位、文件恢复、决定提交统一串行化；锁内重新读取状态；全部入口与 Inbox 共用该执行器；增加崩溃恢复日志。
- 源码：[源码位置](D:/src/OwO/agent-sdk/crates/owo-agent-server/src/change_set_api.rs:219)

### F03 [P1] 直接评审绕过 ChangeSet 批准门
- 证据等级：已复现：HTTP/合成 pending 状态
- 缺陷/是否达到预期：未达到该条件下的预期。GET change-sets 显示 approval_blocked=true，code:v3 却可经直接 review 接口成为 approved_head。初次对 v1/v2 的 409 是 superseded 拦截，并非 ChangeSet 门。未证明 TeamRun 的 DeliveryGate 被绕过。
- 根因：直接 submit_review/apply_artifact_review 不检查 ChangeSet 门；Inbox 分派有该检查，两个入口语义不一致。
- 解决方案：将批准前置门统一放到不可绕过的领域服务，绑定团队、当前 attempt/产物/文件哈希并检查 pending/conflicted；各 UI/API/Inbox 都调用。
- 源码：[源码位置](D:/src/OwO/agent-sdk/crates/owo-agent-server/src/artifact_review_api.rs:157)

### F04 [P1] 评审可写入错误团队归属
- 证据等级：已复现：HTTP
- 缺陷/是否达到预期：未达到该条件下的预期。用团队 B 的 team_id 评审团队 A 的产物返回 201，保存的评审记录归 B。
- 根因：只检查请求团队存在并取其 human_policy，未比较 artifact.team_id 与请求 team_id。
- 解决方案：从产物解析实际团队与项目并验证请求归属；授权只能取实际团队；返工也验证同一绑定。
- 源码：[源码位置](D:/src/OwO/agent-sdk/crates/owo-agent-workswarm/src/project_space_store/review.rs:95)

### F05 [P1] 生产者可用空格绕过自批限制
- 证据等级：已复现：HTTP
- 缺陷/是否达到预期：未达到该条件下的预期。reviewer=m-researcher 返回 403；reviewer=" m-researcher " 返回 201，最终保存的 reviewer 又是 m-researcher。
- 根因：授权比较使用原始输入；保存记录时才 trim。
- 解决方案：先统一规范化和验证身份，再判定授权；尽量使用宿主解析的 actor，禁止靠任意正文字符串授予独立评审资格。
- 源码：[源码位置](D:/src/OwO/agent-sdk/crates/owo-agent-workswarm/src/project_space_store/review.rs:128)

### F06 [P1] 人工结果可在上游未完成时提交
- 证据等级：已复现：HTTP
- 缺陷/是否达到预期：未达到该条件下的预期。上游 sleep 为 Running，human-result 仍返回 200，并把人工任务置 Succeeded。
- 根因：record_human_result 仅检查人节点和非终态，没有 step_deps_satisfied/当前等待节点校验；产物登记允许缺失上游引用。
- 解决方案：提交前在团队锁内确认节点就绪、依赖成功、当前等待身份及 attempt；迟到/重复提交按幂等键返回或拒绝。
- 源码：[源码位置](D:/src/OwO/agent-sdk/crates/owo-agent-core/src/workswarm/coord_handoff.rs:4)

### F07 [P1] 等待人工的团队无法正常恢复
- 证据等级：已复现：真实 Daemon 重启
- 缺陷/是否达到预期：未达到该条件下的预期。重启后 AwaitingHuman 不标记 interrupted；continue 返回 409；human-result 返回 200 后下游仍 Pending，团队停在 AwaitingHuman。
- 根因：中断扫描仅认 Running；Continue 未放行 AwaitingHuman；提交结果只推进进程内 watch，不确保运行循环存在。
- 解决方案：恢复识别所有可续接状态；人工结果提交后确保唯一循环存在；允许显式恢复 AwaitingHuman，保留已完成步骤，禁止静默重放写操作。
- 源码：[源码位置](D:/src/OwO/agent-sdk/crates/owo-agent-core/src/workswarm/coord_human.rs:88)

### F08 [P1] 等待人工时替换 Agent 使用旧注册表
- 证据等级：已复现：HTTP
- 缺陷/是否达到预期：未达到该条件下的预期。replace human->echo 返回 200，随后 Failed，error=worker 未注册:m-human。另测未知 new_worker 也被接受为 200。
- 根因：latch 内反复使用进入等待前的 registry；replace 只改成员和 RunMeta，不刷新该 registry，也缺少与 create 同口径的执行器校验。
- 解决方案：拓扑/绑定变化推进版本；门闩被唤醒后退出至外层重建注册表；持久化前校验执行器；失败状态保留原绑定。
- 源码：[源码位置](D:/src/OwO/agent-sdk/crates/owo-agent-server/src/workswarm_api/runtime.rs:415)

### F09 [P1] 待办领取不约束实际处理者
- 证据等级：已复现：HTTP
- 缺陷/是否达到预期：未达到该条件下的预期。alice 领取后，bob resolve 返回 200，resolved=true。
- 根因：begin_resolve 只检查状态和幂等键，没有检查调用者与 assignee；resolve/dispatch 也未补该校验。
- 解决方案：在同一 CAS 占位事务内校验领取者及 actor；开放待办需要明确的自动领取策略；禁止旁路域接口绕开需要领取的约束。
- 源码：[源码位置](D:/src/OwO/agent-sdk/crates/owo-agent-server/src/human_inbox_store.rs:371)

### F10 [P1] 处理中的人工待办重启后永久占用
- 证据等级：已复现：持久化 resolving 故障注入+重启
- 缺陷/是否达到预期：未达到该条件下的预期。模拟进程在 resolving 落盘后中断；重启 resolve 一直 409：待办正在处理中，请稍后重试。
- 根因：resolving 持久化，但加载不恢复未完成租约；无宿主代次/到期或领域结果对账。
- 解决方案：恢复时依据操作日志与领域幂等结果判断已执行/未执行/未知，再完成或释放占位；未知写操作交人工，不能直接重放。
- 源码：[源码位置](D:/src/OwO/agent-sdk/crates/owo-agent-server/src/human_inbox_store.rs:202)

### F11 [P1] 无变更自动跳过评审后被交付门拒绝
- 证据等级：已复现：携带自动评审标记的隔离角色
- 缺陷/是否达到预期：未达到该条件下的预期。无工作区修改，自动 Reviewer 被置 Succeeded/skip，最终 Team Failed：运行期跳过没有得到允许。
- 根因：调度器允许 code-change 模板或 auto_reviewer 标记跳过；交付门只允许 code-change 模板，忽略 auto_reviewer。
- 解决方案：用同一宿主策略判定可跳过性，落盘原因与证据并由交付门核验；显式必须评审不能被自动标记降级。
- 源码：[源码位置](D:/src/OwO/agent-sdk/crates/owo-agent-core/src/workswarm/coord_delivery_gate.rs:86)

### F12 [P1] Git 路径解析错误
- 证据等级：已复现：原样提取生产 Rust 函数测试
- 缺陷/是否达到预期：未达到该条件下的预期。普通 ASCII 控制通过；中文八进制引用路径与含 " -> " 的普通文件名两项失败。后者被误解析为 final.txt。
- 根因：parse_porcelain_path 仅去引号、不解码 Git C 风格引用，并把任何箭头子串都当重命名。
- 解决方案：使用 porcelain -z，按状态与 NUL 字段严格解析；统一仓库根/绑定根坐标，覆盖中文、引号、制表符、箭头及 rename 双路径。
- 源码：[源码位置](D:/src/OwO/agent-sdk/crates/owo-agent-server/src/workspace_change_tracker/git.rs:49)

### F13 [P2] 标准补丁生成始终失败
- 证据等级：已复现：实际 Git 命令
- 缺陷/是否达到预期：未达到该条件下的预期。git diff --no-index --label 返回 129：unknown option label。
- 根因：使用了 Git 不支持的 --label 参数；错误被转换成 None，进入截断差异摘要。
- 解决方案：改用受支持的差异生成参数/库；保留真实基线、正确文件标签与完整 patch；并为每个步骤使用独立临时目录。
- 源码：[源码位置](D:/src/OwO/agent-sdk/crates/owo-agent-server/src/workspace_change_tracker/tracker.rs:157)

### F14 [P2] Daemon 重启后进度序号被当旧事件
- 证据等级：已复现：实际前端函数；未 GUI 验收
- 缺陷/是否达到预期：未达到该条件下的预期。前端已记 seq=100，实际 applyProgress 收到新进程 seq=1 返回 false；后端 seq 仅存内存。
- 根因：前端 seq 单调去重跨越重启，但事件没有服务代次；重连不重置该游标。
- 解决方案：事件包含服务启动代次，与 seq 一起比较；新代次重新建立快照和游标；同代次继续拒绝倒序事件。
- 源码：[源码位置](D:/src/OwO/agent-sdk/desktop/web/panels/workswarm.panel.js:645)

### F15 [P1] 协调器首次并发初始化可产生多个恢复扫描者
- 证据等级：源码确认竞争窗口，未并发首用复现
- 缺陷/是否达到预期：未达到该条件下的预期。coordinator 检查 None 后各自创建实例；set 失败者仍用自己的实例启动 detect_interrupted。
- 根因：恢复扫描使用局部 coordinator，而其 loop_alive/run_flags/team_locks 与获胜实例不共享；可能把正在运行的团队误标为 interrupted。
- 解决方案：初始化全过程串行化；只发布一个成功实例，只对发布实例执行一次恢复扫描，扫描完成与派发顺序明确。
- 源码：[源码位置](D:/src/OwO/agent-sdk/crates/owo-agent-server/src/workswarm_api/state.rs:32)

### F16 [P1] 同一 Worker 承担多个任务时返工定位错误
- 证据等级：已复现：真实动态 TaskGraph + HTTP 返工（echo Worker）
- 缺陷/是否达到预期：未达到。动态 DAG 正常成功，同一 w1 执行 s-w1 和 s-task-b，生成 v1/v2。v2 metadata.task_id=s-task-b，但评审 review.step_id=s-w1，rework 返回 201 且 target_step=s-w1。第二个任务的返工实际定位到第一个任务。
- 根因：忽略 Artifact.task_id/attempt_id；评审记录 step_id 也从 m-role 推导 s-role，无法区分 s-task-*。
- 解决方案：以 Artifact 的精确步骤/任务/attempt 绑定为主定位并验证当前版本；producer 只能作一致性校验，不能用 first-match 定位。
- 源码：[源码位置](D:/src/OwO/agent-sdk/crates/owo-agent-server/src/artifact_rework_api.rs:206)

### F17 [P1] 变更检测范围与完整性口径错误
- 证据等级：源码确认；绑定子目录有对照运行异常
- 缺陷/是否达到预期：未达到该条件下的预期。绑定 Git 子目录时 status 返回整个仓库坐标；代码仍以绑定目录 join。另对 status 静默取前 500 行；Git 模式不做完整内容快照差集，允许写的 ignored 文件会漏报。
- 根因：未区分 repo_root/workspace_root，未携带 Git 截断标志；git=true 被当作足够完整的唯一变更检测。
- 解决方案：限定查询范围并转换路径坐标；截断/快照失败显式 unverified/fail-closed；合法 ignored 文件使用内容差集补充，不能把未知当无改动。
- 源码：[源码位置](D:/src/OwO/agent-sdk/crates/owo-agent-server/src/workspace_change_tracker/git.rs:85)

### F18 [P2] 团队审计和技能包共享审计缺少持久化
- 证据等级：源码确认，未故障注入审计存储
- 缺陷/是否达到预期：未达到该条件下的预期。TeamCoordinator 使用 AuditLog::default 的内存 Vec，team_api 使用全局内存 AUDIT；重启后完整审计尾迹不可恢复。ProjectSpace 有部分活动记录和指标文件，但不能替代完整审计。
- 根因：审计数据只在进程中，多个领域动作虽修改 DB/sidecar，审计未与动作共同落盘。
- 解决方案：建立隔离于数据根的持久化审计日志/事务表，带稳定事件 ID、脱敏、保留与分页；按提交结果记录，不把 best-effort 当可靠审计。
- 源码：[源码位置](D:/src/OwO/agent-sdk/crates/owo-agent-kernel/src/audit.rs:16)

### F19 [P2] 技能包导入缺少解压后资源上限
- 证据等级：源码资源边界缺口，未执行高膨胀包
- 缺陷/是否达到预期：未达到该条件下的预期。HTTP 32 MiB 限制只约束压缩包的 base64 请求；ZIP 白名单条目通过 read_to_string 无界解压进入内存。
- 根因：只限制请求大小与条目名，没有单条目/总展开字节数和压缩比约束。
- 解决方案：解析中央目录后校验大小与条目唯一性，再用有界流读取；约束单条目、总解压字节、压缩比和解析深度，超限在分配前拒绝。
- 源码：[源码位置](D:/src/OwO/agent-sdk/crates/owo-agent-memory/src/share_skill.rs:86)

## 证据与保护核查

- runtime-results.json：隔离仓库的基本接力、人工提交、替换、重启与取消测试。
- review-results.json：自批空格、跨团队评审、未知执行器替换。
- additional-results.json：领取者绕过；第一次竞态未触发；code:v1 被 superseded 拦截。不要把第一次未触发等同无缺陷。
- race-and-gate-results.json：pending ChangeSet 仍有 approved code head；并发 accept/reject 状态与 1200 文件相反。
- restore-and-inbox-results.json：Junction 恢复越界（工作区外但仍在本次 env 内）。其后 harness 编码错误已另做正确的 inbox-crash 检查。
- inbox-crash-results.json：resolving 断点重启后 409。
- feature-results.json：只读接口、哈希、context revision、模板，以及自动评审跳过冲突。
- 最初没有独立 .git 的子目录对照实验在 env/team-audit-20261013-0c1b3c71；该实验出现错误变更归属；主测试改用 env 内独立 Git 仓库排除干扰。
- 对照开始/结束的 460 个 crate Rust 源文件 SHA-256，均未改变；Git status 保持原有 vision.rs 修改与 ?? env/。
- 本轮原项目源码、配置、锁文件和构建脚本没有主动修改。所有新增脚本、合成文件、日志、DB、Junction 和测试产物均在 env 两个新建审查目录内。
- 本轮启动的检测 Daemon 已通过其专属端口关闭；原有 env/workspace 的 Daemon 保留。

建议优先修复 F01–F10 的越界/审批/恢复问题，再收敛任务与验证身份、变更检测和审计；每项补充跨 HTTP/Inbox/CLI 入口及重启/并发回归。
### F20 [P1] 技能包 ID 可使版本记录写出团队库目录
- 证据等级：已复现：隔离 HTTP 技能包测试
- 缺陷/是否达到预期：未达到。导入 manifest.id=../escaped-audit-version 返回 200，版本文件出现在 data/escaped-audit-version/versions.json。测试目标仍在 env 内；没有写到 env 之外。
- 根因：FlowSkillPackage::validate 只验证 id 非空；版本读写用 versions_root.join(id)，缺少标识符与路径边界校验。
- 解决方案：ID 使用无路径分隔符的严格标识符格式，存储文件名使用编码/映射键；读写均验证规范化目标位于版本根目录，拒绝绝对路径和 ..。
- 源码：[源码位置](D:/src/OwO/agent-sdk/crates/owo-agent-server/src/team_api.rs:74)

### F21 [P2] 技能包危险关键词的大小写检查漏拦
- 证据等级：已复现：隔离 HTTP 技能包测试
- 缺陷/是否达到预期：未达到。包含 subprocess.Popen 的合成 SKILL.md 导入返回 200、blocked=false、findings=[]；没有执行该文本中的命令。
- 根因：检测把全文转小写，却用未统一小写的危险词做 contains；subprocess.Popen、DROP TABLE、DELETE FROM 永远不能按该项匹配。
- 解决方案：统一规范化双方并补大小写用例；关键词扫描仅作为提示/拦截辅助，运行权限仍由工具宿主和策略验证，不能以扫描通过宣称可安全执行。
- 源码：[源码位置](D:/src/OwO/agent-sdk/crates/owo-agent-server/src/team_api.rs:241)

### F22 [P2] 同版本的不同技能内容覆盖，历史不可追溯
- 证据等级：已复现：隔离 HTTP 技能包测试
- 缺陷/是否达到预期：未达到。同一 ID 的 1.0.0 先导入 A 后导入 B 均成功；历史 count=1，原记录被替换，库中正文也被覆盖。
- 根因：append_version retain 移除同版本记录；FlowSkillStore::save 覆盖当前目录，不保存可恢复的版本正文。
- 解决方案：同(ID,version)同哈希按幂等处理，不同哈希返回版本冲突；修改内容必须升级版本，历史包以 CAS 或版本目录不可变保存。
- 源码：[源码位置](D:/src/OwO/agent-sdk/crates/owo-agent-server/src/team_api.rs:81)

### F23 [P2] 技能版本 sha256 字段实际不是 SHA-256
- 证据等级：已复现：隔离 HTTP 技能包测试
- 缺陷/是否达到预期：未达到。导入结果的 sha256 只有 16 个十六进制字符；实际 sha256_hex 使用 DefaultHasher::finish 的 u64。
- 根因：函数名和字段名声明 SHA-256，却返回非加密 64 位普通哈希，外部完整性核验无法按标准 SHA-256 对上。
- 解决方案：改为标准 SHA-256 的 64 个十六进制字符（256 位）；迁移时保留旧值并明确算法，不能把旧 16 字符值当 SHA-256。
- 源码：[源码位置](D:/src/OwO/agent-sdk/crates/owo-agent-server/src/team_api.rs:124)


追加验证：dynamic-rework-results.json 记录同 Worker 多任务的真实 HTTP 返工错位；sharing-results.json 记录共享导入、权限拒绝、路径 ID、关键词漏拦、同版本覆盖与哈希问题。共 23 类发现，其中 19 类已有隔离运行/实际函数/实际 Git 复现，4 类为静态缺陷或边界风险（F15、F17、F18、F19）。
