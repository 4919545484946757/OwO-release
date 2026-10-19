# ADR 0009：插件 ZIP Deflate 使用 zlib 1.3.2

- 状态：P3C 已采用，可通过构建开关回滚
- 日期：2026-08-01

## 决策

插件安装器的 ZIP method 8 原始 Deflate 解码采用官方 zlib `1.3.2`，只静态链接核心 zlib，不使用 minizip、示例、共享库或安装目标。默认构建不联网；开发者先运行 `scripts/fetch_zlib.ps1`，脚本从官方发布地址下载 ZIP，并在解压前强制核对 SHA-256：

```text
version: 1.3.2
release date: 2026-02-17
url: https://zlib.net/zlib132.zip
sha256: e8bf55f3017aa181690990cb58a994e77885da140609fc8f94abe9b65d2cae28
license: zlib License
```

项目保留原始许可证于 `third_party/licenses/zlib-1.3.2.txt`。`windows-release-zlib` 显式设置 `OWO_ENABLE_ZLIB=ON` 和固定源码目录；普通预设保持 Store-only，并在遇到 Deflate 时失败封闭。

## 安全边界

- 只调用 zlib 原始 Deflate 流接口 `inflateInit2(-MAX_WBITS)`，不使用 zlib 的文件、gzip 或 minizip API。
- 解压在任何磁盘写入前完成，输出受既有单文件 64 MiB、总计 256 MiB、压缩比 200:1 和 1024 项限制约束。
- 必须恰好消费全部压缩输入、产生 manifest 声明的精确输出尺寸并匹配 ZIP CRC-32；尾随数据、截断流、尺寸不符或 CRC 不符全部拒绝。
- zlib 1.3.2 是官方当前稳定版，包含针对外部安全审计发现的初始化、长度与不安全函数加固；升级仍需固定新版本、官方哈希、许可证和回归结果。

## 回滚

关闭 `OWO_ENABLE_ZLIB` 即恢复 Store-only；包预检、签名和 Store 解包不依赖 zlib。删除 zlib 专用预设、获取脚本和本 ADR 可移除依赖，不改变 `.owopkg` 或签名协议。
