//! V3 产品基准（50 任务）套件结构与静态校验回归。

use owo_agent_product_eval::product_eval::{
    load_suite, validate_suite, EvalCategory, SuiteBundle, FAULT_INJECTION_KINDS,
};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn v3_suite_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evals/v3-product/suite.json")
}

fn load_v3() -> SuiteBundle {
    let path = v3_suite_path();
    load_suite(&path)
        .unwrap_or_else(|error| panic!("加载 V3 套件失败（{}）：{}", path.display(), error))
}

#[test]
fn v3_product_suite_has_fifty_tasks_and_expected_category_counts() {
    let bundle = load_v3();
    assert_eq!(bundle.suite.name, "v3-product-50");
    assert_eq!(
        bundle.suite.defaults.repetitions, 3,
        "套件默认重复次数必须为 3"
    );
    assert_eq!(bundle.suite.tasks.len(), 50, "suite.tasks 必须恰好 50 项");
    assert_eq!(bundle.cases.len(), 50, "加载后任务数必须恰好 50");

    let count = |category: EvalCategory| {
        bundle
            .cases
            .iter()
            .filter(|case| case.category == category)
            .count()
    };
    assert_eq!(count(EvalCategory::Code), 15, "code 分类计数");
    assert_eq!(count(EvalCategory::Research), 10, "research 分类计数");
    assert_eq!(count(EvalCategory::Document), 10, "document 分类计数");
    assert_eq!(count(EvalCategory::Desktop), 10, "desktop 分类计数");
    assert_eq!(count(EvalCategory::LongHorizon), 5, "long_horizon 分类计数");

    let ids: BTreeSet<&str> = bundle.cases.iter().map(|case| case.id.as_str()).collect();
    assert_eq!(ids.len(), 50, "任务 id 必须唯一");

    assert_eq!(EvalCategory::Desktop.as_str(), "desktop");
    assert_eq!(EvalCategory::LongHorizon.as_str(), "long_horizon");
    assert_eq!(
        EvalCategory::parse("desktop").unwrap(),
        EvalCategory::Desktop
    );
    assert_eq!(
        EvalCategory::parse("long_horizon").unwrap(),
        EvalCategory::LongHorizon
    );
}

#[test]
fn v3_product_suite_passes_static_validation() {
    let bundle = load_v3();
    let validation = validate_suite(&bundle);
    let failed: Vec<_> = validation.tasks.iter().filter(|task| !task.ok).collect();
    assert!(validation.all_ok, "V3 套件静态校验失败：{failed:#?}");
    assert_eq!(validation.task_count, 50);
    assert_eq!(validation.ok_count, 50);
}

#[test]
fn heading_checkers_must_be_declared_in_the_instruction() {
    let mut bundle = load_v3();
    let case = bundle
        .cases
        .iter_mut()
        .find(|case| case.category == EvalCategory::Code)
        .expect("V3 套件必须包含 code 任务");
    case.instruction = case.instruction.replace("## 根因", "根因");
    let mut seen = BTreeSet::new();
    let issues = owo_agent_product_eval::product_eval::validate_case(case, &mut seen);
    assert!(
        issues.iter().any(|issue| issue.contains("## 根因")),
        "检查器要求的标题未写进 instruction 时必须静态报错：{issues:?}"
    );
}

#[test]
fn v3_every_task_declares_repetitions_and_metadata() {
    let bundle = load_v3();
    for case in &bundle.cases {
        assert_eq!(
            case.repetitions,
            Some(3),
            "任务 {} 必须显式声明 repetitions=3",
            case.id
        );
        assert!(
            !case.failure_classes.is_empty(),
            "任务 {} 必须填写 non-empty failure_classes",
            case.id
        );
        assert!(
            !case.expected_side_effects.is_empty(),
            "任务 {} 必须填写 non-empty expected_side_effects",
            case.id
        );
    }
}

#[test]
fn v3_long_horizon_tasks_cover_all_five_fault_kinds_uniquely() {
    let bundle = load_v3();
    let long_horizon: Vec<_> = bundle
        .cases
        .iter()
        .filter(|case| case.category == EvalCategory::LongHorizon)
        .collect();
    assert_eq!(long_horizon.len(), 5, "长程任务必须恰好 5 个");

    let mut kinds = BTreeSet::new();
    for case in &long_horizon {
        let fault = case
            .fault_injection
            .as_ref()
            .unwrap_or_else(|| panic!("长程任务 {} 必须声明 fault_injection", case.id));
        assert!(
            FAULT_INJECTION_KINDS.contains(&fault.kind.as_str()),
            "长程任务 {} 的 fault kind「{}」不在受支持集合",
            case.id,
            fault.kind
        );
        assert!(
            kinds.insert(fault.kind.clone()),
            "fault kind「{}」被多个长程任务重复使用",
            fault.kind
        );
    }

    let expected: BTreeSet<&str> = FAULT_INJECTION_KINDS.into_iter().collect();
    assert_eq!(
        kinds,
        expected.iter().map(|kind| (*kind).to_string()).collect(),
        "五个长程任务必须各覆盖一种 fault kind"
    );
}
