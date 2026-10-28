# Logo 与主题外观

六份用户提供的原始 SVG 保存在 `apps/settings_center/Assets/Brand/`：

| 资源 | 用途 |
| --- | --- |
| `LogoSmallLight.svg` / `LogoSmallDark.svg` | 加粗小图标：设置中心侧栏、窗口标题栏、64 px 及以下 ICO 帧 |
| `LogoRoundedLight.svg` / `LogoRoundedDark.svg` | 原版圆角：大尺寸兼容 PNG、128/256 px ICO 帧 |
| `LogoSquareLight.svg` / `LogoSquareDark.svg` | 安装欢迎页品牌展示 |
| `AppIcon.Light.ico` / `AppIcon.Dark.ico` | 运行中窗口及任务栏图标，含 16–256 px 九档尺寸 |

通用设置中的“Logo 外观”支持跟随系统、浅色、深色。偏好保存于
`config/settings-ui.json` 的 `LogoTheme` 字段：`system` / `light` / `dark`。
该选项独立于整个设置中心的主题、亚克力候选框插件配色和自动保存开关，并立即保存。
跟随系统时依据 WinUI 实际应用主题更新，无需重启；手动覆盖只作用于 Logo。
配置导出包含该偏好；旧配置没有该字段时保留本机偏好，不恢复为默认。

SVG 保持原始路径和配色，不依赖字体。`scripts/generate-brand-assets.cjs`
使用 Node.js 与 sharp，从 SVG 生成 ICO 和 Windows 所需 PNG；运行同一脚本的
`--check` 可只读检查 SVG 尺寸、ICO 帧尺寸、透明通道与静态图标一致性。
这些兼容资源不是运行时 SVG 的替代源，不修改用户提供的原图文件。

Windows 的 EXE 文件、安装包和普通快捷方式使用静态嵌入资源 `Assets/AppIcon.ico`
（圆角深色版）；运行中窗口通过 `AppWindow.SetIcon` 切换对应 ICO。
资源管理器与已固定快捷方式可能显示 Windows 缓存的静态图标，它们不会像界面内 Logo
一样随选项实时更新。不改写用户的桌面/任务栏快捷方式，也不重启 Explorer。
TSF 的“中／英”状态图标保留，避免丢失输入模式提示。

修改品牌资源后需重新编译设置中心和安装助手，发布时必须包含完整 `Assets/Brand` 目录。
不修改安装进度页的摄影横幅 `SetupProgressBanner.png`。
