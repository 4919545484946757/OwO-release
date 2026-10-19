# P3A ONNX Runtime CPU 基准（合成排序模型）

- 日期：2026-08-01
- 配置：MSVC Release、ONNX Runtime CPU 1.28.0、单线程顺序执行、扩展图优化
- 系统：Windows 11 家庭版中文版，版本 10.0.26200（Build 26200）
- CPU：AMD Ryzen 9 9955HX，16 核 32 逻辑处理器
- 可见内存：64,676,080 KiB
- 模型：项目测试资产，589 字节 ONNX、opset 17、序列长度 64、动态批次 1～8

## 方法

命令：

```powershell
cmake --build --preset windows-release-ort --target owo_model_benchmark
1..5 | ForEach-Object {
    build/windows-release-ort/Release/owo_model_benchmark.exe tests/data/model_fixture.manifest
}
```

每轮启动独立进程，先校验资产并创建首个 ORT Session；每个批次预热 20 次，再采样 500 次。延迟包含创建 deadline timer、ORT `Run` 和取消回调管理。工作集使用 Windows `GetProcessMemoryInfo` 的进程峰值，基线在资产加载前采集。

这里的“Session 创建”不是完整 EXE 启动时间：动态链接的 ORT DLL 已在进入 `main` 前由 Windows loader 加载。合成图只用于验证运行时开销和测量管线，不能代表未来真实 BERT 权重的冷启动、内存或推理延迟。

## 结果

5 个独立进程的观测范围：

| 指标 | 结果 |
| --- | ---: |
| 资产校验 | 1.11～1.25 ms |
| 首个 Session 创建 | 44.77～50.47 ms |
| 进程初始峰值工作集 | 9.08～9.10 MB |
| 推理后峰值工作集 | 30.88～31.02 MB |
| 峰值工作集增量 | 约 21.8 MB |

各批次 p95 范围：

| 批次 | p95 | 单轮最大值（5 轮中的最大） |
| ---: | ---: | ---: |
| 1 | 7.5～8.2 μs | 16.6 μs |
| 2 | 7.5～8.4 μs | 29.5 μs |
| 4 | 7.9～9.4 μs | 33.5 μs |
| 8 | 7.9～8.8 μs | 46.2 μs |

## 发现与修正

首版 session 为每次推理创建一个轮询线程监控 deadline，合成图的 p95 因线程调度达到约 12～14 ms。改为 Windows 线程池一次性计时器，并用 `std::stop_callback` 直接触发 ORT terminate 后，p95 降至 7.5～9.4 μs；取消和超时契约测试保持通过。该修正避免了每请求建线程，也更适合 ModelHost 的低延迟路径。

## 发布门禁

- `onnxruntime.dll` SHA-256：`18370c375f07357fa5874344a9d9ac17e6b6fe1eb18b1dd209d79483b4470257`
- 发布目录包含原样 `LICENSE`（1,094 字节）和 `ThirdPartyNotices.txt`（331,175 字节）。
- CTest 对 DLL 和两份许可文件逐一与已验证 SDK 做 SHA-256 比较，缺失或内容变化即失败。
- ORT 配置完整 CTest：20/20 通过。

## 后续门禁

取得许可证清晰且具有候选排序任务头的真实权重后，必须重新测量完整进程启动、模型文件校验、Session 创建、首次推理、稳态 p50/p95/p99、峰值工作集和包体。本报告不得用于推断真实模型性能。
