# OwO 官方与开发中插件

此目录统一管理 OwO 项目维护的插件源码。每个插件必须使用独立子目录，根目录至少包含
`manifest.json`、`config.schema.json`，并遵循 `docs/plugins/development.md`。

目录约定：

```text
plugins/
└─ plugin-name/
   ├─ manifest.json
   ├─ config.schema.json
   ├─ resources/       # 声明式主题、字典或材质资源
   ├─ src/             # 仅进程插件需要
   └─ README.md
```

运行 `scripts/package_plugins.ps1` 会把每个有效插件目录打包到
`artifacts/plugins/*.owopkg`。构建产物、临时文件和签名密钥不得放入插件源码目录。

当前官方插件包括：

- `candidate-glass`：声明式半透明候选框主题；
- `unicode-palette`：分类 Unicode 符号列表；
- `agent-ipc`：需逐版本知情授权的本机第三方 Agent 连接器。

插件内容发生变化时必须提升 manifest 版本，禁止发布“同版本、不同摘要”的包。

