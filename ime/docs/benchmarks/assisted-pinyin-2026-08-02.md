# 不完整与纠错拼音候选基准

- 日期：2026-08-02
- 配置：MSVC x64 Release
- 系统：Windows 11，AMD Ryzen 9 9955HX，32 逻辑处理器
- 词库：雾凇拼音完整启用词表，1,885,739 条

## 方法

`owo_engine_benchmark` 继续以 100 次预热、1,000 次采样测量“解析、索引查词、Beam Search、排序”的同步引擎阶段。默认查询集保持 `nihao`、`zhongguo`、`shijie`、`ceshi`；新增 `--assisted` 查询集为 `b`、`nih`、`zhongg`、`niaho`。

```powershell
cmake --build --preset windows-release --target owo_engine_benchmark
build/windows-release/Release/owo_engine_benchmark.exe `
    build/windows-release/rime-ice-cn-2026.06.30.owolx
build/windows-release/Release/owo_engine_benchmark.exe `
    build/windows-release/rime-ice-cn-2026.06.30.owolx --assisted
```

两组均连续运行三轮。该数据不包含 IPC、TSF 绘制和首次词库加载。

## 结果

单位：微秒。

| 查询集 | 运行 | p50 | p95 | p99 | 最大值 |
| --- | ---: | ---: | ---: | ---: | ---: |
| 精确 | 1 | 238.8 | 1793.4 | 2021.8 | 3186.3 |
| 精确 | 2 | 244.9 | 1867.9 | 2862.8 | 5317.9 |
| 精确 | 3 | 239.3 | 1675.7 | 1738.0 | 2272.9 |
| 辅助 | 1 | 4174.4 | 6622.5 | 7499.4 | 8514.5 |
| 辅助 | 2 | 4079.2 | 6434.9 | 7583.4 | 8432.8 |
| 辅助 | 3 | 4172.1 | 6394.7 | 7186.8 | 9407.9 |

按 p50 取中位运行，精确集为运行 3（p50 239.3 μs，p95 1675.7 μs），辅助集为运行 3（p50 4172.1 μs，p95 6394.7 μs）。两组每轮均返回 10,000 个候选。辅助集会同时探索末尾补全、简拼和一次纠错，因而显著高于精确输入，但仍低于 10 ms。

## 回归判断

同一轮优化前的既有 Release 二进制单轮测得精确集 p50 198.7 μs、p95 1653.5 μs、p99 1734.8 μs。实现过程中曾因“对所有合法输入预生成纠错”出现 p50 6258.7 μs、p95 37004.5 μs 的明确回归；冻结为仅在无完整切分时启用辅助路径后，最终三轮精确集 p50 为 238.8～244.9 μs、p95 为 1.68～1.87 ms，未重新引入该数量级回归。增加多音节简拼后，辅助集三轮 p95 为 6.39～6.62 ms，仍满足同步 Core 引擎的低毫秒预算。
