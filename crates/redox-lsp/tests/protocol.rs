use std::fs;

use redox_lsp::lint::{parse_clippy_output, parse_golangci_lint_text_output, parse_ruff_output};
use redox_lsp::{
    CompletionCandidate, DiagnosticSeverity, InsertTextFormat, ProviderId, SymbolInfoBlock,
    SymbolInfoKind, completion_snippet_expansion, file_uri, parse_code_action_response,
    parse_completion_response, parse_definition_response, parse_hover_response,
    parse_publish_diagnostics, parse_workspace_edit, workspace_root_for,
};
use serde_json::json;

#[test]
fn protocol_responses_parse_into_owned_models() {
    let completion = parse_completion_response(&json!({
        "result": {
            "itemDefaults": {
                "insertTextFormat": 2,
                "editRange": {
                    "start": { "line": 1, "character": 4 },
                    "end": { "line": 1, "character": 7 }
                }
            },
            "items": [{
                "label": "Ok",
                "insertText": "ignored",
                "textEditText": "Ok(${1:value})"
            }]
        }
    }));
    assert_eq!(completion.len(), 1);
    assert_eq!(completion[0].insert_text_format, InsertTextFormat::Snippet);
    assert_eq!(
        completion[0].text_edit.as_ref().unwrap().range.start.line,
        1
    );
    assert_eq!(
        completion[0].text_edit.as_ref().unwrap().new_text,
        "Ok(${1:value})"
    );

    let definition = parse_definition_response(&json!({
        "result": [{
            "uri": "file:///tmp/example.rs",
            "range": {
                "start": { "line": 4, "character": 2 },
                "end": { "line": 4, "character": 7 }
            }
        }]
    }))
    .unwrap();
    assert_eq!(definition.uri, "file:///tmp/example.rs");
    assert_eq!(definition.range.start.character, 2);
}

#[test]
fn diagnostics_preserve_related_information() {
    let (_, version, diagnostics) = parse_publish_diagnostics(&json!({
        "method": "textDocument/publishDiagnostics",
        "params": {
            "uri": "file:///tmp/example.rs",
            "version": 7,
            "diagnostics": [{
                "range": {
                    "start": { "line": 2, "character": 4 },
                    "end": { "line": 2, "character": 9 }
                },
                "severity": 1,
                "message": "type mismatch (see details)",
                "relatedInformation": [{
                    "location": {
                        "uri": "file:///tmp/source.rs",
                        "range": {
                            "start": { "line": 8, "character": 1 },
                            "end": { "line": 8, "character": 6 }
                        }
                    },
                    "message": "expected `usize` here"
                }]
            }]
        }
    }))
    .unwrap();

    assert_eq!(version, Some(7));
    assert_eq!(diagnostics[0].severity, DiagnosticSeverity::Error);
    assert_eq!(diagnostics[0].message, "type mismatch (see details)");
    assert_eq!(diagnostics[0].related_information.len(), 1);
    assert_eq!(
        diagnostics[0].related_information[0].location.uri,
        "file:///tmp/source.rs"
    );
    assert_eq!(
        diagnostics[0].related_information[0].message,
        "expected `usize` here"
    );
}

#[test]
fn snippets_derive_function_parameters_when_servers_omit_tabstops() {
    let item = CompletionCandidate {
        label: "DoThing".to_string(),
        detail: Some("func DoThing(ctx context.Context, name string) error".to_string()),
        label_detail: None,
        label_description: None,
        documentation: None,
        kind: Some("function".to_string()),
        filter_text: None,
        sort_text: None,
        insert_text: "DoThing()".to_string(),
        insert_text_format: InsertTextFormat::PlainText,
        text_edit: None,
        additional_text_edits: Vec::new(),
    };

    let expansion = completion_snippet_expansion(&item, &item.insert_text).unwrap();
    assert_eq!(expansion.text, "DoThing(ctx, name)");
    assert_eq!(expansion.placeholders.len(), 2);
    assert_eq!(
        (
            expansion.placeholders[0].start,
            expansion.placeholders[0].end
        ),
        (8, 11)
    );
}

#[test]
fn snippets_expand_server_tabstops_without_signature_metadata() {
    let item = CompletionCandidate {
        label: "DoThing".to_string(),
        detail: None,
        label_detail: None,
        label_description: None,
        documentation: None,
        kind: Some("function".to_string()),
        filter_text: None,
        sort_text: None,
        insert_text: "DoThing($1, ${2:value})$0".to_string(),
        insert_text_format: InsertTextFormat::Snippet,
        text_edit: None,
        additional_text_edits: Vec::new(),
    };

    let expansion = completion_snippet_expansion(&item, &item.insert_text).unwrap();
    assert_eq!(expansion.text, "DoThing(, value)");
    assert_eq!(expansion.cursor_offset, Some(16));
    assert_eq!(
        expansion
            .placeholders
            .iter()
            .map(|placeholder| (placeholder.tabstop, placeholder.start, placeholder.end))
            .collect::<Vec<_>>(),
        vec![(1, 8, 8), (2, 10, 15)]
    );
}

#[test]
fn hover_and_code_actions_keep_structured_content() {
    let hover = parse_hover_response(&json!({
        "result": {
            "contents": [
                { "language": "rust", "value": "pub fn hover() -> bool" },
                { "kind": "markdown", "value": "Returns `true`.\n\n\n- Fast" }
            ]
        }
    }));
    assert_eq!(
        hover,
        vec![
            SymbolInfoBlock {
                kind: SymbolInfoKind::Code {
                    language: Some("rust".to_string()),
                },
                text: "pub fn hover() -> bool".to_string(),
            },
            SymbolInfoBlock {
                kind: SymbolInfoKind::Markdown,
                text: "Returns `true`.\n\n- Fast".to_string(),
            },
        ]
    );

    let actions = parse_code_action_response(&json!({
        "result": [
            {
                "title": "Import Debug",
                "kind": "quickfix",
                "isPreferred": true,
                "edit": {
                    "documentChanges": [{
                        "textDocument": { "uri": "file:///tmp/example.rs", "version": 1 },
                        "edits": [{
                            "range": {
                                "start": { "line": 0, "character": 0 },
                                "end": { "line": 0, "character": 0 }
                            },
                            "newText": "use std::fmt::Debug;\\n"
                        }]
                    }]
                }
            },
            { "title": "Disabled", "disabled": { "reason": "nope" } }
        ]
    }));
    assert_eq!(actions.len(), 1);
    assert!(actions[0].preferred);
    assert!(actions[0].edit.is_some());

    let edit = parse_workspace_edit(&json!({
        "documentChanges": [{
            "textDocument": { "uri": "file:///tmp/example.rs", "version": 1 },
            "edits": [{
                "range": {
                    "start": { "line": 1, "character": 2 },
                    "end": { "line": 1, "character": 5 }
                },
                "newText": "value"
            }]
        }]
    }))
    .unwrap();
    assert_eq!(edit.document_edits[0].edits[0].new_text, "value");
    assert_eq!(edit.document_edits[0].version, Some(1));

    assert!(
        parse_workspace_edit(&json!({
            "documentChanges": [
                {
                    "kind": "rename",
                    "oldUri": "file:///tmp/old.rs",
                    "newUri": "file:///tmp/new.rs"
                },
                {
                    "textDocument": { "uri": "file:///tmp/example.rs", "version": 1 },
                    "edits": [{
                        "range": {
                            "start": { "line": 1, "character": 2 },
                            "end": { "line": 1, "character": 5 }
                        },
                        "newText": "value"
                    }]
                }
            ]
        }))
        .is_none()
    );
}

#[test]
fn workspace_edits_prefer_document_changes_and_reject_unversioned_changes_alone() {
    let edits = json!([{
        "range": {
            "start": { "line": 0, "character": 0 },
            "end": { "line": 0, "character": 0 }
        },
        "newText": "changed"
    }]);
    let changes = json!({ "file:///tmp/example.rs": edits });
    assert!(parse_workspace_edit(&json!({ "changes": changes })).is_none());
    assert!(
        parse_code_action_response(&json!({
            "result": [{ "title": "Unsafe edit", "edit": { "changes": changes } }]
        }))
        .is_empty()
    );

    for document in [
        json!({ "uri": "file:///tmp/other.rs", "version": 1 }),
        json!({ "uri": "file:///tmp/other.rs", "version": null }),
        json!({ "uri": "file:///tmp/other.rs" }),
    ] {
        let mut payload = json!({
            "documentChanges": [{ "textDocument": document, "edits": edits }]
        });
        let expected = parse_workspace_edit(&payload).unwrap();
        assert_eq!(
            expected.document_edits[0].version,
            document["version"].as_i64().map(|version| version as i32)
        );
        payload["changes"] = json!({ "file:///tmp/example.rs": [] });
        assert_eq!(parse_workspace_edit(&payload).as_ref(), Some(&expected));
        payload["changes"] = changes.clone();
        assert_eq!(parse_workspace_edit(&payload).as_ref(), Some(&expected));
        let actions = parse_code_action_response(&json!({
            "result": [{ "title": "Versioned edit", "edit": payload }]
        }));
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].edit.as_ref(), Some(&expected));
        payload["changes"] = json!({ "file:///tmp/example.rs": "invalid" });
        assert_eq!(parse_workspace_edit(&payload), Some(expected));
    }
    for document_changes in [json!(null), json!({}), json!([]), json!([{}])] {
        assert!(
            parse_workspace_edit(&json!({
                "documentChanges": document_changes, "changes": changes
            }))
            .is_none()
        );
    }
}

#[test]
fn workspace_and_linter_parsers_resolve_file_uris() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path();
    let crate_directory = root.join("member");
    let source_directory = crate_directory.join("src");
    fs::create_dir_all(&source_directory).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nmembers = [\"member\"]\n",
    )
    .unwrap();
    fs::write(
        crate_directory.join("Cargo.toml"),
        "[package]\nname = \"member\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    let rust_file = source_directory.join("lib.rs");
    fs::write(
        &rust_file,
        "pub fn demo() {\n    let unused_value = 42;\n}\n",
    )
    .unwrap();

    assert_eq!(
        workspace_root_for(
            &rust_file,
            &redox_lsp::provider_spec(ProviderId::RustAnalyzer).unwrap(),
            root,
        ),
        root
    );
    let rust_uri = file_uri(&rust_file).unwrap();
    let clippy = parse_clippy_output(
        br#"{"reason":"compiler-message","message":{"level":"warning","message":"unused variable","spans":[{"file_name":"member/src/lib.rs","line_start":2,"line_end":2,"column_start":9,"column_end":21,"is_primary":true}]}}"#,
        root,
    );
    assert_eq!(clippy[&rust_uri][0].start_line, 1);

    let python_file = root.join("example.py");
    fs::write(&python_file, "import os\n").unwrap();
    let ruff = parse_ruff_output(
        br#"[{"filename":"example.py","message":"unused","code":"F401","location":{"row":1,"column":8},"end_location":{"row":1,"column":10}}]"#,
        root,
    );
    assert_eq!(ruff[&file_uri(&python_file).unwrap()][0].start_line, 0);

    let go_file = root.join("lexer.go");
    fs::write(&go_file, "package main\n").unwrap();
    let golangci =
        parse_golangci_lint_text_output(b"lexer.go:1:1: package comment is missing\n", root);
    assert_eq!(golangci[&file_uri(&go_file).unwrap()][0].start_line, 0);
}

#[test]
fn snippet_choices_and_escaped_braces_keep_their_initial_text() {
    let expansion = redox_lsp::expand(r"${1|one\,two,three|} ${2:a\}b} $0");
    assert_eq!(expansion.text, "one,two a}b ");
    assert_eq!(
        expansion
            .placeholders
            .iter()
            .map(|item| (item.tabstop, item.start, item.end))
            .collect::<Vec<_>>(),
        vec![(1, 0, 7), (2, 8, 11)]
    );
    assert_eq!(expansion.cursor_offset, Some(12));
}
