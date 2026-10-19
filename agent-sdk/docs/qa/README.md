# 验收计划与原始证据

本目录区分人工编写的验收计划与原始运行记录。本次整理没有运行这些验收，也没有清理历史证据。

## 计划与检查入口

- [产品评测 UI](product-eval-ui.md)
- [自适应 WorkSwarm](v1-adaptive-workswarm.md)
- [项目启动器](v1-project-launcher.md)
- [Worker 运行与产物交付](v1-worker-runtime-artifact-delivery.md)
- [WorkSwarm 进度、产物和评审](workswarm-progress-artifact-review.md)
- [WorkSwarm UI](workswarm-ui.md)

## 原始数据布局

| 位置 | 用途 | 读取方式 |
| --- | --- | --- |
| [evidence 索引](evidence/README.md) | 专项 JSON、快照和审计材料 | 先看记录中的时间、版本、exit code、方法与范围 |
| [logs 索引](logs/README.md) | 编译、测试和门禁逐行日志 | 与同批次 summary/evidence 配对 |
| 本目录 repair 与 team-engine JSON | 阶段修复和迭代台账 | 文件内容不是当次重新验证结果 |
| p6-current-install / p6-ripgrep-install | 9 月安装快照及附带工具 | 历史发布材料；保留原路径 |
| tools-ripgrep-14.1.1 | 附带工具 | 不作为研究文档 |

某些审查正文引用的证据目录实际缺失，缺失情况见 [整理记录](../internal/文档整理记录-2026-10-13.md)。缺失不能用其他日期的日志补证。
