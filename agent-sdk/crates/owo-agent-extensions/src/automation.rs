//! 自动化（v0.4 P1）：定时任务/提醒/监控，全部经审计；桌面端常驻时生效。
//!
//! 支持提醒（Reminder）和只读 Agent 提示词（RunPrompt）；到期后写入运行台账并经审计。
//! 持久化：`<data>/automations.json`。

use chrono::{DateTime, FixedOffset, Local, NaiveTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Schedule {
    OneShot {
        at: String,
    },
    Interval {
        every_secs: u64,
    },
    /// 每天固定时刻触发。
    ///
    /// `timezone` 可选：`UTC`（默认）、`local`（跟随操作系统时区，夏令时由系统处理）、
    /// 或固定偏移 `+08:00` / `-0500`。按**当地日期**去重，所以 DST 当天不会重复触发，
    /// 也不会因为 UTC 时刻变化而漏掉当天。
    Daily {
        time: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timezone: Option<String>,
    },
}

/// 解析日程时区：默认 UTC；`local` 跟随系统当前偏移；其余必须是 `±HH:MM` / `±HHMM`。
fn schedule_offset(zone: Option<&str>, at: DateTime<Utc>) -> Option<FixedOffset> {
    let text = zone.unwrap_or("UTC").trim();
    if text.is_empty() || text.eq_ignore_ascii_case("utc") {
        return FixedOffset::east_opt(0);
    }
    if text.eq_ignore_ascii_case("local") {
        // 按该时刻在系统时区里的真实偏移取值：DST 前后各用各自的偏移。
        return Some(Local.offset_from_utc_datetime(&at.naive_utc()));
    }
    let (sign, rest) = match text.as_bytes().first() {
        Some(b'+') => (1, &text[1..]),
        Some(b'-') => (-1, &text[1..]),
        _ => return None,
    };
    let digits: String = rest.chars().filter(|ch| *ch != ':').collect();
    if digits.len() != 4 || !digits.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    let hours: i32 = digits[..2].parse().ok()?;
    let minutes: i32 = digits[2..].parse().ok()?;
    if hours > 23 || minutes > 59 {
        return None;
    }
    FixedOffset::east_opt(sign * (hours * 3600 + minutes * 60))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AutomationAction {
    /// 到点写入提醒列表（桌面端轮询展示）。
    Reminder { text: String },
    /// 到点让 Agent 执行一段提示词。
    ///
    /// 执行走**只读子代理**（`Agent::run_subagent(read_only = true)`）：定时任务是
    /// 无人值守场景，没有可以把审批卡送达的客户端——若放行写/执行，工具调用只会
    /// 阻塞在一个永远不会有人点的审批上，直到 300s 超时被拒。只读侧让"定时任务"
    /// 落在安全边界内（查询、汇总、巡检、日报这类），需要改动文件的操作请手动发起。
    RunPrompt { prompt: String },
}

/// 一次自动化执行记录（A8-1 取优合并自远端 engine；任务中心可查）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AutomationRun {
    pub task_id: String,
    pub task_name: String,
    pub at: String,
    /// `ok` / `failed` / `skipped`（并发上限触发时）
    pub status: String,
    #[serde(default)]
    pub output: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomationTask {
    pub id: String,
    pub name: String,
    pub schedule: Schedule,
    pub action: AutomationAction,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub last_run_at: Option<String>,
    #[serde(default)]
    pub created_at: String,
}

fn default_true() -> bool {
    true
}

impl AutomationTask {
    pub fn new(name: &str, schedule: Schedule, action: AutomationAction) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_string(),
            schedule,
            action,
            enabled: true,
            last_run_at: None,
            created_at: Utc::now().to_rfc3339(),
        }
    }

    fn is_due(&self, now: DateTime<Utc>) -> bool {
        if !self.enabled {
            return false;
        }
        match &self.schedule {
            Schedule::OneShot { at } => {
                self.last_run_at.is_none()
                    && DateTime::parse_from_rfc3339(at)
                        .map(|due| now >= due.with_timezone(&Utc))
                        .unwrap_or(false)
            }
            Schedule::Interval { every_secs } => {
                let created = DateTime::parse_from_rfc3339(&self.created_at)
                    .map(|value| value.with_timezone(&Utc))
                    .unwrap_or(now);
                let last = self
                    .last_run_at
                    .as_deref()
                    .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
                    .map(|value| value.with_timezone(&Utc));
                match last {
                    Some(last) => {
                        now.signed_duration_since(last).num_seconds() >= *every_secs as i64
                    }
                    None => now.signed_duration_since(created).num_seconds() >= *every_secs as i64,
                }
            }
            // 时间点由 daily_is_due 自行解析并校验，这里只需要时区。
            Schedule::Daily { time: _, timezone } => {
                let Some(offset) = schedule_offset(timezone.as_deref(), now) else {
                    return false;
                };
                self.daily_is_due(now, offset)
            }
        }
    }

    /// Daily 的当地日判断：同时用「评估时刻」和「上次触发时刻」各自的偏移，
    /// 这样跨 DST 的 23/25 小时日不会重复触发，也不会漏掉当天。
    fn daily_is_due(&self, now: DateTime<Utc>, offset: FixedOffset) -> bool {
        let time = match &self.schedule {
            Schedule::Daily { time, .. } => time,
            _ => return false,
        };
        let Ok(target) = NaiveTime::parse_from_str(time, "%H:%M") else {
            return false;
        };
        let local_now = now.with_timezone(&offset);
        let last_date = self
            .last_run_at
            .as_deref()
            .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.with_timezone(&offset).date_naive());
        last_date != Some(local_now.date_naive()) && local_now.time() >= target
    }

    /// 幂等键：同一次日程时段内的重复 tick、重复触发必须得到同一个键。
    ///
    /// 不能用墙钟时间做键——两次 tick 差一秒就会变成两个键。
    pub fn occurrence_key(&self, now: DateTime<Utc>) -> String {
        match &self.schedule {
            Schedule::OneShot { at } => format!("oneshot:{at}"),
            Schedule::Interval { every_secs } => {
                let every = (*every_secs).max(1) as i64;
                let created = DateTime::parse_from_rfc3339(&self.created_at)
                    .map(|value| value.with_timezone(&Utc))
                    .unwrap_or(now);
                let elapsed = now.signed_duration_since(created).num_seconds().max(0);
                format!("interval:{}", elapsed / every)
            }
            Schedule::Daily { time, timezone } => match schedule_offset(timezone.as_deref(), now) {
                Some(offset) => format!("daily:{}:{time}", now.with_timezone(&offset).date_naive()),
                None => format!("daily:invalid:{time}"),
            },
        }
    }
}

/// 执行记录封顶（超出丢弃最旧）。
const RUNS_LIMIT: usize = 500;

#[derive(Debug, Clone, Default)]
pub struct AutomationStore {
    root: PathBuf,
    tasks: HashMap<String, AutomationTask>,
    reminders: Vec<String>,
    runs: Vec<AutomationRun>,
}

impl AutomationStore {
    pub fn new(root: PathBuf) -> Self {
        let mut store = Self {
            root,
            tasks: HashMap::new(),
            reminders: Vec::new(),
            runs: Vec::new(),
        };
        store.load();
        store
    }

    fn path(&self) -> PathBuf {
        self.root.join("automations.json")
    }

    fn load(&mut self) {
        let Ok(content) = std::fs::read_to_string(self.path()) else {
            return;
        };
        if let Ok(data) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Ok(tasks) = serde_json::from_value::<Vec<AutomationTask>>(
                data.get("tasks").cloned().unwrap_or_default(),
            ) {
                for task in tasks {
                    self.tasks.insert(task.id.clone(), task);
                }
            }
            if let Ok(reminders) = serde_json::from_value::<Vec<String>>(
                data.get("reminders").cloned().unwrap_or_default(),
            ) {
                self.reminders = reminders;
            }
            if let Ok(runs) = serde_json::from_value::<Vec<AutomationRun>>(
                data.get("runs").cloned().unwrap_or_default(),
            ) {
                self.runs = runs;
            }
        }
    }

    pub fn save(&self) -> Result<(), String> {
        std::fs::create_dir_all(&self.root).map_err(|error| error.to_string())?;
        let mut tasks: Vec<AutomationTask> = self.tasks.values().cloned().collect();
        tasks.sort_by(|left, right| left.created_at.cmp(&right.created_at));
        let data = serde_json::json!({
            "tasks": tasks,
            "reminders": self.reminders,
            "runs": self.runs,
        });
        let content = serde_json::to_string_pretty(&data).map_err(|e| e.to_string())?;
        crate::atomic_write(&self.path(), content.as_bytes()).map_err(|error| error.to_string())
    }

    pub fn list(&self) -> Vec<AutomationTask> {
        let mut tasks: Vec<AutomationTask> = self.tasks.values().cloned().collect();
        tasks.sort_by(|left, right| left.created_at.cmp(&right.created_at));
        tasks
    }

    pub fn get(&self, id: &str) -> Option<&AutomationTask> {
        self.tasks.get(id)
    }

    pub fn upsert(&mut self, task: AutomationTask) -> Result<(), String> {
        self.tasks.insert(task.id.clone(), task);
        self.save()
    }

    pub fn remove(&mut self, id: &str) -> Result<(), String> {
        self.tasks.remove(id);
        self.save()
    }

    pub fn toggle(&mut self, id: &str) -> Result<bool, String> {
        let enabled = {
            let task = self
                .tasks
                .get_mut(id)
                .ok_or_else(|| format!("任务不存在：{id}"))?;
            task.enabled = !task.enabled;
            task.enabled
        };
        self.save()?;
        Ok(enabled)
    }

    pub fn due_tasks(&self, now: DateTime<Utc>) -> Vec<String> {
        self.tasks
            .values()
            .filter(|task| task.is_due(now))
            .map(|task| task.id.clone())
            .collect()
    }

    /// 触发任务：标记 last_run_at；提醒类追加到提醒列表；返回本次的动作载荷
    /// （提醒文本 / 待执行的提示词）。
    pub fn fire(&mut self, id: &str, now: DateTime<Utc>) -> Result<String, String> {
        let task = self
            .tasks
            .get_mut(id)
            .ok_or_else(|| format!("任务不存在：{id}"))?;
        task.last_run_at = Some(now.to_rfc3339());
        let (text, is_reminder) = match &task.action {
            AutomationAction::Reminder { text } => (text.clone(), true),
            // 跑任务的载荷就是提示词本身；它不进提醒列表（那是提醒专用通道）。
            AutomationAction::RunPrompt { prompt } => (prompt.clone(), false),
        };
        if is_reminder {
            self.reminders.push(text.clone());
            if self.reminders.len() > 200 {
                self.reminders.drain(..self.reminders.len() - 200);
            }
        }
        self.save()?;
        Ok(text)
    }

    /// A8-1：记录一次执行结果（任务中心可查；封顶 [`RUNS_LIMIT`] 条，丢最旧）。
    pub fn record_run(&mut self, run: AutomationRun) -> Result<(), String> {
        self.runs.push(run);
        if self.runs.len() > RUNS_LIMIT {
            let drop_count = self.runs.len() - RUNS_LIMIT;
            self.runs.drain(..drop_count);
        }
        self.save()
    }

    /// 执行记录（时间倒序；`task_id` 过滤可选）。
    pub fn runs(&self, task_id: Option<&str>, limit: usize) -> Vec<AutomationRun> {
        self.runs
            .iter()
            .rev()
            .filter(|run| task_id.map(|id| run.task_id == id).unwrap_or(true))
            .take(limit)
            .cloned()
            .collect()
    }

    pub fn reminders(&self) -> &[String] {
        &self.reminders
    }

    pub fn clear_reminders(&mut self) -> Result<(), String> {
        self.reminders.clear();
        self.save()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn root() -> PathBuf {
        std::env::temp_dir().join(format!("owo-automation-{}", uuid::Uuid::new_v4()))
    }

    #[test]
    fn interval_fires_after_elapsed() {
        let now = Utc.with_ymd_and_hms(2026, 8, 12, 12, 0, 0).unwrap();
        let mut task = AutomationTask::new(
            "check",
            Schedule::Interval { every_secs: 60 },
            AutomationAction::Reminder {
                text: "检查".to_string(),
            },
        );
        task.created_at = now.to_rfc3339();
        assert!(!task.is_due(now));
        let later = now + chrono::Duration::seconds(61);
        assert!(task.is_due(later));
        task.last_run_at = Some(later.to_rfc3339());
        assert!(!task.is_due(later));
    }

    #[test]
    fn one_shot_fires_once() {
        let now = Utc.with_ymd_and_hms(2026, 8, 12, 12, 0, 0).unwrap();
        let at = (now + chrono::Duration::seconds(10)).to_rfc3339();
        let task = AutomationTask::new(
            "once",
            Schedule::OneShot { at },
            AutomationAction::Reminder {
                text: "单次".to_string(),
            },
        );
        assert!(!task.is_due(now));
        let later = now + chrono::Duration::seconds(11);
        assert!(task.is_due(later));
        let mut fired = task.clone();
        fired.last_run_at = Some(later.to_rfc3339());
        assert!(!fired.is_due(later + chrono::Duration::seconds(60)));
    }

    #[test]
    fn daily_fires_once_per_day() {
        let now = Utc.with_ymd_and_hms(2026, 8, 12, 9, 0, 0).unwrap();
        let task = AutomationTask::new(
            "daily",
            Schedule::Daily {
                time: "08:00".to_string(),
                timezone: None,
            },
            AutomationAction::Reminder {
                text: "日报".to_string(),
            },
        );
        assert!(task.is_due(now));
        let mut fired = task.clone();
        fired.last_run_at = Some(now.to_rfc3339());
        assert!(!fired.is_due(now + chrono::Duration::hours(1)));
        let next_day = now + chrono::Duration::days(1);
        assert!(fired.is_due(next_day));
    }

    #[test]
    fn store_persists_and_fires() {
        let dir = root();
        let mut store = AutomationStore::new(dir.clone());
        let now = Utc::now();
        let mut task = AutomationTask::new(
            "remind-me",
            Schedule::Interval { every_secs: 1 },
            AutomationAction::Reminder {
                text: "休息一下".to_string(),
            },
        );
        task.created_at = (now - chrono::Duration::seconds(2)).to_rfc3339();
        store.upsert(task.clone()).unwrap();
        let due = store.due_tasks(now);
        assert_eq!(due.len(), 1);
        let text = store.fire(&due[0], now).unwrap();
        assert_eq!(text, "休息一下");
        assert_eq!(store.reminders().len(), 1);
        assert!(store.due_tasks(now).is_empty());

        let mut reloaded = AutomationStore::new(dir.clone());
        assert_eq!(reloaded.list().len(), 1);
        assert_eq!(reloaded.reminders().len(), 1);
        reloaded.clear_reminders().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn toggle_disables_task() {
        let dir = root();
        let mut store = AutomationStore::new(dir.clone());
        let now = Utc::now();
        let mut task = AutomationTask::new(
            "t",
            Schedule::Interval { every_secs: 1 },
            AutomationAction::Reminder {
                text: "x".to_string(),
            },
        );
        task.created_at = (now - chrono::Duration::seconds(5)).to_rfc3339();
        store.upsert(task).unwrap();
        assert_eq!(store.due_tasks(now).len(), 1);
        store.toggle(&store.list()[0].id).unwrap();
        assert!(store.due_tasks(now).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn daily_task(time: &str, timezone: Option<&str>) -> AutomationTask {
        AutomationTask::new(
            "daily",
            Schedule::Daily {
                time: time.to_string(),
                timezone: timezone.map(str::to_string),
            },
            AutomationAction::Reminder {
                text: "日报".to_string(),
            },
        )
    }

    #[test]
    fn daily_schedule_respects_configured_timezone() {
        let task = daily_task("09:00", Some("+08:00"));
        // UTC 01:00 = 当地 09:00，触发。
        assert!(task.is_due(Utc.with_ymd_and_hms(2026, 8, 12, 1, 0, 0).unwrap()));
        // UTC 00:59 还没到当地 09:00。
        assert!(!task.is_due(Utc.with_ymd_and_hms(2026, 8, 12, 0, 59, 0).unwrap()));
        // 默认 UTC 的任务在 UTC 09:00 才触发，证明时区参数真的生效。
        let utc_task = daily_task("09:00", None);
        assert!(!utc_task.is_due(Utc.with_ymd_and_hms(2026, 8, 12, 1, 0, 0).unwrap()));
        assert!(utc_task.is_due(Utc.with_ymd_and_hms(2026, 8, 12, 9, 0, 0).unwrap()));
    }

    #[test]
    fn daily_dedupes_by_local_date_across_dst_offsets() {
        let plus_one = FixedOffset::east_opt(3600).unwrap();
        let plus_two = FixedOffset::east_opt(7200).unwrap();
        let task = daily_task("08:30", None);
        // 3 月 28 日 08:30（+01:00）。
        let before_dst = Utc.with_ymd_and_hms(2026, 3, 28, 7, 30, 0).unwrap();
        assert!(task.daily_is_due(before_dst, plus_one));

        let mut fired = task.clone();
        fired.last_run_at = Some(before_dst.to_rfc3339());
        // 同一当地日期，偏移已切到 +02:00（07:30Z = 09:30 当地）：不得重复触发。
        assert!(!fired.daily_is_due(before_dst, plus_two));
        // 3 月 29 日 08:30（+02:00）= 06:30Z：新的一天，必须触发。
        let after_dst = Utc.with_ymd_and_hms(2026, 3, 29, 6, 30, 0).unwrap();
        assert!(fired.daily_is_due(after_dst, plus_two));
        // 但当天 06:30Z 之后再评估也不再重复。
        let mut done = fired.clone();
        done.last_run_at = Some(after_dst.to_rfc3339());
        assert!(!done.daily_is_due(after_dst + chrono::Duration::minutes(30), plus_two));
    }

    #[test]
    fn occurrence_key_is_stable_within_one_slot() {
        let daily = daily_task("09:00", Some("+08:00"));
        let first = Utc.with_ymd_and_hms(2026, 8, 12, 1, 0, 5).unwrap();
        let second = Utc.with_ymd_and_hms(2026, 8, 12, 1, 0, 55).unwrap();
        assert_eq!(daily.occurrence_key(first), daily.occurrence_key(second));
        assert_ne!(
            daily.occurrence_key(first),
            daily.occurrence_key(first + chrono::Duration::days(1))
        );

        let mut interval = AutomationTask::new(
            "interval",
            Schedule::Interval { every_secs: 10 },
            AutomationAction::Reminder {
                text: "巡检".to_string(),
            },
        );
        interval.created_at = first.to_rfc3339();
        assert_eq!(
            interval.occurrence_key(first + chrono::Duration::seconds(3)),
            interval.occurrence_key(first + chrono::Duration::seconds(9))
        );
        assert_ne!(
            interval.occurrence_key(first + chrono::Duration::seconds(9)),
            interval.occurrence_key(first + chrono::Duration::seconds(14))
        );

        let one_shot = AutomationTask::new(
            "once",
            Schedule::OneShot {
                at: "2026-08-12T09:00:00Z".to_string(),
            },
            AutomationAction::Reminder {
                text: "一次".to_string(),
            },
        );
        assert_eq!(
            one_shot.occurrence_key(first),
            "oneshot:2026-08-12T09:00:00Z"
        );
    }

    #[test]
    fn invalid_timezone_is_rejected_instead_of_guessed() {
        let task = daily_task("09:00", Some("Mars/Olympus"));
        assert!(!task.is_due(Utc.with_ymd_and_hms(2026, 8, 12, 9, 0, 0).unwrap()));
        assert!(task
            .occurrence_key(Utc.with_ymd_and_hms(2026, 8, 12, 9, 0, 0).unwrap())
            .starts_with("daily:invalid:"));
    }
}
