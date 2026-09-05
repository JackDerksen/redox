#![cfg(unix)]

use std::fs;
use std::thread;
use std::time::{Duration, Instant};

use redox_lsp::{Client, ClientEvent, ClientInfo, ServerCommand};
use serde_json::json;

fn frame(message: serde_json::Value) -> String {
    let payload = message.to_string();
    format!("Content-Length: {}\r\n\r\n{payload}", payload.len())
}

#[test]
fn notifications_cannot_deadlock_writes_and_server_ids_are_independent() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut output = frame(json!({
        "jsonrpc":"2.0", "id":1, "method":"window/showMessageRequest",
        "params":{"type":3,"message":"Starting"}
    }));
    output.push_str(&frame(
        json!({"jsonrpc":"2.0","id":1,"result":{"capabilities":{}}}),
    ));
    let notification = frame(json!({"jsonrpc":"2.0","method":"window/logMessage",
        "params":{"type":3,"message":"x".repeat(4096)}}));
    output.push_str(&notification.repeat(1024));
    fs::write(root.join("output"), output).unwrap();
    let command =
        ServerCommand::new("burst", "sh").args(&["-c", "exec 3>&1; cat output; exec cat > input"]);
    let mut client = Client::spawn(
        &command,
        root,
        ClientInfo {
            name: "test",
            version: "1",
        },
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut server_request = false;
    let mut opened = false;
    let mut notifications = 0;
    loop {
        assert!(
            Instant::now() < deadline,
            "transport stopped making progress"
        );
        match client.try_recv() {
            Some(ClientEvent::Message(message)) if message.get("id").is_some() => {
                assert_eq!(message["method"], "window/showMessageRequest");
                server_request = true;
                client
                    .send_response(message["id"].clone(), json!(null))
                    .unwrap();
            }
            Some(ClientEvent::Initialized { .. }) => {
                assert!(server_request);
                // Let stdout and the event queue fill before writing a large document.
                thread::sleep(Duration::from_millis(100));
                let started = Instant::now();
                client
                    .send_did_open(
                        &root.join("example.rs"),
                        "rust",
                        1,
                        &"x".repeat(2 * 1024 * 1024),
                    )
                    .unwrap();
                client.send_did_close(&root.join("example.rs")).unwrap();
                assert!(started.elapsed() < Duration::from_secs(1));
                opened = true;
            }
            Some(ClientEvent::Message(_)) => notifications += 1,
            Some(event) => panic!("unexpected event: {event:?}"),
            None => thread::sleep(Duration::from_millis(1)),
        }
        if opened
            && notifications == 1024
            && fs::read_to_string(root.join("input")).is_ok_and(|input| {
                input.contains("textDocument/didOpen") && input.contains("textDocument/didClose")
            })
        {
            break;
        }
    }
}

#[test]
fn a_full_write_queue_fails_the_session_without_blocking() {
    let directory = tempfile::tempdir().unwrap();
    let command = ServerCommand::new("blocked", "sh").args(&["-c", "exec sleep 10"]);
    let mut client = Client::spawn(
        &command,
        directory.path(),
        ClientInfo {
            name: "test",
            version: "1",
        },
    )
    .unwrap();
    let started = Instant::now();
    let mut failed = false;
    for _ in 0..512 {
        if client
            .send_notification("test", json!({"text":"x".repeat(65536)}))
            .is_err()
        {
            failed = true;
            break;
        }
    }
    assert!(failed);
    assert!(started.elapsed() < Duration::from_secs(3));
    assert!(matches!(
        client.try_recv(),
        Some(ClientEvent::Terminated { error: Some(_) })
    ));
    assert!(client.try_recv().is_none());
    assert!(client.send_notification("test", json!({})).is_err());
}
