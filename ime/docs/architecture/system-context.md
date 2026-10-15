# 系统上下文与进程边界

状态：P0 草案。

```text
Windows 应用
  ↕ TSF/COM
OwO.TSF.dll
  ↕ 版本化命名管道 IPC
OwO Core Service
  ├─ Input Engine / Dictionary / Candidate Pipeline
  ├─ Candidate UI Controller / Configuration
  ├─ ModelHost Client → ModelHost（独立进程）
  └─ PluginHost Client → PluginHost/插件（独立进程）
```

## 硬边界

- TSF DLL 不加载模型、第三方插件或不可信代码，不执行网络、磁盘或其他耗时 I/O。
- 按键处理不得等待模型、插件或网络。
- 基础候选先返回；智能结果只能以匹配的 `context_generation` 增量更新。
- 所有跨进程请求都有请求 ID、版本、超时、取消和明确错误。
- 插件默认无网络、剪贴板、输入上下文、文件写入及外部进程权限。

## P1 最小闭环

```text
测试壳/TSF 最小入口 → Core Service → 固定候选 → 选择 → 提交
```

先用测试壳验证协议、重连、超时和旧结果丢弃，再接候选窗口。完整业务接口不在 P0 冻结。

