use super::*;
use serde_json::json;

fn record(fields: Value) -> String {
    let mut value = json!({"__REALTIME_TIMESTAMP":"1000001"});
    value
        .as_object_mut()
        .unwrap()
        .extend(fields.as_object().unwrap().clone());
    value.to_string()
}

fn bad(input: &str) {
    let error = parse_system_journal(input).unwrap_err();
    assert_eq!(error.code, ErrorCode::Discovery);
    assert_eq!(
        error.message,
        "System journal entries are unavailable for this connection."
    );
    assert!(error.host_key.is_none());
}

#[test]
fn projects_only_selected_fields_and_converts_microseconds() {
    let entries = parse_system_journal(&record(json!({
        "MESSAGE":"synthetic text", "PRIORITY":"6", "_SYSTEMD_UNIT":"example.service",
        "SYSLOG_IDENTIFIER":"example worker", "__CURSOR":"excluded", "_BOOT_ID":"excluded",
        "__MONOTONIC_TIMESTAMP":"excluded", "UNSELECTED":"excluded"
    })))
    .unwrap();
    assert_eq!(entries.len(), 1);
    let entry = &entries[0];
    assert_eq!(entry.timestamp, "1970-01-01T00:00:01.000001Z");
    assert_eq!(entry.priority, Some(JournalPriority::Info));
    assert_eq!(entry.unit.as_deref(), Some("example.service"));
    assert_eq!(entry.identifier.as_deref(), Some("example worker"));
    assert_eq!(entry.message_state, SystemJournalMessageState::Text);
    assert_eq!(entry.message.as_deref(), Some("synthetic text"));
    let serialized = serde_json::to_string(entry).unwrap();
    assert!(!serialized.contains("excluded"));
    assert!(!serialized.contains("1000001"));
}

#[test]
fn empty_ten_records_order_and_trailing_newline() {
    assert!(parse_system_journal("").unwrap().is_empty());
    let records = (1..=10)
        .rev()
        .map(|i| record(json!({"__REALTIME_TIMESTAMP":i.to_string()})))
        .collect::<Vec<_>>()
        .join("\n");
    let parsed = parse_system_journal(&(records.clone() + "\n")).unwrap();
    assert_eq!(parsed.len(), 10);
    assert!(
        parsed
            .windows(2)
            .all(|pair| pair[0].timestamp > pair[1].timestamp)
    );
    bad(&(records + "\n" + &record(json!({}))));
    assert_eq!(
        parse_system_journal(&record(json!({"__REALTIME_TIMESTAMP":"0"}))).unwrap()[0].timestamp,
        "1970-01-01T00:00:00.000000Z"
    );
}

#[test]
fn rejects_malformed_records_without_partial_success_or_echo() {
    for value in [
        "\n",
        " ",
        "{}",
        "null",
        "true",
        "123",
        "[]",
        "[\"1000001\"]",
        "\"hostile journal data\"",
        "{invalid}",
    ] {
        bad(value);
    }
    let valid = record(json!({}));
    bad(&format!("{valid}\n\n{valid}"));
    bad(&format!("{valid}\n\n"));
    bad(&format!("{valid}\nmalformed secret"));
    bad(&" ".repeat(MAX_RAW_BYTES + 1));
    bad(&format!("{valid}\0"));
}

#[test]
fn timestamp_requires_one_valid_scalar_text_and_rfc3339_range() {
    for value in [
        json!(null),
        json!([]),
        json!([49, 50]),
        json!(["1", "2"]),
        json!(true),
        json!(1),
        json!(""),
        json!("-1"),
        json!("+1"),
        json!("1.1"),
        json!(" 1"),
        json!("secret"),
        json!("18446744073709551616"),
        json!("18446744073709551615"),
        json!("253402300800000000"),
    ] {
        bad(&record(json!({"__REALTIME_TIMESTAMP":value})));
    }
    bad(r#"{"__REALTIME_TIMESTAMP":"1","__REALTIME_TIMESTAMP":"2"}"#);
}

#[test]
fn missing_null_binary_multivalue_and_unexpected_messages_are_distinct() {
    let missing = parse_system_journal(&record(json!({}))).unwrap().remove(0);
    assert_eq!(missing.message_state, SystemJournalMessageState::Missing);
    assert!(missing.message.is_none());
    for value in [
        json!(null),
        json!([1, 2, 255]),
        json!(["one", "two"]),
        json!({"nested":"secret"}),
        json!(1),
        json!(true),
    ] {
        let entry = parse_system_journal(&record(json!({"MESSAGE":value})))
            .unwrap()
            .remove(0);
        assert_eq!(entry.message_state, SystemJournalMessageState::Omitted);
        assert!(entry.message.is_none());
    }
    let empty = parse_system_journal(&record(json!({"MESSAGE":""})))
        .unwrap()
        .remove(0);
    assert_eq!(empty.message_state, SystemJournalMessageState::Text);
    assert_eq!(empty.message.as_deref(), Some(""));
    bad(r#"{"__REALTIME_TIMESTAMP":"1","MESSAGE":"a","MESSAGE":"b"}"#);
}

#[test]
fn controls_are_visible_inert_and_distinct_from_literal_escapes() {
    let text =
        "line\n\r\t\u{0}\u{1}\u{1b}[31m\u{7f}\u{85}\u{202e}\u{2066}\u{200b}\u{feff}<img src=x>\\n";
    let entry = parse_system_journal(&record(json!({"MESSAGE":text})))
        .unwrap()
        .remove(0);
    let display = entry.message.unwrap();
    assert_eq!(
        display,
        "line\\n\\r\\t\\u{0000}\\u{0001}\\u{001b}[31m\\u{007f}\\u{0085}\\u{202e}\\u{2066}\\u{200b}\\u{feff}<img src=x>\\\\n"
    );
    assert!(!display.chars().any(unsafe_display));
    for c in [
        '\u{061c}',
        '\u{200d}',
        '\u{2060}',
        '\u{034f}',
        '\u{2028}',
        '\u{00ad}',
        '\u{e0001}',
    ] {
        let entry = parse_system_journal(&record(json!({"MESSAGE":c.to_string()})))
            .unwrap()
            .remove(0);
        assert!(!entry.message.unwrap().chars().any(unsafe_display));
    }
}

#[test]
fn raw_text_and_expanded_display_limits_are_independent() {
    let valid = record(json!({"MESSAGE":"x".repeat(4096)}));
    assert_eq!(
        parse_system_journal(&valid).unwrap()[0]
            .message
            .as_ref()
            .unwrap()
            .len(),
        4096
    );
    bad(&record(json!({"MESSAGE":"x".repeat(4097)})));
    bad(&record(json!({"MESSAGE":"é".repeat(2049)})));
    let entry = parse_system_journal(&record(json!({"MESSAGE":"\u{1}".repeat(4096)})))
        .unwrap()
        .remove(0);
    assert_eq!(entry.message_state, SystemJournalMessageState::Omitted);
    assert!(entry.message.is_none());
    let entry = parse_system_journal(&record(json!({"MESSAGE":"\n".repeat(4096)})))
        .unwrap()
        .remove(0);
    assert_eq!(entry.message.unwrap().len(), 8192);
}

#[test]
fn optional_fields_handle_omission_and_enforce_scalar_bounds() {
    for field in ["_SYSTEMD_UNIT", "SYSLOG_IDENTIFIER"] {
        for value in [
            json!(null),
            json!(["one", "two"]),
            json!([0, 255]),
            json!({}),
        ] {
            let entry = parse_system_journal(&record(json!({field:value})))
                .unwrap()
                .remove(0);
            assert!(entry.unit.is_none() && entry.identifier.is_none());
        }
        let limit = if field == "_SYSTEMD_UNIT" { 255 } else { 128 };
        assert!(parse_system_journal(&record(json!({field:"a".repeat(limit)}))).is_ok());
        for value in [
            json!("a".repeat(limit + 1)),
            json!(""),
            json!(1),
            json!(true),
        ] {
            bad(&record(json!({field:value})));
        }
        for c in [
            '\n', '\t', '\r', '\u{1b}', '\u{85}', '\u{202e}', '\u{200b}', '\u{feff}',
        ] {
            bad(&record(json!({field:format!("text{c}")})));
        }
        bad(&format!(
            r#"{{"__REALTIME_TIMESTAMP":"1","{field}":null,"{field}":"x"}}"#
        ));
    }
}

#[test]
fn priority_is_a_validated_enum_never_an_arbitrary_scalar() {
    let expected = [
        JournalPriority::Emergency,
        JournalPriority::Alert,
        JournalPriority::Critical,
        JournalPriority::Error,
        JournalPriority::Warning,
        JournalPriority::Notice,
        JournalPriority::Info,
        JournalPriority::Debug,
    ];
    for (i, priority) in expected.into_iter().enumerate() {
        assert_eq!(
            parse_system_journal(&record(json!({"PRIORITY":i.to_string()}))).unwrap()[0].priority,
            Some(priority)
        );
    }
    for value in [json!(null), json!(["1", "2"]), json!([49]), json!({})] {
        assert_eq!(
            parse_system_journal(&record(json!({"PRIORITY":value}))).unwrap()[0].priority,
            None
        );
    }
    for value in [
        json!("8"),
        json!("-1"),
        json!("01"),
        json!(""),
        json!("secret"),
        json!("0".repeat(100)),
        json!(1),
        json!(true),
    ] {
        bad(&record(json!({"PRIORITY":value})));
    }
    bad(r#"{"__REALTIME_TIMESTAMP":"1","PRIORITY":"1","PRIORITY":"2"}"#);
}
