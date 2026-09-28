use super::*;
use serde_json::{Value, json};

fn record() -> Value {
    json!({
        "id": "a".repeat(64),
        "image": "registry.example/app:1",
        "name": "example",
        "state": "running",
        "status": "Up 3 minutes",
        "ports": "127.0.0.1:8080->80/tcp",
        "networks": "bridge"
    })
}

fn bad(input: &str) {
    let error = parse_docker_containers(input).unwrap_err();
    assert_eq!(error.code, ErrorCode::Discovery);
    assert_eq!(
        error.message,
        "Docker container inventory is unavailable for this connection."
    );
    assert!(error.host_key.is_none());
}

#[test]
fn empty_one_and_sixty_four_records_preserve_order() {
    assert!(parse_docker_containers("").unwrap().is_empty());
    let one = parse_docker_containers(&(record().to_string() + "\n")).unwrap();
    assert_eq!(one.len(), 1);
    assert_eq!(one[0].id, "a".repeat(64));
    assert_eq!(one[0].state, ContainerState::Running);
    let records = (0..64)
        .map(|index| {
            let mut value = record();
            value["name"] = json!(format!("entry-{index}"));
            value.to_string()
        })
        .collect::<Vec<_>>();
    let parsed = parse_docker_containers(&(records.join("\n") + "\n")).unwrap();
    assert_eq!(parsed.len(), 64);
    assert_eq!(parsed[0].name, "entry-0");
    assert_eq!(parsed[63].name, "entry-63");
    bad(&(records.join("\n") + "\n" + &record().to_string()));
}

#[test]
fn malformed_blank_unknown_and_oversized_records_fail_without_partial_result() {
    for input in [
        "\n",
        " ",
        "{}",
        "null",
        "true",
        "[]",
        "[{}]",
        "{invalid}",
        "\"secret remote text\"",
    ] {
        bad(input);
    }
    let valid = record().to_string();
    bad(&format!("{valid}\n\n{valid}"));
    bad(&format!("{valid}\n\n"));
    bad(&format!("{valid}\nprivate remote output"));
    bad(&(valid.clone() + "\0"));
    bad(&"x".repeat(MAX_RAW_BYTES + 1));
    let mut unknown = record();
    unknown["Labels"] = json!({"secret":"never publish"});
    bad(&unknown.to_string());
}

#[test]
fn every_selected_field_is_required_unique_and_string_typed() {
    for key in [
        "id", "image", "name", "state", "status", "ports", "networks",
    ] {
        let mut missing = record();
        missing.as_object_mut().unwrap().remove(key);
        bad(&missing.to_string());
        let original = record();
        let serialized = original.to_string();
        let duplicate = format!(
            "{},\"{key}\":{}}}",
            &serialized[..serialized.len() - 1],
            original[key]
        );
        bad(&duplicate);
        for value in [json!(null), json!(true), json!(1), json!([]), json!({})] {
            let mut wrong = record();
            wrong[key] = value;
            bad(&wrong.to_string());
        }
    }
}

#[test]
fn full_lowercase_hex_id_and_every_documented_state_are_required() {
    for state in [
        ("created", ContainerState::Created),
        ("restarting", ContainerState::Restarting),
        ("running", ContainerState::Running),
        ("removing", ContainerState::Removing),
        ("paused", ContainerState::Paused),
        ("exited", ContainerState::Exited),
        ("dead", ContainerState::Dead),
    ] {
        let mut value = record();
        value["state"] = json!(state.0);
        assert_eq!(
            parse_docker_containers(&value.to_string()).unwrap()[0].state,
            state.1
        );
    }
    for id in [
        "a".repeat(63),
        "a".repeat(65),
        format!("{}g", "a".repeat(63)),
        "A".repeat(64),
        "é".repeat(32),
    ] {
        let mut value = record();
        value["id"] = json!(id);
        bad(&value.to_string());
    }
    let mut unknown = record();
    unknown["state"] = json!("healthy");
    bad(&unknown.to_string());
}

#[test]
fn fields_are_byte_bounded_and_optional_display_fields_may_be_empty() {
    for (key, max, required) in [
        ("name", 255, true),
        ("image", 512, true),
        ("status", 512, true),
        ("ports", 1024, false),
        ("networks", 1024, false),
    ] {
        let mut value = record();
        value[key] = json!("x".repeat(max));
        assert!(parse_docker_containers(&value.to_string()).is_ok());
        value[key] = json!("x".repeat(max + 1));
        bad(&value.to_string());
        value[key] = json!("é".repeat(max / 2 + 1));
        bad(&value.to_string());
        value[key] = json!("");
        if required {
            bad(&value.to_string());
        } else {
            assert!(parse_docker_containers(&value.to_string()).is_ok());
        }
        value[key] = json!("München / 東京");
        assert!(parse_docker_containers(&value.to_string()).is_ok());
    }
}

#[test]
fn deceptive_controls_reject_the_whole_snapshot_without_echo() {
    for key in ["name", "image", "status", "ports", "networks"] {
        for control in [
            '\0', '\n', '\r', '\t', '\u{1b}', '\u{85}', '\u{202e}', '\u{2066}', '\u{200b}',
            '\u{200d}', '\u{feff}',
        ] {
            let mut value = record();
            value[key] = json!(format!("normal{control}deceptive"));
            bad(&value.to_string());
        }
    }
}
