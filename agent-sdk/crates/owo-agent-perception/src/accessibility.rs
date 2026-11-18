//! L1 界面层（v0.4 P2）：Windows UI Automation 无障碍 UI 树摘要。
//!
//! 只取语义锚点（角色 + 名称 + 类名），不包含坐标序列；供情景模型与操作学习使用。
use serde::{Deserialize, Serialize};

#[cfg(target_os = "windows")]
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
    COINIT_DISABLE_OLE1DDE,
};
#[cfg(target_os = "windows")]
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, TreeScope_Children,
};
#[cfg(target_os = "windows")]
use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiNode {
    pub name: String,
    pub control_type: i32,
    pub class: String,
    pub depth: u32,
    /// 元素在屏幕上的边界框（OCR/坐标点击定位用）。
    #[serde(default)]
    pub x: i32,
    #[serde(default)]
    pub y: i32,
    #[serde(default)]
    pub width: i32,
    #[serde(default)]
    pub height: i32,
    /// 仅由实时 UIA 树填充的安全属性；历史/合成节点为 None。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_safety: Option<UiValueSafety>,
}

/// `ValuePattern` 写值前必须确认的控件身份与可写属性。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiValueSafety {
    pub is_password: bool,
    pub is_read_only: bool,
    pub is_offscreen: bool,
    pub is_enabled: bool,
    pub has_keyboard_focus: bool,
    pub has_value_pattern: bool,
    /// Native UIA identity used to rebind the observation at write time.
    pub automation_id: String,
    pub process_id: u32,
    pub native_window_handle: i64,
}

/// 匹配 `uia:` 锚点的候选控件必须可访问、可写且不是密码框。
/// `focus_required` 仅供 `set_value`：写值前焦点必须仍在被批准的控件上。
pub fn safe_value_target(node: &UiNode, name: &str, focus_required: bool) -> bool {
    let Some(safety) = node.value_safety.as_ref() else {
        return false;
    };
    node.name.trim() == name
        && node.width > 0
        && node.height > 0
        && node.control_type == 50004 // UIA_EditControlTypeId
        && safety.has_value_pattern
        && !safety.is_password
        && !safety.is_read_only
        && !safety.is_offscreen
        && safety.is_enabled
        && (!focus_required || safety.has_keyboard_focus)
}

/// 抓取前台窗口的无障碍 UI 树摘要（Windows UI Automation）。
#[cfg(target_os = "windows")]
pub fn foreground_ui_tree(max_depth: u32, max_nodes: usize) -> Option<Vec<UiNode>> {
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.0.is_null() {
        return None;
    }
    ui_tree_for_hwnd(hwnd.0 as isize, max_depth, max_nodes)
}

/// `uia:` 锚点的受控观察：把命中的名字、角色、边界、ValuePattern 与窗口句柄
/// 合成和 OCR 同构的 lines，并给出不依赖 OCR 文本的稳定观察引用。
#[cfg(target_os = "windows")]
pub fn accessible_anchor_observation(hwnd: isize, name: &str) -> Option<serde_json::Value> {
    use serde_json::json;
    let nodes = ui_tree_for_hwnd(hwnd, 8, 160).unwrap_or_default();
    let matches: Vec<&UiNode> = nodes
        .iter()
        .filter(|node| safe_value_target(node, name, false))
        .collect();
    if matches.len() != 1 {
        return None;
    }
    let lines: Vec<serde_json::Value> = matches
        .iter()
        .map(|node| {
            json!({
                "text": node.name,
                "x": node.x,
                "y": node.y,
                "width": node.width,
                "height": node.height,
                "role_hint": "edit",
                "control_type": node.control_type,
                "has_value_pattern": node
                    .value_safety
                    .as_ref()
                    .map(|v| v.has_value_pattern)
                    .unwrap_or(false),
                "has_keyboard_focus": node
                    .value_safety
                    .as_ref()
                    .map(|v| v.has_keyboard_focus)
                    .unwrap_or(false),
                "automation_id": node
                    .value_safety
                    .as_ref()
                    .map(|v| v.automation_id.as_str())
                    .unwrap_or(""),
                "process_id": node.value_safety.as_ref().map(|v| v.process_id).unwrap_or(0),
                "native_window_handle": node
                    .value_safety
                    .as_ref()
                    .map(|v| v.native_window_handle)
                    .unwrap_or(0),
                "hwnd": hwnd,
            })
        })
        .collect();
    let mut digest = sha2::Sha256::new();
    use sha2::Digest;
    digest.update(hwnd.to_le_bytes());
    digest.update(name.as_bytes());
    for line in &lines {
        digest.update(line.to_string().as_bytes());
    }
    Some(json!({
        "surface": "uia",
        "text": name,
        "lines": lines,
        "observation_ref": format!("sha256:{:x}", digest.finalize()),
    }))
}

#[cfg(not(target_os = "windows"))]
pub fn accessible_anchor_observation(_hwnd: isize, _name: &str) -> Option<serde_json::Value> {
    None
}

/// 按窗口句柄抓取无障碍 UI 树（不要求前台，窗口存在即可，用于窗口模板/后台情景理解）。
#[cfg(target_os = "windows")]
pub fn ui_tree_for_hwnd(hwnd: isize, max_depth: u32, max_nodes: usize) -> Option<Vec<UiNode>> {
    unsafe {
        // COM 已在 MTA 时忽略错误，UIA 仍可工作。
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
        let automation: IUIAutomation =
            CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok()?;
        let root = automation
            .ElementFromHandle(windows::Win32::Foundation::HWND(
                hwnd as *mut core::ffi::c_void,
            ))
            .ok()?;
        let mut nodes = Vec::new();
        collect_ui(&automation, &root, 0, max_depth, max_nodes, &mut nodes);
        if nodes.is_empty() {
            None
        } else {
            Some(nodes)
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub fn ui_tree_for_hwnd(_hwnd: isize, _max_depth: u32, _max_nodes: usize) -> Option<Vec<UiNode>> {
    None
}

#[cfg(target_os = "windows")]
fn collect_ui(
    automation: &IUIAutomation,
    element: &IUIAutomationElement,
    depth: u32,
    max_depth: u32,
    max_nodes: usize,
    out: &mut Vec<UiNode>,
) {
    unsafe {
        if out.len() >= max_nodes {
            return;
        }
        let name = element
            .CurrentName()
            .map(|value| value.to_string())
            .unwrap_or_default();
        let class = element
            .CurrentClassName()
            .map(|value| value.to_string())
            .unwrap_or_default();
        let control_type = element
            .CurrentControlType()
            .map(|value| value.0)
            .unwrap_or(0);
        let rect = element
            .CurrentBoundingRectangle()
            .map(|rect| {
                (
                    rect.left,
                    rect.top,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                )
            })
            .unwrap_or((0, 0, 0, 0));
        if !name.trim().is_empty() || !class.trim().is_empty() {
            let is_password = element
                .CurrentIsPassword()
                .map(|value| value.as_bool())
                .unwrap_or(true);
            let is_enabled = element
                .CurrentIsEnabled()
                .map(|value| value.as_bool())
                .unwrap_or(false);
            let is_offscreen = element
                .CurrentIsOffscreen()
                .map(|value| value.as_bool())
                .unwrap_or(true);
            let has_keyboard_focus = element
                .CurrentHasKeyboardFocus()
                .map(|value| value.as_bool())
                .unwrap_or(false);
            let (is_read_only, has_value_pattern) = element
                .GetCurrentPatternAs::<
                    windows::Win32::UI::Accessibility::IUIAutomationValuePattern,
                >(windows::Win32::UI::Accessibility::UIA_ValuePatternId)
                .map(|pattern| {
                    (
                        pattern
                            .CurrentIsReadOnly()
                            .map(|value| value.as_bool())
                            .unwrap_or(true),
                        true,
                    )
                })
                .unwrap_or((false, false));
            out.push(UiNode {
                name,
                control_type,
                class,
                depth,
                x: rect.0,
                y: rect.1,
                width: rect.2,
                height: rect.3,
                value_safety: Some(UiValueSafety {
                    is_password,
                    is_enabled,
                    is_read_only,
                    is_offscreen,
                    has_keyboard_focus,
                    has_value_pattern,
                    automation_id: element
                        .CurrentAutomationId()
                        .map(|value| value.to_string())
                        .unwrap_or_default(),
                    process_id: element.CurrentProcessId().unwrap_or(0) as u32,
                    native_window_handle: element.CurrentNativeWindowHandle().unwrap_or_default().0
                        as i64,
                }),
            });
        }
        if depth >= max_depth {
            return;
        }
        let Ok(condition) = automation.CreateTrueCondition() else {
            return;
        };
        let Ok(children) = element.FindAll(TreeScope_Children, &condition) else {
            return;
        };
        let Ok(length) = children.Length() else {
            return;
        };
        for index in 0..length {
            if out.len() >= max_nodes {
                return;
            }
            if let Ok(child) = children.GetElement(index) {
                collect_ui(automation, &child, depth + 1, max_depth, max_nodes, out);
            }
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub fn foreground_ui_tree(_max_depth: u32, _max_nodes: usize) -> Option<Vec<UiNode>> {
    None
}

/// UIA-only observation is safe when the full screen OCR is empty but a unique,
/// bound, non-password Edit exposes ValuePattern; scan_ui_sensitive stays active.
///
/// This is a pure contract helper for tests and callers that inspect the shape of
/// an observation. The platform implementation is `accessible_anchor_observation`.
pub fn uia_observation_is_safe(ocr: &serde_json::Value, observation: &serde_json::Value) -> bool {
    let ocr_empty = ocr
        .get("lines")
        .and_then(serde_json::Value::as_array)
        .map(|lines| lines.is_empty())
        .unwrap_or(false);
    let bound_unique_edit = observation
        .get("lines")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|lines| {
            lines.len() == 1
                && lines.iter().all(|line| {
                    line.get("has_value_pattern")
                        .and_then(serde_json::Value::as_bool)
                        == Some(true)
                        && line.get("control_type").and_then(serde_json::Value::as_i64)
                            == Some(50004)
                        && line
                            .get("hwnd")
                            .and_then(serde_json::Value::as_i64)
                            .is_some()
                        && line
                            .get("automation_id")
                            .and_then(serde_json::Value::as_str)
                            .is_some_and(|id| !id.is_empty())
                        && line
                            .get("process_id")
                            .and_then(serde_json::Value::as_u64)
                            .is_some_and(|pid| pid > 0)
                })
        });
    ocr_empty && bound_unique_edit
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_tree_poll_is_callable() {
        // 交互会话返回前台应用的无障碍树；无窗口/不可用返回 None（不强制断言）。
        let _ = foreground_ui_tree(2, 32);
    }

    fn editable(name: &str) -> UiNode {
        UiNode {
            name: name.to_string(),
            control_type: 50004,
            class: "Edit".into(),
            depth: 2,
            x: 10,
            y: 20,
            width: 100,
            height: 24,
            value_safety: Some(UiValueSafety {
                is_password: false,
                is_enabled: true,
                is_read_only: false,
                is_offscreen: false,
                has_keyboard_focus: true,
                has_value_pattern: true,
                automation_id: "File name".into(),
                process_id: 42,
                native_window_handle: 0,
            }),
        }
    }

    #[test]
    fn safe_value_target_accepts_a_focused_bound_edit() {
        assert!(safe_value_target(&editable("文件名"), "文件名", true));
    }

    #[test]
    fn uia_observation_requires_empty_ocr_and_one_bound_value_edit() {
        use serde_json::json;
        let empty_ocr = json!({"lines": []});
        let observation = json!({"lines": [{
            "control_type": 50004,
            "has_value_pattern": true,
            "hwnd": 7,
            "automation_id": "File name",
            "process_id": 42,
        }]});
        assert!(uia_observation_is_safe(&empty_ocr, &observation));
        assert!(!uia_observation_is_safe(
            &empty_ocr,
            &json!({"lines": [{
                "control_type": 50004,
                "has_value_pattern": true,
                "hwnd": 7,
            }]})
        ));
        assert!(!uia_observation_is_safe(
            &json!({"lines": [{"text": "可见文字"}]}),
            &observation
        ));
        assert!(!uia_observation_is_safe(
            &empty_ocr,
            &json!({"lines": [
                {"control_type": 50004, "has_value_pattern": true, "hwnd": 7},
                {"control_type": 50004, "has_value_pattern": true, "hwnd": 7}
            ]})
        ));
    }

    #[test]
    fn serde_defaults_keep_legacy_ui_nodes_enabled_but_not_value_targets() {
        let node: UiNode = serde_json::from_value(serde_json::json!({
            "name": "文件名",
            "control_type": 50004,
            "class": "Edit",
            "depth": 1,
            "x": 10,
            "y": 20,
            "width": 100,
            "height": 24,
        }))
        .unwrap();
        assert!(node.value_safety.is_none());
        assert!(!safe_value_target(&node, "文件名", false));
    }

    #[test]
    fn safe_value_target_refuses_password_disabled_or_wrong_controls() {
        let mut node = editable("文件名");
        node.value_safety.as_mut().unwrap().is_password = true;
        assert!(!safe_value_target(&node, "文件名", false));
        let mut node = editable("文件名");
        node.value_safety.as_mut().unwrap().is_enabled = false;
        assert!(!safe_value_target(&node, "文件名", false));
        let mut node = editable("文件名");
        node.value_safety.as_mut().unwrap().is_read_only = true;
        assert!(!safe_value_target(&node, "文件名", false));
        let mut node = editable("文件名");
        node.value_safety.as_mut().unwrap().is_offscreen = true;
        assert!(!safe_value_target(&node, "文件名", false));
        let mut node = editable("文件名");
        node.value_safety.as_mut().unwrap().has_value_pattern = false;
        assert!(!safe_value_target(&node, "文件名", false));
        let mut node = editable("文件名");
        node.control_type = 50000; // Button
        assert!(!safe_value_target(&node, "文件名", false));
        assert!(!safe_value_target(&editable("文件名"), "另一个控件", false));
        let mut node = editable("文件名");
        node.value_safety.as_mut().unwrap().has_keyboard_focus = false;
        assert!(!safe_value_target(&node, "文件名", true));
    }
}
