//! 语言选择链与 i18n 接线：`POC_LANG` → `LC_ALL` → `LC_MESSAGES` → `LANG` → `en`。
//! 消息目录在 locales/*.yml（en 为事实源），编译期嵌入二进制。

use rust_i18n::t;

/// 各二进制 main 入口调用一次：确定本次运行的 locale。
pub fn init() {
    rust_i18n::set_locale(&detect());
}

fn detect() -> String {
    for key in ["POC_LANG", "LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(v) = std::env::var(key) {
            let v = v.trim();
            if v.is_empty() || v == "C" || v == "POSIX" {
                continue;
            }
            return pick(normalize(v));
        }
    }
    "en".into()
}

/// "zh_CN.UTF-8" → "zh-CN"；无地区只留语言。
fn normalize(v: &str) -> String {
    let base = v.split('.').next().unwrap_or(v).replace('-', "_");
    let mut parts = base.splitn(2, '_');
    let lang = parts.next().unwrap_or("en").to_ascii_lowercase();
    match parts.next() {
        Some(region) if !region.is_empty() => format!("{lang}-{}", region.to_ascii_uppercase()),
        _ => lang,
    }
}

/// 精确命中可用目录；否则回退到同语言的变体（zh → zh-CN）；再否则 en。
fn pick(cand: String) -> String {
    let available = rust_i18n::available_locales!();
    if available.contains(&cand.as_str()) {
        return cand;
    }
    let lang = cand.split('-').next().unwrap_or("en");
    let mut variants: Vec<&str> = available
        .iter()
        .copied()
        .filter(|l| l.starts_with(&format!("{lang}-")))
        .collect();
    variants.sort_by_key(|l| l.len());
    if let Some(full) = variants.first() {
        return (*full).to_string();
    }
    "en".into()
}

/// 错误/用法前缀（cli wrap 用）：保持英文规范词形，前缀与正文同色渲染在 O2 接入。
pub fn error_prefix() -> String {
    t!("err.prefix").to_string()
}

pub fn usage_prefix() -> String {
    t!("err.usage_prefix").to_string()
}
