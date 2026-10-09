// Adapted from yetone/magpie internal/sessions/claude_usage.go, claude.go,
// claude_usage_test.go and claude_cache_ttl_test.go at
// d1d4b7ed2a34cd3aac280ce0e69dfce2989264e9 (MIT). See THIRD_PARTY_NOTICES.md.
// Unlike the live upstream index, this bounded-file reader retains all identities
// for this invocation: eviction would double-count old non-adjacent replays.
use super::{ModelUsage, ParsedUsage, Tokens};
use crate::error::{AppError, AppResult};
use chrono::{DateTime, FixedOffset};
use serde_json::Value;
use std::collections::BTreeMap;

type Time = Option<DateTime<FixedOffset>>;
#[derive(Default)]
struct Contribution {
    latest: Option<(String, Tokens)>,
    previous: Vec<(String, Tokens)>,
    updated: Time,
}

fn warn(warnings: &mut Vec<String>, message: &str) {
    if !warnings.iter().any(|w| w == message) {
        warnings.push(message.to_owned());
    }
}
fn count(value: &Value, name: &str, warnings: &mut Vec<String>) -> Option<u64> {
    match value.get(name) {
        None => None,
        Some(v) => match v.as_u64() {
            Some(n) => Some(n),
            None => {
                warn(warnings, "Claude 存在无效 Token 字段；统计不完整。");
                None
            }
        },
    }
}
fn tokens(value: &Value, warnings: &mut Vec<String>) -> Option<Tokens> {
    if !value.is_object() {
        warn(warnings, "Claude 部分回复缺少用量记录；统计不完整。");
        return None;
    }
    let input = count(value, "input_tokens", warnings);
    let output = count(value, "output_tokens", warnings);
    if input.is_none() || output.is_none() {
        warn(warnings, "Claude 部分回复缺少输入或输出用量；统计不完整。");
    }
    let read = count(value, "cache_read_input_tokens", warnings);
    let write = count(value, "cache_creation_input_tokens", warnings);
    let split = value.get("cache_creation");
    let five = split.and_then(|v| count(v, "ephemeral_5m_input_tokens", warnings));
    let hour = split.and_then(|v| count(v, "ephemeral_1h_input_tokens", warnings));
    if input.is_none()
        && output.is_none()
        && read.is_none()
        && write.is_none()
        && five.is_none()
        && hour.is_none()
    {
        return None;
    }
    let Some(split_total) = five.unwrap_or(0).checked_add(hour.unwrap_or(0)) else {
        warn(warnings, "Claude 缓存 TTL 合计溢出；该用量记录未计入。");
        return None;
    };
    let cache_write = write.unwrap_or(split_total);
    if split.is_some() && split_total != cache_write {
        warn(
            warnings,
            "Claude 缓存写入总量与 TTL 明细不一致；费用估算可能不完整。",
        );
    }
    if split.is_none() && cache_write > 0 {
        warn(
            warnings,
            "Claude 缓存写入未记录 TTL；费用按 5 分钟缓存估算。",
        );
    }
    Some(Tokens {
        input: input.unwrap_or(0),
        output: output.unwrap_or(0),
        cache_read: read.unwrap_or(0),
        cache_write,
        cache_write_1h: hour.unwrap_or(0).min(cache_write),
        reasoning: 0,
    })
}

pub(super) fn parse(lines: impl Iterator<Item = AppResult<Value>>) -> AppResult<ParsedUsage> {
    let mut warnings = Vec::new();
    let mut messages: BTreeMap<String, BTreeMap<String, Contribution>> = BTreeMap::new();
    for (line_number, line) in lines.enumerate() {
        let line = line?;
        if line.get("type").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let message = &line["message"];
        let model = message
            .get("model")
            .and_then(Value::as_str)
            .filter(|m| !m.is_empty())
            .unwrap_or("(unknown)");
        if model == "<synthetic>" {
            continue;
        }
        let Some(usage) = tokens(&message["usage"], &mut warnings) else {
            continue;
        };
        let id = message.get("id").and_then(Value::as_str).unwrap_or("");
        let request = line.get("requestId").and_then(Value::as_str).unwrap_or("");
        let key = if !id.is_empty() {
            format!("message:{id}")
        } else if !request.is_empty() {
            format!("request:{request}")
        } else {
            warn(
                &mut warnings,
                "Claude 部分用量缺少消息和请求标识，无法确认是否为重放。",
            );
            format!("unidentified:{line_number}")
        };
        let branches = messages.entry(key).or_default();
        let branch_key = if branches.contains_key(request) {
            request.to_owned()
        } else if branches.len() == 1 && request.is_empty() {
            branches.keys().next().unwrap().clone()
        } else {
            if branches.len() == 1 && !request.is_empty() {
                if let Some(previous) = branches.remove("") {
                    branches.insert(request.to_owned(), previous);
                }
            }
            if request.is_empty() && branches.len() > 1 {
                warn(
                    &mut warnings,
                    "Claude 消息存在多个请求分支且部分请求标识缺失；统计可能重复。",
                );
            }
            request.to_owned()
        };
        let contribution = branches.entry(branch_key).or_default();
        let at = line
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok());
        let new = (model.to_owned(), usage);
        if let Some(old) = &contribution.latest {
            if *old == new {
                if at > contribution.updated {
                    contribution.updated = at;
                }
                continue;
            }
            if contribution.previous.contains(&new) {
                continue;
            }
            if at.is_none() || (contribution.updated.is_some() && at <= contribution.updated) {
                warn(
                    &mut warnings,
                    "Claude 存在无法确认顺序的用量修订，保留已确认记录。",
                );
                continue;
            }
            contribution.previous.push(old.clone());
        }
        contribution.latest = Some(new);
        contribution.updated = at;
    }
    let mut models: BTreeMap<String, Tokens> = BTreeMap::new();
    for contribution in messages.into_values().flat_map(BTreeMap::into_values) {
        if let Some((model, tokens)) = contribution.latest {
            let total = models.entry(model).or_default();
            macro_rules! add { ($field:ident) => { total.$field = total.$field.checked_add(tokens.$field).ok_or_else(|| AppError::Other("Claude Token 合计超出可表示范围".into()))?; }; }
            add!(input);
            add!(output);
            add!(cache_read);
            add!(cache_write);
            add!(cache_write_1h);
            add!(reasoning);
        }
    }
    if models.is_empty() {
        warn(
            &mut warnings,
            "未找到可用的 Claude 用量记录，不能视为零消耗。",
        );
    }
    Ok(ParsedUsage {
        models: models
            .into_iter()
            .map(|(model, tokens)| ModelUsage { model, tokens })
            .collect(),
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn line(second: u32, id: &str, request: &str, model: &str, output: u64) -> Value {
        json!({"type":"assistant","timestamp":format!("2026-09-20T12:00:{second:02}Z"),"requestId":request,"message":{"id":id,"model":model,"usage":{"input_tokens":10,"output_tokens":output,"cache_read_input_tokens":30,"cache_creation_input_tokens":0}}})
    }
    fn read(lines: Vec<Value>) -> ParsedUsage {
        parse(lines.into_iter().map(Ok)).unwrap()
    }
    #[test]
    fn non_adjacent_replay_revision_correction_and_old_snapshot() {
        // Adapted from Magpie TestClaudeUsageNonAdjacentReplay/RevisionAndTools.
        let got = read(vec![
            line(0, "a", "r1", "m", 1),
            line(1, "b", "r2", "m", 3),
            line(2, "a", "r1", "m", 7),
            line(3, "a", "r1", "m", 1),
            line(1, "a", "r1", "m", 99),
            line(4, "a", "r1", "m2", 5),
            line(5, "a", "r1", "m", 7),
        ]);
        assert_eq!(got.models.len(), 2);
        assert_eq!(got.models[0].tokens.output, 3);
        assert_eq!(got.models[1].model, "m2");
        assert_eq!(got.models[1].tokens.output, 5);
    }
    #[test]
    fn request_identity_branches_and_missing_identity() {
        // Adapted from Magpie TestClaudeUsageIdentity; unrelated message IDs do not merge.
        for (identities, calls) in [
            (vec![("a", ""), ("a", "r1"), ("a", "")], 1),
            (vec![("a", "r1"), ("a", "r2"), ("a", "r1")], 2),
            (vec![("a", "r1"), ("a", "r2"), ("a", "")], 3),
            (vec![("", "r1"), ("", "r1")], 1),
            (vec![("", ""), ("", "")], 2),
            (vec![("a", "r1"), ("b", "r1")], 2),
        ] {
            let got = read(
                identities
                    .into_iter()
                    .enumerate()
                    .map(|(i, (id, request))| line(i as u32, id, request, "m", 4))
                    .collect(),
            );
            assert_eq!(got.models[0].tokens.output, 4 * calls);
        }
    }
    #[test]
    fn cache_ttl_split_and_missing_total() {
        let mut a = line(0, "a", "r", "m", 2);
        a["message"]["usage"]["cache_creation_input_tokens"] = json!(3000);
        a["message"]["usage"]["cache_creation"] =
            json!({"ephemeral_5m_input_tokens":1000,"ephemeral_1h_input_tokens":2000});
        let mut b = line(1, "b", "s", "m", 2);
        b["message"]["usage"]
            .as_object_mut()
            .unwrap()
            .remove("cache_creation_input_tokens");
        b["message"]["usage"]["cache_creation"] = json!({"ephemeral_1h_input_tokens":40});
        let got = read(vec![a, b]);
        assert_eq!(got.models[0].tokens.cache_write, 3040);
        assert_eq!(got.models[0].tokens.cache_write_1h, 2040);
        assert_eq!(got.models[0].tokens.input, 20);
    }
    #[test]
    fn missing_invalid_synthetic_and_unordered_are_not_exact_zero() {
        let mut missing = line(0, "a", "r", "m", 2);
        missing["message"]["usage"] = Value::Null;
        let got = read(vec![missing, line(0, "s", "r", "<synthetic>", 100)]);
        assert!(got.models.is_empty());
        assert!(!got.warnings.is_empty());
        let mut invalid = line(0, "a", "r", "", 2);
        invalid["message"]["usage"]["input_tokens"] = json!(-1);
        let got = read(vec![invalid]);
        assert_eq!(got.models[0].model, "(unknown)");
        assert!(!got.warnings.is_empty());
        let mut late = line(2, "a", "r", "m", 8);
        late["timestamp"] = Value::Null;
        let got = read(vec![line(0, "a", "r", "m", 2), late]);
        assert_eq!(got.models[0].tokens.output, 2);
        assert!(!got.warnings.is_empty());
    }
    #[test]
    fn repeat_parse_append_and_cancellation() {
        let a = line(0, "a", "r", "m", 2);
        assert_eq!(read(vec![a.clone()]).models[0].tokens.output, 2);
        assert_eq!(
            read(vec![a.clone(), line(1, "a", "r", "m", 8)]).models[0]
                .tokens
                .output,
            8
        );
        assert!(matches!(
            parse(vec![Ok(a), Err(AppError::Cancelled)].into_iter()),
            Err(AppError::Cancelled)
        ));
    }
    #[test]
    fn sum_overflow_is_rejected() {
        assert!(parse(
            vec![
                Ok(line(0, "a", "r", "m", u64::MAX)),
                Ok(line(1, "b", "s", "m", 1))
            ]
            .into_iter()
        )
        .is_err());
    }
}
