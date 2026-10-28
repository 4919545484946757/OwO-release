# 部分候选选择与剩余拼音

12个合成场景覆盖连续/显式拼音、三段消费、大写、单字拆分与较长定语。

- 直接CLI回放：10项通过；2项显式撇号场景因剩余首字符为 `'` 未达到预期。
- 按已检查的TSF宿主规则清理剩余开头撇号后，12项全部完成上屏内容与剩余拼音断言。

实例 `ni'hao'shi'jie` 选择“你好”时，Core消费6字节，原始剩余为 `'shi'jie`。
`src/tsf/text_service.cpp` 的直接上屏路径会移除剩余开头撇号；暂存路径也会越过这些分隔符。
CLI的 `input replay` 只删除消费字节，没有这层宿主清理，因此不能把CLI两项失败直接当作TSF产品故障。
若CLI定位为完整宿主行为模拟器，这属于需要完善的工具一致性事项。

本测试不模拟真实文本服务焦点、键盘或鼠标点击，也没有修改现有CLI/Core/TSF实现。
`-HostSeparatorCleanup` 只是测试脚本显式模拟该已知适配步骤，原始失败记录保留。

原始结果目录：

- `build/quality-partial-selection-20261029/run-2c5b194164474ec4b72cd2535d948154`：直接CLI模式。
- `build/quality-partial-selection-20261029/run-c14a1462a01740a1b126b52e809480c0`：模拟宿主清理。

复现脚本：`scripts/run-partial-selection-quality-repros.ps1`。
