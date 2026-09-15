use serde::{Deserialize, Serialize};

/// 审计日志条目（仅记录操作元数据，绝不写入密钥/密码）。
/// 日志以加密文件形式保存在保险箱内（remote: audit.log），由 VaultService 读写。
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AuditEntry {
    pub ts: String,
    pub username: String,
    pub action: String,
    pub detail: String,
}

const MAX_LOG_LINES: usize = 500;

/// 构造一条新的 JSONL 明文日志行
pub fn new_line(username: &str, action: &str, detail: &str) -> String {
    serde_json::json!({
        "ts": iso_now(),
        "username": username,
        "action": action,
        "detail": detail,
    })
    .to_string()
}

/// 在既有日志内容上追加一行，并裁剪到上限（保留最新的 MAX_LOG_LINES 行）
pub fn append_line(content: &str, line: &str) -> String {
    let mut lines: Vec<&str> = content.lines().collect();
    lines.push(line);
    trim_lines(&mut lines)
}

/// 将日志内容裁剪到上限（保留尾部最新行）
pub fn trim_content(content: &str) -> String {
    let mut lines: Vec<&str> = content.lines().collect();
    trim_lines(&mut lines)
}

fn trim_lines(lines: &mut Vec<&str>) -> String {
    if lines.len() > MAX_LOG_LINES {
        lines.drain(0..(lines.len() - MAX_LOG_LINES));
    }
    lines.join("\n")
}

/// 解析日志内容，返回最近 `limit` 条（按时间正序）
pub fn read(content: &str, limit: usize) -> Vec<AuditEntry> {
    let mut entries: Vec<AuditEntry> = content
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    if entries.len() > limit {
        entries.drain(0..(entries.len() - limit));
    }
    entries
}

fn iso_now() -> String {
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // 使用 civil_from_days 算法将 Unix 秒精确转换为公历日期时间 (UTC)
    let days = (d / 86400) as i64;
    let secs_of_day = d % 86400;
    let (y, m, day) = civil_from_days(days);
    let h = secs_of_day / 3600;
    let min = (secs_of_day % 3600) / 60;
    let s = secs_of_day % 60;
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, m, day, h, min, s)
}

/// Howard Hinnant 的 civil_from_days 算法：将“自 1970-01-01 的天数”转换为 (年, 月, 日)
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}
