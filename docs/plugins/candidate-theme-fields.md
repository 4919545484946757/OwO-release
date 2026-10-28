# 亚克力候选框与设置中心颜色字段

## 插件配置入口

设置中心 → OwO 亚克力候选框 → 配置页选项框：公共效果／浅色配色／深色配色。
选项框只控制显示哪一套编辑器，不切换实际主题、不触发保存、不丢失其他页尚未保存的值。
“候选框主题”仍负责跟随系统／固定浅色／固定深色。全局搜索命中隐藏页的配置项时会自动选中该页。

颜色值使用 `#RRGGBB`，透明度值为 0–1（0 透明，1 不透明）。下表为深色字段，
浅色在颜色和透明度字段前均加 `light_`，例如 `light_caret_color`、`light_caret_opacity`。

| 元素 | 深色颜色字段 | 深色透明度字段 |
| --- | --- | --- |
| 候选框背景 | `background_color` | `background_opacity` |
| 外边框 | `border_color` | `border_opacity` |
| 分割线 | `divider_color` | `divider_opacity` |
| 拼音/预览文字 | `pinyin_text_color` | `pinyin_text_opacity` |
| 候选文字 | `candidate_text_color` | `candidate_text_opacity` |
| 首候选额外高亮 | `candidate_block_color` | `candidate_block_opacity` |
| 候选序号 | `candidate_number_color` | `candidate_number_opacity` |
| 阴影 | `shadow_color` | `shadow_opacity` |
| 序号底块 | `candidate_badge_color` | `candidate_badge_opacity` |
| 展开选中页背景 | `selected_page_color` | `selected_page_opacity` |
| 候选悬停背景 | `hover_block_color` | `hover_block_opacity` |
| 候选按下背景 | `pressed_block_color` | `pressed_block_opacity` |
| 控制按钮背景 | `button_background_color` | `button_background_opacity` |
| 控制按钮边框 | `button_border_color` | `button_border_opacity` |
| 控制按钮文字 | `button_text_color` | `button_text_opacity` |
| 状态与次要文字 | `secondary_text_color` | `secondary_text_opacity` |
| 拼音光标 | `caret_color` | `caret_opacity` |

背景开启高斯模糊后，`background_opacity` 控制底色与模糊背景的混合强度，
不是再次降低整个窗口的 alpha。首候选额外高亮在浅色展开状态保留 0.8 倍调节，
即默认 5% 得到 4%；`selected_page` 独立控制整行高亮，不与首候选使用同一透明度。
悬停、按下状态分别优先采用 `hover_block`、`pressed_block`，不受首候选额外高亮缩放影响。
控制按钮包括翻页、展开/收起和符号入口；禁用按钮文字使用 `secondary_text`。

公共字段：`theme_mode`（`system`／`light`／`dark`）、`background_blur_enabled`、
`background_blur_radius`、`shadow_enabled`、`shadow_blur_radius`、`shadow_offset_x`、`shadow_offset_y`。
两套主题共用这些效果参数；阴影颜色及透明度仍独立。

旧主题未定义新增角色时保持继承：序号底块/按钮边框继承边框，悬停/按下继承候选块，
选中行继承候选块并乘 0.7，按钮背景透明，按钮文字继承候选文字，次要文字透明度乘 0.64，
光标继承拼音文字。新增主题角色在资源中定义时必须同时声明 `.color` 与 `.opacity`。
插件 1.4.0 需要本次更新的宿主，不能直接安装到不认识这些角色的旧宿主。

## 设置中心自身的 WinUI 颜色资源

这些是界面源码中的 XAML 资源键，不是插件 `settings.json` 的可保存字段；插件不能修改设置中心的主题资源。

| 资源键 | 用途 |
| --- | --- |
| `SettingsContentBackgroundBrush` | 内容区域背景；浅色 `#D1DDEC`，深色 `#1A2032`，高对比度跟随系统 |
| `SolidBackgroundFillColorBaseBrush` | 根容器、导航和底栏背景 |
| `CardBackgroundFillColorDefaultBrush` | 设置项卡片、插件颜色折叠栏底色 |
| `CardStrokeColorDefaultBrush` | 卡片和折叠栏边框 |
| `TextFillColorPrimaryBrush` | WinUI 默认主要文字 |
| `TextFillColorSecondaryBrush` | 说明、状态和辅助文字 |
| `DividerStrokeColorDefaultBrush` | 分割线 |

自定义内容背景定义于 `apps/settings_center/MainPage.xaml` 的 `ThemeDictionaries`；
其余键由 `App.xaml` 合并的 WinUI `XamlControlsResources` 提供。使用 `ThemeResource` 引用，
不要把某个主题解析后的颜色永久写回另一套主题配置。
