# 更新与资源下载

## 用户使用

入口：设置中心 → 通用设置 → 更新与资源下载。插件、词库管理器和智能排序设置也提供在线下载入口；全局搜索可搜索“更新”“下载”“模型”。

点击“检查 GitHub 更新与资源”后，查询官方仓库 [`4919545484946757/OwO-release`](https://github.com/4919545484946757/OwO-release/releases) 的最新正式发行版。只在主动操作时联网，不在输入链路中运行，不上传拼音、正文、应用上下文、配置或用户词库。GitHub/CDN 会接收 IP、请求路径和客户端标识，这是下载连接本身所需的数据。

- 更新：比较稳定版本号，拒绝自动降级；当前仅提供 Windows x64 安装入口。先确认下载，校验成功后再次确认启动现有安装助手，不静默安装、不结束输入服务、不自动删除旧数据。其他架构可查看发行页，但本入口不会自动启动不匹配的安装包。
- 插件：下载后沿用现有插件安装和逐包风险、权限确认流程，不因来源为官方仓库而免授权。
- 词库：下载后进入现有导入预览，确认后才保存。支持 JSON、TXT、TSV、CSV、YAML/YML 和 Gboard ZIP，由现有导入器识别；单个文件不超过 32 MiB。并非任意 ZIP 都是受支持的词典。
- 模型：目前支持 `.lm` 文件的下载和校验，显示保存位置；不会自动切换模型、重启 ModelHost 或承诺任意 `.lm` 均与当前后端兼容。模型启用仍需兼容性、许可及本地部署确认。

下载有进度和取消按钮；元数据请求限时 30 秒，单次整体操作限时 15 分钟。断网、限流、格式错误或校验失败会提示原因，可重新操作；不会影响离线输入和当前已启用资源。暂不提供断点续传。

缓存位置：`%LOCALAPPDATA%\OwO\InputMethod\downloads\<SHA-256>\<文件名>`。流式下载到唯一临时文件；成功校验声明长度和 SHA-256 后才替换同哈希缓存文件，失败或取消清理临时文件。启动安装助手前再验一次哈希。只允许官方仓库的初始资产 URL 和白名单 GitHub HTTPS/CDN 重定向，不接受任意镜像 URL。

哈希校验保证下载与官方发布信息一致，不等同于商业代码签名，也不能证明资源安全。当前程序不包含 GitHub 发布令牌、不调用第三方代理。公开 GitHub 可能因网络或未认证 API 限流不可用；可通过官方发行页手动下载，不会降低校验标准。

旧发行版没有资源目录时，仅列出 GitHub 提供 SHA-256 的 `.owopkg` 资产。没有目录的词库和模型不会凭空出现。首次上线本功能不代表已把本机插件、私人词库或模型上传至 GitHub。

## 发行资源目录（初版，可扩展）

发行资产中的 `owo-resources.json` 必须有 GitHub 的 SHA-256 摘要，最大 1 MiB。目录与资源属于同一发行版；资产引用只能是名称，不能指定远程 URL、本地路径或执行命令。

```json
{
  "schema_version": 1,
  "release_tag": "v0.2.7",
  "resources": [
    {
      "id": "org.example.dictionary",
      "name": "示例字典",
      "kind": "dictionary",
      "version": "1.0.0",
      "license": "按实际资源填写许可证",
      "asset": "example-dictionary.tsv",
      "sha256": "填写该资产完整的64位十六进制SHA-256"
    }
  ]
}
```

`kind` 为 `plugin`、`dictionary`、`model`；资源上限 100 项，单资产上限 2 GiB（词库另受 32 MiB 限制）。名称和 ID 有长度限制，资产名只允许安全的 ASCII 文件名。目录摘要由 GitHub 提供，资源摘要若与 GitHub 资产摘要同时存在必须一致；缺失、无效或冲突均拒绝。

这是分发元数据，不取代插件清单、权限、输入方案 API 或模型 ABI。插件安装仍由宿主独立校验；模型目录暂不做跨后端激活。将来增加架构、后端及最低宿主版本字段时需版本化扩展，不能用自然语言备注驱动执行。

## 维护者发布流程

发布脚本固定写入已确认的 `4919545484946757/OwO-release`，不会向另一个 Git remote 发布。应先提交代码、完成版本号更新与所需验收，推送该提交到此仓库，然后从干净提交重新编译和打包；不要给旧载荷改名充当新版本。

```powershell
# 先按 README 完成 Release、设置中心构建；版本仅为示例。
.\scripts\package_release.ps1 -Version 0.2.7
.\scripts\package_setup.ps1 -Version 0.2.7

# 仅读取、校验并显示计划，不写 GitHub、不生成目录。
.\scripts\publish_github_release.ps1 -Version 0.2.7

# 可选：把已获分发许可的资源放进发行目录，再传入资源清单。
.\scripts\publish_github_release.ps1 -Version 0.2.7 -ResourceList .\build\resources-to-publish.json

# 确认计划后才发布（需要已在本机环境中配置发布凭据）。
.\scripts\publish_github_release.ps1 -Version 0.2.7 -ResourceList .\build\resources-to-publish.json -Publish
```

`ResourceList` 是 JSON 数组，每项填写 `id/name/kind/version/license/asset`，字段同上；无需填写 `sha256`，由发布脚本计算。资源文件名不可与安装包、源码包、校验单、构建收据或资源目录重名。没有资源也可发布空目录，但此时不会下载到额外插件、词库和模型。

`package_release.ps1` 写入 `release-build.json`，记录版本、提交、工作区是否干净和时间。发布脚本校验：收据提交必须等于 HEAD、收据无未提交改动、Setup/便携包/源码包均存在且符合 `SHA256SUMS.txt`；真正发布时要求当前工作区也干净，并检查该源提交已在官方仓库。收据是流程检查，不是密码学构建证明。

真正发布读取环境变量 `GH_TOKEN` 或 `GITHUB_TOKEN`，令牌只需官方发行仓库的 `Contents: write` 权限。通过可信凭据管理或本机环境配置，禁止提交到仓库、嵌入应用、打印令牌或把令牌粘贴到聊天。没有凭据时只能验证计划，不能声称已发布。

脚本先创建草稿，上传 Setup、便携包、源码包、校验单、构建收据、资源目录和显式列出的资源，并逐一比较 GitHub 返回的长度及 SHA-256，全部成功后才公开为最新正式版。已存在的版本拒绝覆盖；上传失败保留草稿、不会公开，也不会自动删除远程资产。处理草稿后使用新版本或由维护者明确处理原草稿，不能盲目重试覆盖。

首次实际发布前仍需独立检查许可证及资源再分发权、安装兼容性、发布说明和签名状态。程序不会自动采集或上传本机启用插件、配置、私人词库或模型。

## 定向测试

```powershell
dotnet run --project tests/settings_center/online_distribution_tests.csproj -c Release
dotnet run --project tests/settings_center/online_distribution_tests.csproj -c Release -- --live-read-only
dotnet run --project tests/settings_center/dictionary_manager_tests.csproj -c Release
powershell -NoProfile -ExecutionPolicy Bypass -File tests/scripts/publish_github_release_tests.ps1
```

离线测试覆盖版本比较、来源约束、重定向、大小、摘要、取消、临时文件及资源目录。`--live-read-only` 只读取真实公开版本和目录，不下载安装包，不安装资源、不发布。发布脚本测试使用隔离的假载荷，只验证离线计划和错误拒绝，不测试真实上传权限或远程写入。
