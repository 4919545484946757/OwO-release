use serde_json::{json, Value};
pub(super) fn sim_base_url() -> Option<String> {
    std::env::var("OWO_SIM_QQ_URL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// 是否配置了模拟面（供服务端接线选择 SimTaskSurface / RealTaskSurface）。
pub fn sim_base_url_configured() -> bool {
    sim_base_url().is_some()
}

pub(super) fn on_sim_surface() -> bool {
    sim_base_url().is_some()
}

pub(super) async fn sim_fetch_frame() -> Result<Vec<u8>, String> {
    let base = sim_base_url().ok_or("模拟环境未配置 OWO_SIM_QQ_URL")?;
    let url = format!("{}/frame", base.trim_end_matches('/'));
    let response = reqwest::get(&url)
        .await
        .map_err(|e| format!("模拟窗口截图失败：{e}"))?;
    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("模拟窗口截图读取失败：{e}"))?
        .to_vec();
    if bytes.len() < 54 || &bytes[..2] != b"BM" {
        return Err("模拟窗口返回的不是 BMP".to_string());
    }
    Ok(bytes)
}

/// 模拟面真值版面（优先）：模拟服务知道每个控件的文字与位置，直接返回
/// 与 screen_ocr 同构的 lines，避免离屏渲染 + Media.Ocr 的小字识别问题。
pub(super) async fn sim_ocr_lines() -> Option<Value> {
    let base = sim_base_url()?;
    let url = format!("{}/ocr", base.trim_end_matches('/'));
    let response = reqwest::get(&url).await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let value: Value = response.json().await.ok()?;
    let has_lines = value
        .get("lines")
        .and_then(Value::as_array)
        .map(|lines| !lines.is_empty())
        .unwrap_or(false);
    if !has_lines {
        return None;
    }
    Some(value)
}

/// 向指定模拟服务地址 POST JSON（同 `sim_post`，但 base 由调用方给定）。
pub(super) async fn sim_post_at(base: &str, path: &str, body: Value) -> Result<Value, String> {
    let url = format!(
        "{}/{}",
        base.trim_end_matches('/'),
        path.trim_start_matches('/')
    );
    let client = reqwest::Client::new();
    let response = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("模拟窗口 {path} 失败：{e}"))?;
    response
        .json::<Value>()
        .await
        .map_err(|e| format!("模拟窗口 {path} 响应解析失败：{e}"))
}

pub(super) async fn sim_post(path: &str, body: Value) -> Result<Value, String> {
    let base = sim_base_url().ok_or("模拟环境未配置 OWO_SIM_QQ_URL")?;
    let url = format!(
        "{}/{}",
        base.trim_end_matches('/'),
        path.trim_start_matches('/')
    );
    let client = reqwest::Client::new();
    let response = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("模拟窗口 {path} 失败：{e}"))?;
    response
        .json::<Value>()
        .await
        .map_err(|e| format!("模拟窗口 {path} 响应解析失败：{e}"))
}

pub(super) fn ocr_summary_json(summary: &crate::ocr::OcrSummary, max_boxes: usize) -> Value {
    // 控制传给模型的上下文体积：超大 OCR 结果会让多轮工具调用不稳定。
    let max_lines = 60usize;
    let max_boxes = max_boxes.min(80);
    let text: String = summary.text.chars().take(2000).collect();
    let lines: Vec<Value> = crate::ocr::group_ocr_lines(&summary.boxes)
        .into_iter()
        .take(max_lines)
        .map(|line| {
            let role_hint = if line.text.contains("发送")
                || line.text.contains("搜索")
                || line.text.contains("提交")
            {
                "button"
            } else if line.text.contains("输入")
                || line.text.contains("搜索")
                || line.text.contains("请输入")
            {
                "input"
            } else if line.y < 60 {
                "header"
            } else {
                "text"
            };
            json!({
                "text": line.text,
                "x": line.x,
                "y": line.y,
                "width": line.width,
                "height": line.height,
                "role_hint": role_hint,
            })
        })
        .collect();
    let boxes: Vec<Value> = summary
        .boxes
        .iter()
        .take(max_boxes)
        .map(|b| {
            json!({
                "text": b.text,
                "x": b.x,
                "y": b.y,
                "width": b.width,
                "height": b.height,
            })
        })
        .collect();
    json!({
        "text": text,
        "chars": summary.chars,
        "lines": lines,
        "boxes": boxes,
        "box_count": summary.boxes.len(),
    })
}

/// 统一 OCR 入口（模拟面走真值版面，真实面走 Media.Ocr），返回 screen_ocr 同构 JSON。
pub(super) async fn ocr_screen(max_boxes: usize) -> Result<Value, String> {
    if on_sim_surface() {
        if let Some(mut result) = sim_ocr_lines().await {
            if let Value::Object(map) = &mut result {
                map.insert("surface".into(), json!("sim"));
            }
            return Ok(result);
        }
    }
    let (bmp, surface) = if on_sim_surface() {
        (sim_fetch_frame().await?, "sim")
    } else {
        (
            crate::platform::capture_screen().ok_or("屏幕截图失败")?,
            "desktop",
        )
    };
    let summary = crate::paddle_ocr::ocr_preferred(&bmp)
        .await
        .map_err(|e| format!("OCR 失败：{e}"))?;
    let mut result = ocr_summary_json(&summary, max_boxes);
    if let Value::Object(map) = &mut result {
        map.insert("surface".into(), json!(surface));
    }
    Ok(result)
}

/// 在 OCR lines 中查找包含目标文本的所有行（可带 role_hint 过滤）。
///
/// 保留全量候选而不是只取第一个：动作定位需要判断"锚点是否歧义"，
/// 命中多个候选时不得静默选择第一个（Agent V2 §5 任务 1.4）。
pub(super) fn find_ocr_lines(ocr: &Value, needle: &str, role: &str) -> Vec<Value> {
    let needle_lower = needle.to_lowercase();
    let Some(lines) = ocr.get("lines").and_then(Value::as_array) else {
        return Vec::new();
    };
    lines
        .iter()
        .filter(|line| {
            let text = line
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_lowercase();
            let role_ok = role.is_empty()
                || line
                    .get("role_hint")
                    .and_then(Value::as_str)
                    .map(|line_role| line_role == role)
                    .unwrap_or(false);
            role_ok && text.contains(&needle_lower)
        })
        .cloned()
        .collect()
}

/// 在 OCR lines 中查找包含目标文本的第一行（单结果调用方的便捷入口）。
pub(super) fn find_ocr_line(ocr: &Value, needle: &str, role: &str) -> Option<Value> {
    find_ocr_lines(ocr, needle, role).into_iter().next()
}
