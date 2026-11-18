use async_trait::async_trait;
use owo_agent_kernel::required_string;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

use crate::executor;
use crate::tools::{ToolContext, ToolSpec};
use crate::Tool;

use super::sim::*;
pub struct ScreenOcrTool;

#[async_trait]
impl Tool for ScreenOcrTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "screen_ocr".into(),
            description: "截取当前屏幕（或模拟窗口）并做本地 OCR，返回整行文本 lines（含坐标和 role_hint=button/input/header/text）。定位控件优先用本工具：找到目标行后点击该行中心。不要用 ocr_region 代替本工具".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "max_boxes": { "type": "integer", "description": "最多返回多少词框（默认 0：不带 boxes，避免超大结果影响多轮工具调用；lines 已含坐标）" }
                }
            }),
            effect: None,
        }
    }

    async fn run(&self, _ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let max_boxes = args.get("max_boxes").and_then(Value::as_u64).unwrap_or(0) as usize;
        ocr_screen(max_boxes).await
    }
}

pub struct OcrRegionTool;

#[async_trait]
impl Tool for OcrRegionTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "ocr_region".into(),
            description: "仅当需要放大识别小字/弹窗时才用（可传 scale 放大）；正常情况下定位控件请用 screen_ocr 的 lines".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "x": { "type": "integer" },
                    "y": { "type": "integer" },
                    "width": { "type": "integer" },
                    "height": { "type": "integer" },
                    "scale": { "type": "integer", "description": "放大倍数，默认 2" }
                },
                "required": ["x", "y", "width", "height"]
            }),
            effect: None,
        }
    }

    async fn run(&self, _ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let x = args.get("x").and_then(Value::as_i64).ok_or("缺少 x")? as i32;
        let y = args.get("y").and_then(Value::as_i64).ok_or("缺少 y")? as i32;
        let width = args
            .get("width")
            .and_then(Value::as_i64)
            .ok_or("缺少 width")? as i32;
        let height = args
            .get("height")
            .and_then(Value::as_i64)
            .ok_or("缺少 height")? as i32;
        let scale = args.get("scale").and_then(Value::as_u64).unwrap_or(2) as u32;
        if on_sim_surface() {
            if let Some(mut result) = sim_ocr_lines().await {
                let lines = result.get("lines").cloned().unwrap_or_else(|| json!([]));
                let filtered: Vec<Value> = lines
                    .as_array()
                    .map(|array| {
                        array
                            .iter()
                            .filter(|line| {
                                let line_x =
                                    line.get("x").and_then(Value::as_i64).unwrap_or(0) as i32;
                                let line_y =
                                    line.get("y").and_then(Value::as_i64).unwrap_or(0) as i32;
                                let line_w =
                                    line.get("width").and_then(Value::as_i64).unwrap_or(0) as i32;
                                let line_h =
                                    line.get("height").and_then(Value::as_i64).unwrap_or(0) as i32;
                                line_x < x + width
                                    && line_x + line_w > x
                                    && line_y < y + height
                                    && line_y + line_h > y
                            })
                            .cloned()
                            .collect()
                    })
                    .unwrap_or_default();
                let text: String = filtered
                    .iter()
                    .filter_map(|line| line.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join(" ");
                result["lines"] = json!(filtered);
                result["text"] = json!(text);
                result["chars"] = json!(text.chars().count());
                if let Value::Object(map) = &mut result {
                    map.insert("surface".into(), json!("sim"));
                    map.insert("note".into(), json!("坐标为模拟窗口坐标"));
                }
                return Ok(result);
            }
        }
        let bmp = if on_sim_surface() {
            sim_fetch_frame().await?
        } else {
            crate::platform::capture_screen().ok_or("屏幕截图失败")?
        };
        let cropped = crate::ocr::crop_scale_bmp(&bmp, x, y, width, height, scale)
            .map_err(|e| format!("区域裁剪失败：{e}"))?;
        let summary = crate::paddle_ocr::ocr_preferred(&cropped)
            .await
            .map_err(|e| format!("区域 OCR 失败：{e}"))?;
        let mut result = ocr_summary_json(&summary, 200);
        if let Value::Object(map) = &mut result {
            map.insert(
                "surface".into(),
                json!(if on_sim_surface() { "sim" } else { "desktop" }),
            );
        }
        Ok(result)
    }
}

pub struct DesktopWindowOcrTool;

#[async_trait]
impl Tool for DesktopWindowOcrTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "desktop_window_ocr".into(),
            description: "后台只读抓取指定窗口内容并 OCR（PrintWindow，可抓被遮挡窗口；传 hwnd，或 process/title 模糊匹配），返回窗口屏幕矩形和整行文本（屏幕坐标），用于窗口级情景理解".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "hwnd": { "type": "integer" },
                    "process": { "type": "string" },
                    "title": { "type": "string" }
                }
            }),
            effect: None,
        }
    }

    async fn run(&self, _ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let hwnd: isize = if let Some(value) = args.get("hwnd").and_then(Value::as_i64) {
            value as isize
        } else {
            let process = args
                .get("process")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let title = args
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if process.is_empty() && title.is_empty() {
                return Err("desktop_window_ocr 需要 hwnd 或 process/title".to_string());
            }
            let windows = crate::platform::window_list();
            windows
                .iter()
                .find(|window| {
                    window.visible
                        && ((!process.is_empty() && window.process.contains(process))
                            || (!title.is_empty() && window.title.contains(title)))
                })
                .map(|window| window.hwnd)
                .ok_or_else(|| format!("未找到窗口（process={process}, title={title}）"))?
        };
        ocr_window_at_screen_coordinates(hwnd).await
    }
}

pub(super) async fn ocr_window_at_screen_coordinates(hwnd: isize) -> Result<Value, String> {
    let (bmp, rect) = crate::platform::capture_window_bmp_deep(hwnd)
        .ok_or_else(|| format!("窗口截图失败（hwnd={hwnd}）"))?;
    let summary = crate::paddle_ocr::ocr_preferred(&bmp).await?;
    let mut result = ocr_summary_json(&summary, 200);
    if let Value::Object(map) = &mut result {
        if let Some(lines) = map.get_mut("lines").and_then(Value::as_array_mut) {
            for line in lines {
                if let Some(x) = line.get("x").and_then(Value::as_i64) {
                    line["x"] = json!(x + rect.0 as i64);
                }
                if let Some(y) = line.get("y").and_then(Value::as_i64) {
                    line["y"] = json!(y + rect.1 as i64);
                }
            }
        }
        if let Some(boxes) = map.get_mut("boxes").and_then(Value::as_array_mut) {
            for b in boxes {
                if let Some(x) = b.get("x").and_then(Value::as_i64) {
                    b["x"] = json!(x + rect.0 as i64);
                }
                if let Some(y) = b.get("y").and_then(Value::as_i64) {
                    b["y"] = json!(y + rect.1 as i64);
                }
            }
        }
        map.insert("surface".into(), json!("window"));
        map.insert(
            "window".into(),
            json!({ "hwnd": hwnd, "rect": [rect.0, rect.1, rect.2, rect.3] }),
        );
    }
    Ok(result)
}

pub struct DesktopForegroundTool;

#[async_trait]
impl Tool for DesktopForegroundTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "desktop_foreground".into(),
            description: "返回当前前台窗口的进程名、标题和屏幕矩形".into(),
            input_schema: json!({ "type": "object", "properties": {} }),
            effect: None,
        }
    }

    async fn run(&self, _ctx: &mut ToolContext<'_>, _args: Value) -> Result<Value, String> {
        if on_sim_surface() {
            return Ok(json!({
                "process": "owo-sim-qq",
                "title": "OwO 模拟QQ - 张子豪",
                "rect": [0, 0, 1020, 700],
                "surface": "sim",
            }));
        }
        let (app_id, title) =
            crate::platform::poll_foreground_app().ok_or_else(|| "无法获取前台窗口".to_string())?;
        let rect = crate::platform::foreground_window_rect();
        Ok(json!({ "process": app_id, "title": title, "rect": rect }))
    }
}

pub struct DesktopWindowListTool;

#[async_trait]
impl Tool for DesktopWindowListTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "desktop_window_list".into(),
            description:
                "列出当前所有可见顶层窗口（进程名/标题/矩形），用于找到 QQ、浏览器等目标窗口".into(),
            input_schema: json!({ "type": "object", "properties": {} }),
            effect: None,
        }
    }

    async fn run(&self, _ctx: &mut ToolContext<'_>, _args: Value) -> Result<Value, String> {
        if on_sim_surface() {
            return Ok(json!({
                "windows": [{
                    "hwnd": 1,
                    "pid": 1,
                    "process": "owo-sim-qq",
                    "title": "OwO 模拟QQ - 张子豪",
                    "rect": [0, 0, 1020, 700],
                    "visible": true,
                }],
                "surface": "sim",
            }));
        }
        let windows = crate::platform::window_list();
        Ok(json!({ "windows": windows }))
    }
}

pub struct DesktopActivateTool;

#[async_trait]
impl Tool for DesktopActivateTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "desktop_activate".into(),
            description: "把指定进程名或标题的窗口切到前台（可传 process 或 title，模糊匹配）"
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "process": { "type": "string", "description": "例如 qq / msedge / owo-sim-qq" },
                    "title": { "type": "string", "description": "窗口标题包含文本" }
                }
            }),
            effect: None,
        }
    }

    async fn run(&self, _ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        if on_sim_surface() {
            let process = args
                .get("process")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let title = args
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if process.is_empty() && title.is_empty() {
                return Err("desktop_activate 需要 process 或 title".to_string());
            }
            return Ok(json!({ "activated": true, "foreground": "owo-sim-qq", "surface": "sim" }));
        }
        let process = args
            .get("process")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let title = args
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        crate::platform::activate_window(&process, &title)?;
        std::thread::sleep(Duration::from_millis(200));
        let (app_id, title) = crate::platform::poll_foreground_app().unwrap_or_default();
        Ok(json!({ "activated": true, "foreground": app_id, "title": title }))
    }
}

pub struct DesktopClickTool;

#[async_trait]
impl Tool for DesktopClickTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "desktop_click".into(),
            description: "单击鼠标左键：传入 (x, y) 屏幕坐标，或传入元素注册表的 element_id（需先经 vision_ground/screen_ocr 刷新）自动取元素中心"
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "x": { "type": "integer", "description": "屏幕坐标（与 y 同传时使用）" },
                    "y": { "type": "integer" },
                    "element_id": { "type": "string", "description": "窗口元素注册表的稳定元素 ID（与 app_id 同传时优先于坐标）" },
                    "app_id": { "type": "string", "description": "element_id 所属应用标识（如 qq）" }
                }
            }),
            effect: None,
        }
    }

    async fn run(&self, ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        if let Some(element_id) = args.get("element_id").and_then(Value::as_str) {
            let app_id = args
                .get("app_id")
                .and_then(Value::as_str)
                .ok_or("缺少 app_id（element_id 需要所属应用）")?;
            let (x, y) = {
                let registry = ctx
                    .elements
                    .lock()
                    .map_err(|_| "窗口元素注册表锁中毒".to_string())?;
                let element = registry.get_by_id(app_id, element_id).ok_or_else(|| {
                    format!("稳定元素 ID 未命中（可能已失效）：{element_id}；请先刷新感知")
                })?;
                (
                    element.x + element.width / 2,
                    element.y + element.height / 2,
                )
            };
            if on_sim_surface() {
                return sim_post("click", json!({ "x": x, "y": y })).await;
            }
            executor::click_at_screen(x, y)?;
            return Ok(json!({
                "clicked": [x, y],
                "element_id": element_id,
                "app_id": app_id,
            }));
        }
        let x = args
            .get("x")
            .and_then(Value::as_i64)
            .ok_or("缺少 x 或 element_id")? as i32;
        let y = args.get("y").and_then(Value::as_i64).ok_or("缺少 y")? as i32;
        if on_sim_surface() {
            return sim_post("click", json!({ "x": x, "y": y })).await;
        }
        executor::click_at_screen(x, y)?;
        Ok(json!({ "clicked": [x, y] }))
    }
}

pub struct DesktopTypeTool;

#[async_trait]
impl Tool for DesktopTypeTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "desktop_type".into(),
            description: "向前台窗口注入 UTF-16 文本（不依赖 IME；中文/英文/表情均可）".into(),
            input_schema: json!({
                "type": "object",
                "properties": { "text": { "type": "string" } },
                "required": ["text"]
            }),
            effect: None,
        }
    }

    async fn run(&self, _ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let text = required_string(&args, "text")?;
        if on_sim_surface() {
            return sim_post("type", json!({ "text": text })).await;
        }
        executor::send_unicode(&text)?;
        Ok(json!({ "typed_chars": text.chars().count() }))
    }
}

pub struct DesktopKeyTool;

#[async_trait]
impl Tool for DesktopKeyTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "desktop_key".into(),
            description: "向前台窗口发送单个按键（enter/tab/backspace/delete/escape/space/up/down/left/right/home/end/f1-f24）".into(),
            input_schema: json!({
                "type": "object",
                "properties": { "key": { "type": "string" } },
                "required": ["key"]
            }),
            effect: None,
        }
    }

    async fn run(&self, _ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let key = required_string(&args, "key")?;
        if on_sim_surface() {
            return sim_post("key", json!({ "key": key })).await;
        }
        executor::send_shortcut(&key)?;
        Ok(json!({ "key": key }))
    }
}

pub struct DesktopShortcutTool;

#[async_trait]
impl Tool for DesktopShortcutTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "desktop_shortcut".into(),
            description:
                "向前台窗口发送组合键，例如 ctrl+a / ctrl+c / ctrl+v / alt+tab / ctrl+shift+o"
                    .into(),
            input_schema: json!({
                "type": "object",
                "properties": { "combo": { "type": "string" } },
                "required": ["combo"]
            }),
            effect: None,
        }
    }

    async fn run(&self, _ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let combo = required_string(&args, "combo")?;
        if on_sim_surface() {
            return sim_post("key", json!({ "key": combo })).await;
        }
        executor::send_shortcut(&combo)?;
        Ok(json!({ "combo": combo }))
    }
}

pub struct DesktopLaunchTool;

#[async_trait]
impl Tool for DesktopLaunchTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "desktop_launch".into(),
            description: "启动应用（exe 路径）或打开 URL（交给系统默认浏览器）".into(),
            input_schema: json!({
                "type": "object",
                "properties": { "target": { "type": "string" } },
                "required": ["target"]
            }),
            effect: None,
        }
    }

    async fn run(&self, _ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let target = required_string(&args, "target")?;
        if on_sim_surface() {
            return Ok(json!({ "launched": target, "surface": "sim" }));
        }
        executor::launch_target(&target)?;
        Ok(json!({ "launched": target }))
    }
}

pub struct DesktopWaitTool;

#[async_trait]
impl Tool for DesktopWaitTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "desktop_wait".into(),
            description: "等待指定毫秒（最多 120000），用于等对方回复/页面加载/动画完成".into(),
            input_schema: json!({
                "type": "object",
                "properties": { "ms": { "type": "integer" } },
                "required": ["ms"]
            }),
            effect: None,
        }
    }

    async fn run(&self, _ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let ms = args
            .get("ms")
            .and_then(Value::as_u64)
            .ok_or("缺少 ms")?
            .min(120_000);
        tokio::time::sleep(Duration::from_millis(ms)).await;
        Ok(json!({ "waited_ms": ms }))
    }
}

pub struct DesktopScrollTool;

#[async_trait]
impl Tool for DesktopScrollTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "desktop_scroll".into(),
            description: "把鼠标移到屏幕坐标 (x,y) 并滚动滚轮（delta 正数向上、负数向下，一格 120），用于滚动聊天/列表".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "x": { "type": "integer" },
                    "y": { "type": "integer" },
                    "delta": { "type": "integer" }
                },
                "required": ["x", "y", "delta"]
            }),
            effect: None,
        }
    }

    async fn run(&self, _ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let x = args.get("x").and_then(Value::as_i64).ok_or("缺少 x")? as i32;
        let y = args.get("y").and_then(Value::as_i64).ok_or("缺少 y")? as i32;
        let delta = args
            .get("delta")
            .and_then(Value::as_i64)
            .ok_or("缺少 delta")? as i32;
        if on_sim_surface() {
            // 模拟窗口布局固定，无需滚动。
            return Ok(json!({ "scrolled": [x, y, delta], "surface": "sim" }));
        }
        executor::scroll_at_screen(x, y, delta)?;
        Ok(json!({ "scrolled": [x, y, delta] }))
    }
}

pub struct DesktopWaitUntilTool;

#[async_trait]
impl Tool for DesktopWaitUntilTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "desktop_wait_until".into(),
            description: "轮询屏幕 OCR，直到出现包含指定文本的行（可限定 role_hint=button/input/message/header）；用于等待对方回复/页面加载/消息上屏，返回匹配行与坐标；超时返回 matched=false".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "text": { "type": "string", "description": "要等待出现的文本" },
                    "role": { "type": "string", "description": "可选：限定行类型 button/input/message/header" },
                    "timeout_ms": { "type": "integer", "description": "最长等待毫秒，默认 30000，最大 120000" },
                    "interval_ms": { "type": "integer", "description": "轮询间隔毫秒，默认 1000" }
                },
                "required": ["text"]
            }),
            effect: None,
        }
    }

    async fn run(&self, _ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let needle = required_string(&args, "text")?;
        let role = args
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let timeout_ms = args
            .get("timeout_ms")
            .and_then(Value::as_u64)
            .unwrap_or(30_000)
            .min(120_000);
        let interval_ms = args
            .get("interval_ms")
            .and_then(Value::as_u64)
            .unwrap_or(1_000)
            .max(200);
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        let mut last_ocr = Value::Null;
        let mut last_error = String::new();
        while Instant::now() < deadline {
            match ocr_screen(0).await {
                Ok(ocr) => {
                    last_ocr = ocr.clone();
                    if let Some(line) = find_ocr_line(&ocr, &needle, &role) {
                        let elapsed = timeout_ms.saturating_sub(
                            deadline
                                .saturating_duration_since(Instant::now())
                                .as_millis() as u64,
                        );
                        return Ok(json!({
                            "matched": true,
                            "text": needle,
                            "line": line,
                            "elapsed_ms": elapsed,
                            "surface": ocr.get("surface").cloned().unwrap_or(json!("unknown")),
                        }));
                    }
                }
                Err(error) => {
                    last_error = error;
                }
            }
            tokio::time::sleep(Duration::from_millis(interval_ms)).await;
        }
        let elapsed_ms = timeout_ms.saturating_sub(
            deadline
                .saturating_duration_since(Instant::now())
                .as_millis() as u64,
        );
        let preview: String = last_ocr
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .chars()
            .take(500)
            .collect();
        Ok(json!({
            "matched": false,
            "text": needle,
            "elapsed_ms": elapsed_ms,
            "surface": last_ocr.get("surface").cloned().unwrap_or(json!("unknown")),
            "last_error": last_error,
            "last_ocr_preview": preview,
        }))
    }
}

pub struct ScreenVisionTool;

/// 视觉面截图：可选区域裁剪+放大（小字/局部验证用）。
pub(super) async fn capture_vision_png_with_region(
    args: &Value,
) -> Result<(Vec<u8>, String), String> {
    if let (Some(x), Some(y), Some(width), Some(height)) = (
        args.get("x").and_then(Value::as_i64),
        args.get("y").and_then(Value::as_i64),
        args.get("width").and_then(Value::as_i64),
        args.get("height").and_then(Value::as_i64),
    ) {
        let scale = args.get("scale").and_then(Value::as_u64).unwrap_or(3) as u32;
        return crate::vision::capture_vision_png_region(
            x as i32,
            y as i32,
            width as i32,
            height as i32,
            scale,
        )
        .await;
    }
    crate::vision::capture_vision_png().await
}

#[async_trait]
impl Tool for ScreenVisionTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "screen_vision".into(),
            description: "把当前屏幕（或模拟窗口）截图交给视觉模型做场景描述（本地 Ollama 或 BYOK 云端）；视觉只用于理解与验证，不直接控制，主控制仍用 screen_ocr".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "prompt": { "type": "string", "description": "可选：自定义描述指令" },
                    "x": { "type": "integer", "description": "可选：区域左上角 x（与 width/height 同传时裁剪放大）" },
                    "y": { "type": "integer" },
                    "width": { "type": "integer" },
                    "height": { "type": "integer" },
                    "scale": { "type": "integer", "description": "区域放大倍数，默认 3" }
                }
            }),
            effect: None,
        }
    }

    async fn run(&self, _ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let (png, surface) = capture_vision_png_with_region(&args).await?;
        let prompt = args
            .get("prompt")
            .and_then(Value::as_str)
            .unwrap_or(
                "请用中文描述这个界面的当前状态：这是什么应用？有哪些关键控件（按钮/输入框/消息）？\
                 它们大致在什么位置（给出屏幕坐标范围）？最新消息内容是什么？",
            )
            .to_string();
        let description = crate::vision::describe_image(&png, &prompt).await?;
        let config = crate::vision::VisionConfig::from_env();
        Ok(json!({
            "surface": surface,
            "provider": config.provider,
            "model": config.model,
            "description": description,
        }))
    }
}

pub struct VisionVerifyTool;

#[async_trait]
impl Tool for VisionVerifyTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "vision_verify".into(),
            description: "让视觉模型针对当前截图回答 yes/no 问题（如“消息是否已上屏”“输入框是否已清空”），返回 answer/confidence，用于异步完成验证；默认忽略输入框占位文字".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "question": { "type": "string" },
                    "ignore_placeholder": { "type": "boolean", "description": "是否忽略输入框占位文字（默认 true，避免把“输入消息...”误判为实际内容）" },
                    "x": { "type": "integer", "description": "可选：只验证该区域（裁剪放大）" },
                    "y": { "type": "integer" },
                    "width": { "type": "integer" },
                    "height": { "type": "integer" },
                    "scale": { "type": "integer" }
                },
                "required": ["question"]
            }),
            effect: None,
        }
    }

    async fn run(&self, _ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let question = required_string(&args, "question")?;
        let (png, surface) = capture_vision_png_with_region(&args).await?;
        let ignore_placeholder = args
            .get("ignore_placeholder")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let prompt = crate::vision::verification_prompt(&question, ignore_placeholder);
        let raw = crate::vision::describe_image(&png, &prompt).await?;
        let (answer, confidence) = crate::vision::parse_verification(&raw);
        let config = crate::vision::VisionConfig::from_env();
        Ok(json!({
            "surface": surface,
            "provider": config.provider,
            "model": config.model,
            "question": question,
            "answer": answer,
            "confidence": confidence,
            "raw": raw,
        }))
    }
}

pub struct VisionGroundTool;

#[async_trait]
impl Tool for VisionGroundTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "vision_ground".into(),
            description: "让视觉模型定位描述的元素（返回坐标框），并与 OCR 文本交叉验证；matched=true 且 cross_validated=true 时点击 line 中心；matched=true 且 vision_only=true（置信度≥0.9、无 OCR 文本的纯视觉元素，如图片表情/自绘按钮）时只能点击 box 中心；提供 app_id 时结果并入窗口元素注册表并返回稳定 element_id，后续 desktop_click 可直接用 element_id".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "description": { "type": "string", "description": "要定位的元素描述，例如“发送按钮”“输入框”" },
                    "app_id": { "type": "string", "description": "可选：应用标识（如 qq/weixin/notepad），提供时注册到窗口元素注册表" }
                },
                "required": ["description"]
            }),
            effect: None,
        }
    }

    async fn run(&self, ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let description = required_string(&args, "description")?;
        let app_id = args
            .get("app_id")
            .and_then(Value::as_str)
            .map(str::to_string);
        let mut result = crate::vision::ground_element(&description).await?;
        if let Some(app_id) = app_id {
            if result
                .get("matched")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                let grounding = vision_grounding_from_value(&result, &description)?;
                let mut registry = ctx
                    .elements
                    .lock()
                    .map_err(|_| "窗口元素注册表锁中毒".to_string())?;
                let element_id =
                    crate::register_vision_grounding(&mut registry, &app_id, grounding);
                if let Some(element_id) = element_id {
                    result["element_id"] = json!(element_id);
                    result["app_id"] = json!(app_id);
                }
            }
        }
        Ok(result)
    }
}

/// 从 vision_ground 返回值构造注册表输入（box=[x,y,w,h]）。
pub fn vision_grounding_from_value(
    value: &serde_json::Value,
    description: &str,
) -> Result<crate::VisionGrounding, String> {
    let r#box = value
        .get("box")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| "grounding 结果缺少 box".to_string())?;
    let numbers: Vec<i32> = r#box
        .iter()
        .filter_map(|item| item.as_i64().map(|value| value as i32))
        .collect();
    if numbers.len() != 4 {
        return Err("grounding 结果 box 格式错误".to_string());
    }
    Ok(crate::VisionGrounding {
        description: description.to_string(),
        x: numbers[0],
        y: numbers[1],
        width: numbers[2],
        height: numbers[3],
        confidence: value
            .get("confidence")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.7),
        cross_validated: value
            .get("cross_validated")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
    })
}
