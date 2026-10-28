# 媒体实际执行与失败输出保护：原生验证证据

日期：2026-10-27。基线 HEAD 为 21d64098，验证时 dirty=true。未把测试 EXE 当作桌面发布程序。

- results.json：最终阶段退出码、26 项原生测试分组、测试二进制 SHA-256 与范围。
- inputs-before.json：运行前冻结的 637 项选定 Rust/manifest/lock/config 输入；结束后逐项复核，一致。
- native-execution.log：4 项新集成测试。实际 HTTP 审批、内置工具、独立命名端点、六份产物、历史、重开宿主及认证交付；上游失败、拒绝、取消及同路径重试。
- native-core-media.log：19 项既有媒体/目标路径原生回归，包含范围、并发、junction、有界响应和取消。
- native-media-contracts.log：3 项既有媒体交付/宿主连接配置/TCP 观察契约；没有把 TCP 连接成功当作模型能力已验证。
- red-invalid-content.log：修复前新增回归实际 exit=101，确认失败媒体已占用输出路径。
- first-assertion-failure.log：最初测试误把来源值 b64_json 视为内联载荷，属于测试断言问题；不是业务缺陷复现。
- initial-execution-baseline.log：修正上述断言后，修复生产逻辑前的 3 项执行基线通过；当时无效内容仍留原始文件。
- check*.log、clippy*.log、fmt.log：最终定向静态检查。Clippy 使用 -D warnings；原生 MSVC 链接器创建导入库的提示保留。
- verify.ps1：本次实际 canonical 资源门禁命令，UTF-8 BOM；使用固定 T 盘源码与现存 C 盘 native-target，-j 1 / 测试线程 1。

媒体 HTTP 和模型输出为确定性夹具；未调用真实云服务、运行完整 workspace 测试、浏览器/Electron、麦克风或发布验收。文件头识别不等于完整解码；合法文件写入后 CAS/数据库失败的原子恢复不在本批完成范围。
