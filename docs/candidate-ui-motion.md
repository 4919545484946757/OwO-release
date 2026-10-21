# 候选栏视觉与交互动效

本轮保留进程内 Win32 + Direct2D / DirectWrite 渲染，不引入 WinUI、WebView、大型模型或新的常驻线程。升级的是候选栏的视觉交互层，不修改候选生成、分区、按键或上屏协议。

## 视觉

- 候选正文为 16 DIP 常规字重；拼音与标签也使用常规字重，避免粗体汉字。
- 首项有细强调线和更明确的选中轮廓；普通序号底色减弱，分区线缩短，保留原分区布局。
- 翻页按钮使用 Direct2D 矢量 chevron，不依赖三角字符字体；展开／收起按钮有独立箭头与文字。
- 皮肤插件的背景、颜色、透明度、模糊与阴影设置仍有效，主卡片、模糊覆盖和阴影圆角均维持 10 DIP。
- Unicode 插件仍是无边框的方格密堆，交互动画不添加单元格圆角。

## 动效与防闪烁

- 悬停约 140ms、按压约 80ms 的临界阻尼过渡，连续重定向保留速度，不跳变到起点。
- 展开箭头约 160ms 旋转；首次显示、翻页及展开／收起时，首项底色短时增强。候选文字、拼音和命中区域即时更新，不淡出整框，不等待动画。
- 保留稳定画布、缓存背景及 `UpdateLayeredWindow` 原子发布；动效不改变屏幕坐标、画布尺寸，不再次抓取桌面或计算高斯模糊。
- 定时器名义间隔 16ms；一次绘制超过 10ms 时，本次交互降为 33ms，下一次显示再尝试恢复。实际帧率取决于系统调度与硬件，不保证 120Hz。
- 终态即停表，隐藏／失焦不继续刷新；不调用 `timeBeginPeriod`，不创建动画线程，也不重复请求 Core。
- 跟随 Windows 客户区动画设置；高对比度与远程会话关闭动效。关闭后直接呈现最终状态。

## 专项验证

`owo.tsf.candidate_motion` 验证连续性、反向重定向、重复目标、帧率无关、时钟倒退、长时间挂起、数值边界及关闭动画。

`owo.tsf.candidate_render_preview` 使用同一原生渲染器和固定内存候选，生成普通、悬停、五行展开与玻璃皮肤画布，并检查命中区数量和边界。工具不激活 TSF、不连接 Core，也不显示或截取桌面；玻璃路径使用合成背景。图片输出到 `build/windows-release/candidate-ui-preview/`。

```powershell
cmake --build --preset windows-release --target owo_tsf owo_candidate_motion_tests owo_candidate_render_preview owo_tsf_smoke owo_candidate_window_order_tests owo_pinyin_cursor_tests
ctest --test-dir build/windows-release -C Release -R '^owo.tsf.(candidate_motion|candidate_render_preview|dll_smoke|candidate_window_order|pinyin_cursor)$' --output-on-failure
cmake --build --preset windows-x86-release --target owo_tsf
```

此为专项验证，不代替真实应用、不同显示器 DPI 和远程桌面的人工验收，不执行全量自检。开发测试部署通过带 SHA256 的 x64/x86 DLL 注册，不覆盖被宿主占用的 DLL，不停止用户应用；已经加载旧 TSF 的应用需保存后重新打开。Core 与 ModelHost 不因 UI 更新而重启。
