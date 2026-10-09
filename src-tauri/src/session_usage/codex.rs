// Usage reconciliation adapted from yetone/magpie internal/sessions/codex_usage.go,
// commit d1d4b7ed2a34cd3aac280ce0e69dfce2989264e9 (MIT, Copyright 2026 yetone).
// See THIRD_PARTY_NOTICES.md. No session text or upstream runtime is retained.
use super::{ModelUsage, ParsedUsage, Tokens};
use crate::error::{AppError, AppResult};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
struct Usage([u64; 5]);

impl Usage {
    fn read(value: &Value) -> Option<Self> {
        let mut result = [0; 5];
        for (index, name) in [
            "input_tokens",
            "output_tokens",
            "cached_input_tokens",
            "cache_write_input_tokens",
            "reasoning_output_tokens",
        ]
        .iter()
        .enumerate()
        {
            result[index] = match value.get(name) {
                None | Some(Value::Null) if index >= 2 => 0,
                Some(number) => number.as_u64()?,
                _ => return None,
            };
        }
        let usage = Self(result);
        usage.valid().then_some(usage)
    }

    fn valid(self) -> bool {
        self.0[2]
            .checked_add(self.0[3])
            .is_some_and(|cached| cached <= self.0[0])
            && self.0[4] <= self.0[1]
    }

    fn difference(self, previous: Self) -> Option<Self> {
        let mut difference = [0; 5];
        for (index, value) in difference.iter_mut().enumerate() {
            *value = self.0[index].checked_sub(previous.0[index])?;
        }
        Some(Self(difference))
    }

    fn plus(self, other: Self) -> Option<Self> {
        let mut sum = [0; 5];
        for (index, value) in sum.iter_mut().enumerate() {
            *value = self.0[index].checked_add(other.0[index])?;
        }
        Some(Self(sum))
    }
}

#[derive(Clone)]
struct Contribution {
    session: String,
    turn: String,
    model: String,
    response: String,
    timestamp: String,
    usage: Usage,
    // Legacy checkpoints and thread checkpoints are different domains after compaction.
    legacy_total: Option<Usage>,
    thread_total: Option<Usage>,
    epoch: usize,
    compaction: bool,
}

#[derive(Default)]
struct State {
    session: String,
    turn: String,
    model: String,
    models: HashMap<String, String>,
    high: Option<Usage>,
    epoch: usize,
    pending: Option<usize>,
    entries: Vec<Contribution>,
    responses: HashMap<(String, String), usize>,
    // Timestamp is part of the identity: equal amounts in distinct calls must survive.
    observed: HashSet<(String, String, Usage, Option<Usage>)>,
    warnings: Vec<String>,
}

fn text<'a>(value: &'a Value, field: &str) -> &'a str {
    value.get(field).and_then(Value::as_str).unwrap_or("")
}

impl State {
    fn warn(&mut self, message: &str) {
        if !self.warnings.iter().any(|warning| warning == message) {
            self.warnings.push(message.to_owned());
        }
    }

    fn model(&self, turn: &str) -> String {
        self.models
            .get(turn)
            .cloned()
            .unwrap_or_else(|| self.model.clone())
    }

    fn count(&mut self, at: &str, info: &Value) {
        let total = Usage::read(&info["total_token_usage"]);
        let last = Usage::read(&info["last_token_usage"]);
        let Some(total) = total else {
            self.warn("Codex 存在缺失或无效的累计用量，该条记录未计入。");
            return;
        };
        if !info["last_token_usage"].is_null() && last.is_none() {
            self.warn("Codex 单次用量无效，已退回累计差值。");
        }
        // A replay with its original observation time cannot restart the runtime counter.
        if !at.is_empty()
            && !self
                .observed
                .insert((self.session.clone(), at.to_owned(), total, last))
        {
            return;
        }
        if self.high == Some(total) {
            return;
        }
        // Pair the response and legacy observation in either order. Input/turn boundaries
        // clear pending, so numerically identical independent requests remain distinct.
        if let (Some(index), Some(last)) = (self.pending.take(), last) {
            let old = &mut self.entries[index];
            if !old.response.is_empty()
                && !old.compaction
                && old.legacy_total.is_none()
                && old.session == self.session
                && old.turn == self.turn
                && old.usage == last
            {
                old.legacy_total = Some(total);
                self.high = Some(total);
                return;
            }
        }
        // A previously paired checkpoint is also proof of a non-adjacent replay.
        if let Some(last) = last {
            if self.entries.iter().any(|old| {
                old.session == self.session
                    && old.turn == self.turn
                    && old.epoch == self.epoch
                    && old.legacy_total == Some(total)
                    && old.usage == last
            }) {
                return;
            }
        }
        let reset = self.high.is_some_and(|high| {
            total.difference(high).is_none()
                || (last == Some(total) && total != high && high != Usage::default())
        });
        if reset {
            self.epoch += 1;
            self.high = None;
            self.warn("Codex 累计计数发生重置，已按新的计数区间统计；无响应标识的旧记录重放可能存在歧义。");
        }
        let usage = match self.high {
            Some(previous) => total.difference(previous).unwrap_or_default(),
            None => last.unwrap_or(total),
        };
        if self.high.is_none() && last.is_none() && total != Usage::default() {
            self.warn("Codex 首条记录只有累计值，无法确认历史用量的模型归属。");
        }
        if let Some(last) = last {
            if self.high.is_some() && last != usage {
                self.warn("Codex 累计差值与单次用量不同，该区间按未知模型统计。");
            }
        }
        let model = if (self.high.is_none() && last.is_none())
            || last.is_some_and(|last| self.high.is_some() && last != usage)
        {
            "(unknown)".to_owned()
        } else {
            self.model(&self.turn)
        };
        self.high = Some(total);
        if usage == Usage::default() {
            return;
        }
        if !usage.valid() {
            self.warn("Codex 累计字段变化不一致，该差值未计入。");
            return;
        }
        let index = self.entries.len();
        self.entries.push(Contribution {
            session: self.session.clone(),
            turn: self.turn.clone(),
            model,
            response: String::new(),
            timestamp: at.to_owned(),
            usage,
            legacy_total: Some(total),
            thread_total: None,
            epoch: self.epoch,
            compaction: false,
        });
        self.pending = Some(index);
    }

    fn record(&mut self, at: &str, payload: &Value, compaction: bool) {
        let response = text(payload, "response_id");
        let Some(usage) = Usage::read(&payload["usage"]) else {
            self.warn("Codex 响应用量缺失或无效，该条记录未计入。");
            return;
        };
        if response.is_empty() {
            self.warn("Codex 响应用量缺少 response_id，无法可靠去重，该条记录未计入。");
            return;
        }
        let session = match text(payload, "session_id") {
            "" => self.session.clone(),
            value => value.to_owned(),
        };
        let turn = match text(payload, "turn_id") {
            "" => self.turn.clone(),
            value => value.to_owned(),
        };
        let model = self.model(&turn);
        let thread_total = Usage::read(&payload["thread_token_usage"]);
        let key = (session.clone(), response.to_owned());
        if let Some(&index) = self.responses.get(&key) {
            let old = &mut self.entries[index];
            if old.usage == usage {
                old.compaction |= compaction;
                if at > old.timestamp.as_str() {
                    old.timestamp = at.to_owned();
                }
                return;
            }
            // Compare RFC3339 instants, not lexical timezone spellings.
            let newer = chrono::DateTime::parse_from_rfc3339(at)
                .ok()
                .zip(chrono::DateTime::parse_from_rfc3339(&old.timestamp).ok())
                .is_some_and(|(new, previous)| new > previous);
            if newer && (old.turn.is_empty() || turn.is_empty() || old.turn == turn) {
                old.usage = usage;
                old.timestamp = at.to_owned();
                old.compaction |= compaction;
            } else {
                self.warn("Codex 相同响应存在冲突或无序用量，已保留最后可验证版本。");
            }
            return;
        }
        let pending = self.pending.take().filter(|&index| {
            let old = &self.entries[index];
            !compaction
                && old.response.is_empty()
                && old.session == session
                && old.turn == turn
                && old.usage == usage
        });
        let exact = if pending.is_none() && !compaction {
            let mut matches = self.entries.iter().enumerate().filter(|(_, old)| {
                old.response.is_empty()
                    && old.session == session
                    && old.turn == turn
                    && old.epoch == self.epoch
                    && old.usage == usage
                    && thread_total.is_some()
                    && old.legacy_total == thread_total
            });
            let first = matches.next().map(|(index, _)| index);
            if matches.next().is_none() {
                first
            } else {
                None
            }
        } else {
            None
        };
        let index = if let Some(index) = pending.or(exact) {
            let old = &mut self.entries[index];
            old.response = response.to_owned();
            old.thread_total = thread_total;
            old.model = model;
            old.timestamp = at.to_owned();
            index
        } else {
            let index = self.entries.len();
            self.entries.push(Contribution {
                session,
                turn,
                model,
                response: response.to_owned(),
                timestamp: at.to_owned(),
                usage,
                legacy_total: None,
                thread_total,
                epoch: self.epoch,
                compaction,
            });
            index
        };
        self.responses.insert(key, index);
        if !compaction {
            self.pending = Some(index);
        }
    }
}

pub(super) fn parse(lines: impl Iterator<Item = AppResult<Value>>) -> AppResult<ParsedUsage> {
    let mut state = State {
        model: "(unknown)".to_owned(),
        ..State::default()
    };
    for line in lines {
        let line = line?;
        let payload = &line["payload"];
        let at = text(&line, "timestamp");
        match text(&line, "type") {
            "session_meta" => {
                let session = match text(payload, "session_id") {
                    "" => text(payload, "id"),
                    value => value,
                };
                if !session.is_empty() && session != state.session {
                    state.high = None;
                    state.pending = None;
                    state.epoch += 1;
                    state.session = session.to_owned();
                }
                if text(payload, "history_mode") == "paginated" {
                    state.warn("Codex 为分页历史，仅统计当前文件可见用量，无法还原缺失历史。");
                }
            }
            "turn_context" => {
                let turn = text(payload, "turn_id");
                if !turn.is_empty() && turn != state.turn {
                    state.pending = None;
                    state.turn = turn.to_owned();
                }
                if !text(payload, "model").is_empty() {
                    state.model = text(payload, "model").to_owned();
                    state.models.insert(state.turn.clone(), state.model.clone());
                }
            }
            "token_usage_record" => state.record(at, payload, false),
            "compacted" => {
                let record = &payload["latest_token_usage_record"];
                if !text(payload, "compaction_response_id").is_empty()
                    && text(payload, "compaction_response_id") == text(record, "response_id")
                {
                    state.record(at, record, true);
                } else {
                    state.warn("Codex 压缩记录没有可验证用量，压缩费用可能未包含。");
                }
            }
            "event_msg" => match text(payload, "type") {
                "token_count" if !payload["info"].is_null() => state.count(at, &payload["info"]),
                "user_message" | "task_started" => {
                    state.pending = None;
                    if !text(payload, "turn_id").is_empty() {
                        state.turn = text(payload, "turn_id").to_owned();
                    }
                }
                "thread_settings_applied" => {
                    if !text(&payload["settings"], "model").is_empty() {
                        state.model = text(&payload["settings"], "model").to_owned();
                        state.models.insert(state.turn.clone(), state.model.clone());
                    }
                }
                _ => {}
            },
            "response_item" if text(payload, "role") == "user" => state.pending = None,
            _ => {}
        }
    }
    // Mixed, unpaired domains cannot establish a precise total. Surface that explicitly.
    if state.entries.iter().any(|entry| {
        !entry.compaction && !entry.response.is_empty() && entry.legacy_total.is_none()
    }) && state.entries.iter().any(|entry| entry.response.is_empty())
    {
        state.warn("Codex 存在无法配对的累计与响应用量，结果可能重复，不能视为精确账单。");
    }
    let mut totals: BTreeMap<String, Usage> = BTreeMap::new();
    for entry in state.entries {
        let total = totals.entry(entry.model).or_default();
        *total = total
            .plus(entry.usage)
            .ok_or_else(|| AppError::Other("Codex 用量累计溢出".into()))?;
    }
    Ok(ParsedUsage {
        models: totals
            .into_iter()
            .map(|(model, usage)| ModelUsage {
                model,
                tokens: Tokens {
                    input: usage.0[0] - usage.0[2] - usage.0[3],
                    output: usage.0[1],
                    cache_read: usage.0[2],
                    cache_write: usage.0[3],
                    cache_write_1h: 0,
                    reasoning: usage.0[4],
                },
            })
            .collect(),
        warnings: state.warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // Synthetic cases adapted from Magpie codex_usage_test.go, same locked MIT source.
    fn usage(input: u64, output: u64) -> Value {
        json!({"input_tokens":input,"output_tokens":output,"cached_input_tokens":input/2,"reasoning_output_tokens":output/2})
    }
    fn count(second: u64, total: Value, last: Value) -> Value {
        json!({"timestamp":format!("2026-01-01T00:00:{second:02}Z"),"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":total,"last_token_usage":last}}})
    }
    fn record(second: u64, id: &str, total: Value, used: Value) -> Value {
        json!({"timestamp":format!("2026-01-01T00:00:{second:02}Z"),"type":"token_usage_record","payload":{"response_id":id,"thread_token_usage":total,"usage":used}})
    }
    fn run(lines: Vec<Value>) -> ParsedUsage {
        parse(lines.into_iter().map(Ok)).unwrap()
    }
    fn total(result: &ParsedUsage) -> u64 {
        result
            .models
            .iter()
            .map(|model| model.tokens.input + model.tokens.cache_read + model.tokens.cache_write)
            .sum()
    }

    #[test]
    fn legacy_incremental_replay_reset_and_model_switch() {
        let first = count(1, usage(100, 20), usage(100, 20));
        let result = run(vec![
            json!({"type":"turn_context","payload":{"turn_id":"a","model":"model-a"}}),
            first.clone(),
            count(2, usage(200, 40), usage(100, 20)),
            first,
            json!({"type":"turn_context","payload":{"turn_id":"b","model":"model-b"}}),
            count(3, usage(30, 10), usage(30, 10)),
        ]);
        assert_eq!(total(&result), 230);
        assert_eq!(result.models.len(), 2);
        assert_eq!(result.models[0].tokens.input, 100);
        assert!(result
            .warnings
            .iter()
            .any(|warning| warning.contains("重置")));
    }

    #[test]
    fn response_and_count_pair_in_both_orders_with_compaction() {
        for reverse in [false, true] {
            let c = count(1, usage(100, 20), usage(100, 20));
            let r = record(2, "r1", usage(600, 120), usage(100, 20));
            let compact = json!({"type":"compacted","payload":{"compaction_response_id":"compact","latest_token_usage_record":{"response_id":"compact","usage":usage(500,100)}}});
            let mut lines = vec![compact.clone()];
            lines.extend(if reverse { vec![r, c] } else { vec![c, r] });
            lines.push(compact);
            lines.push(record(4, "r1", usage(600, 120), usage(100, 20)));
            assert_eq!(total(&run(lines)), 600);
        }
    }

    #[test]
    fn input_boundary_keeps_identical_independent_calls() {
        let result = run(vec![
            record(1, "r", usage(100, 20), usage(100, 20)),
            json!({"type":"event_msg","payload":{"type":"user_message"}}),
            count(2, usage(200, 40), usage(100, 20)),
        ]);
        assert_eq!(total(&result), 200);
        assert!(!result.warnings.is_empty());
    }

    #[test]
    fn response_revision_replay_and_conflict_are_bounded() {
        let result = run(vec![
            record(1, "r", usage(100, 20), usage(100, 20)),
            record(3, "r", usage(120, 30), usage(120, 30)),
            record(1, "r", usage(100, 20), usage(100, 20)),
            record(3, "r", usage(500, 20), usage(500, 20)),
        ]);
        assert_eq!(total(&result), 120);
        assert_eq!(result.warnings.len(), 1);
    }

    #[test]
    fn no_last_uses_delta_and_unknown_historical_model() {
        let result = run(vec![
            count(1, usage(100, 20), Value::Null),
            count(2, usage(150, 30), Value::Null),
        ]);
        assert_eq!(total(&result), 150);
        assert_eq!(result.models[0].model, "(unknown)");
        assert!(!result.warnings.is_empty());
    }

    #[test]
    fn malformed_negative_and_excess_cached_usage_not_counted() {
        let result = run(vec![
            count(1, json!({"input_tokens":-1,"output_tokens":5}), Value::Null),
            record(
                2,
                "r",
                Value::Null,
                json!({"input_tokens":5,"output_tokens":10,"cached_input_tokens":6}),
            ),
        ]);
        assert!(result.models.is_empty());
        assert_eq!(result.warnings.len(), 2);
    }

    #[test]
    fn iterator_error_propagates_and_unknown_is_explicit() {
        assert!(parse(vec![Err(AppError::Cancelled)].into_iter()).is_err());
        let result = run(vec![count(1, usage(100, 20), usage(100, 20))]);
        assert_eq!(result.models[0].model, "(unknown)");
        assert_eq!(result.models[0].tokens.reasoning, 10);
        assert_eq!(result.models[0].tokens.output, 20);
    }

    #[test]
    fn upstream_synthetic_compaction_domains_preserve_exact_totals_both_orders() {
        // Copied synthetic fixture: Magpie internal/sessions/testdata/
        // codex-compaction-counter-domains.jsonl at the MIT commit cited above.
        let original: Vec<Value> =
            include_str!("../../../tests/fixtures/usage/codex-compaction-counter-domains.jsonl")
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect();
        for reverse in [false, true] {
            let mut lines = original.clone();
            if reverse {
                for index in [2, 8, 10] {
                    lines.swap(index, index + 1);
                }
            }
            let result = run(lines);
            assert_eq!(result.models.len(), 1);
            assert_eq!(result.models[0].tokens.input, 11157);
            assert_eq!(result.models[0].tokens.cache_read, 496640);
            assert_eq!(result.models[0].tokens.output, 6856);
            assert!(result.warnings.is_empty(), "{:?}", result.warnings);
        }
    }

    #[test]
    fn larger_first_request_proves_new_legacy_runtime() {
        let result = run(vec![
            count(1, usage(100, 20), usage(100, 20)),
            count(2, usage(200, 40), usage(200, 40)),
            count(3, usage(200, 40), usage(200, 40)),
        ]);
        assert_eq!(total(&result), 300);
    }

    #[test]
    fn paginated_zero_checkpoint_does_not_import_unavailable_history() {
        let result = run(vec![
            json!({"type":"session_meta","payload":{"id":"page","history_mode":"paginated"}}),
            count(1, usage(1000, 200), usage(0, 0)),
            count(2, usage(1100, 220), usage(100, 20)),
        ]);
        assert_eq!(total(&result), 100);
        assert!(result
            .warnings
            .iter()
            .any(|warning| warning.contains("分页")));
    }

    #[test]
    fn checked_sum_rejects_overflow_instead_of_wrapping() {
        let huge = json!({"input_tokens":u64::MAX,"output_tokens":0});
        assert!(parse(
            vec![
                record(1, "one", Value::Null, huge.clone()),
                record(2, "two", Value::Null, huge)
            ]
            .into_iter()
            .map(Ok)
        )
        .is_err());
    }
}
