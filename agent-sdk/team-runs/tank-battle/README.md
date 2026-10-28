# Tank Arena · Neon Battle

可双击打开的单页坦克大战。Canvas 2D、原生 JavaScript，无外部素材、无构建步骤、无运行时依赖。

## 启动

直接打开 `index.html`。也可以：

```powershell
python -m http.server 8080
# 浏览器打开 http://127.0.0.1:8080/
```

## 操作

| 输入 | 行为 |
| --- | --- |
| `W A S D` / 方向键 | 移动 |
| 鼠标移动 | 瞄准 |
| 鼠标左键 / `Space` | 开火（0.28s 冷却） |
| `P` | 暂停/继续 |
| `R` | 换一张新图（重新播种） |
| `Enter` / 点击 | 菜单开始、胜负后同图重开 |

最高分会保存在浏览器 localStorage（`tank-arena-best`），菜单和结算界面显示。

## 玩法

- 玩家 3 条命、每命 3 格血；受击有 1.2s 无敌闪烁。
- 敌人巡逻、追击、瞄准开火；关卡越高，敌人更多、更快、更硬。
- 砖墙可被子弹摧毁，钢墙不可摧毁。
- 清空 3 波敌人获胜；生命耗尽失败。
- HUD 显示波次、得分、生命、血量、剩余敌人；得分/击杀/过关有 WebAudio 提示音。
- 固定 60Hz 逻辑步进 + Canvas 霓虹像素风，响应式 letterbox。

## 测试

```powershell
node --test
```

当前 9 项验收测试全部通过：种子/初始状态、移动与边界阻挡、射击冷却、砖墙破坏、暂停/重开与计数归零、波次推进、三波胜利、HUD 快照，以及 20000 帧混合输入的长跑稳定性。`node --test` 使用 Node 自动发现；不要传目录（Node 22 会把目录当入口模块）。

## 结构

```
index.html            页面与 HUD 容器
styles.css            霓虹/像素风与响应式布局
src/game-core.js      确定性纯逻辑（浏览器 + Node 共用）
src/main.js           Canvas 渲染、输入、音效、rAF 循环
tests/acceptance/     自动化验收测试
playtest/             团队试玩报告（如已生成）
```
