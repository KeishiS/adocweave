use super::*;

#[tokio::test(flavor = "current_thread")]
async fn protocol_async_lsp_transport_runs_typed_lifecycle_and_features() {
    use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

    let (server_stream, client_stream) = tokio::io::duplex(64 * 1024);
    let (server_read, server_write) = tokio::io::split(server_stream);
    let server = run(server_read.compat(), server_write.compat_write());
    let (client_read, mut client_write) = tokio::io::split(client_stream);
    let mut client_read = BufReader::new(client_read);

    let client = async move {
        write_message(
            &mut client_write,
            &json!({
                "jsonrpc":"2.0",
                "id":1,
                "method":"initialize",
                "params":{
                    "processId":null,
                    "rootUri":null,
                    "capabilities":full_capabilities(&["utf-16"])
                }
            }),
        )
        .await;
        let initialize_response = read_message(&mut client_read).await;
        assert_eq!(initialize_response["id"], 1);
        assert_eq!(
            initialize_response["result"]["capabilities"]["workspace"]["workspaceFolders"]["supported"],
            true
        );
        assert_eq!(
            initialize_response["result"]["capabilities"]["codeActionProvider"],
            true
        );
        assert!(
            initialize_response["result"]["capabilities"]["semanticTokensProvider"].is_object()
        );
        write_message(
            &mut client_write,
            &json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
        )
        .await;
        let registration = read_message(&mut client_read).await;
        assert_eq!(registration["method"], "client/registerCapability");
        write_message(
            &mut client_write,
            &json!({"jsonrpc":"2.0","id":registration["id"].clone(),"result":null}),
        )
        .await;
        write_message(
            &mut client_write,
            &json!({
                "jsonrpc":"2.0",
                "method":"textDocument/didOpen",
                "params":{"textDocument":{
                    "uri":"file:///typed.adoc",
                    "languageId":"asciidoc",
                    "version":1,
                    "text":"[[part]]\n= Typed path\n\n<<part>>\ntext  \n"
                }}
            }),
        )
        .await;
        assert_eq!(
            read_message(&mut client_read).await["method"],
            "textDocument/publishDiagnostics"
        );
        write_message(
            &mut client_write,
            &json!({
                "jsonrpc":"2.0",
                "id":3,
                "method":"textDocument/documentSymbol",
                "params":{"textDocument":{"uri":"file:///typed.adoc"}}
            }),
        )
        .await;
        assert_eq!(
            read_message(&mut client_read).await["result"][0]["name"],
            "Typed path"
        );
        write_message(
            &mut client_write,
            &json!({
                "jsonrpc":"2.0",
                "id":10,
                "method":"textDocument/hover",
                "params":{
                    "textDocument":{"uri":"file:///typed.adoc"},
                    "position":{"line":1,"character":3}
                }
            }),
        )
        .await;
        assert_eq!(
            read_message(&mut client_read).await["result"]["contents"]["kind"],
            "markdown"
        );
        write_message(
            &mut client_write,
            &json!({
                "jsonrpc":"2.0",
                "id":11,
                "method":"textDocument/completion",
                "params":{
                    "textDocument":{"uri":"file:///typed.adoc"},
                    "position":{"line":3,"character":3}
                }
            }),
        )
        .await;
        assert_eq!(
            read_message(&mut client_read).await["result"][0]["label"],
            "part"
        );
        write_message(
            &mut client_write,
            &json!({
                "jsonrpc":"2.0",
                "id":12,
                "method":"textDocument/codeAction",
                "params":{
                    "textDocument":{"uri":"file:///typed.adoc"},
                    "range":{
                        "start":{"line":4,"character":0},
                        "end":{"line":4,"character":6}
                    },
                    "context":{"diagnostics":[],"only":["quickfix"]}
                }
            }),
        )
        .await;
        assert!(
            read_message(&mut client_read).await["result"][0]["edit"]["documentChanges"].is_array()
        );
        write_message(
            &mut client_write,
            &json!({
                "jsonrpc":"2.0",
                "id":13,
                "method":"textDocument/formatting",
                "params":{
                    "textDocument":{"uri":"file:///typed.adoc"},
                    "options":{"tabSize":4,"insertSpaces":true}
                }
            }),
        )
        .await;
        assert!(
            read_message(&mut client_read).await["result"]
                .as_array()
                .is_some_and(|edits| !edits.is_empty())
        );
        write_message(
            &mut client_write,
            &json!({
                "jsonrpc":"2.0",
                "id":20,
                "method":"textDocument/prepareRename",
                "params":{
                    "textDocument":{"uri":"file:///typed.adoc"},
                    "position":{"line":0,"character":3}
                }
            }),
        )
        .await;
        let prepared = read_message(&mut client_read).await;
        assert!(prepared["result"]["range"].is_object());
        assert!(prepared["result"]["placeholder"].is_string());
        write_message(
            &mut client_write,
            &json!({
                "jsonrpc":"2.0",
                "id":14,
                "method":"textDocument/rename",
                "params":{
                    "textDocument":{"uri":"file:///typed.adoc"},
                    "position":{"line":0,"character":3},
                    "newName":"renamed"
                }
            }),
        )
        .await;
        assert!(
            read_message(&mut client_read).await["result"]["changes"]["file:///typed.adoc"]
                .is_array()
        );
        write_message(
            &mut client_write,
            &json!({
                "jsonrpc":"2.0",
                "id":6,
                "method":"textDocument/definition",
                "params":{
                    "textDocument":{"uri":"file:///typed.adoc"},
                    "position":{"line":3,"character":3}
                }
            }),
        )
        .await;
        assert_eq!(
            read_message(&mut client_read).await["result"]["uri"],
            "file:///typed.adoc"
        );
        write_message(
            &mut client_write,
            &json!({
                "jsonrpc":"2.0",
                "id":7,
                "method":"textDocument/semanticTokens/full",
                "params":{"textDocument":{"uri":"file:///typed.adoc"}}
            }),
        )
        .await;
        assert!(
            read_message(&mut client_read).await["result"]["data"]
                .as_array()
                .is_some_and(|data| !data.is_empty())
        );
        write_message(
            &mut client_write,
            &json!({
                "jsonrpc":"2.0",
                "id":8,
                "method":"textDocument/documentLink",
                "params":{"textDocument":{"uri":"file:///typed.adoc"}}
            }),
        )
        .await;
        assert_eq!(
            read_message(&mut client_read).await["result"][0]["target"],
            "file:///typed.adoc#part"
        );
        write_message(
            &mut client_write,
            &json!({
                "jsonrpc":"2.0",
                "id":9,
                "method":"textDocument/references",
                "params":{
                    "textDocument":{"uri":"file:///typed.adoc"},
                    "position":{"line":0,"character":3},
                    "context":{"includeDeclaration":false}
                }
            }),
        )
        .await;
        assert_eq!(
            read_message(&mut client_read).await["result"]
                .as_array()
                .map(Vec::len),
            Some(1)
        );
        write_message(
            &mut client_write,
            &json!({"jsonrpc":"2.0","id":2,"method":"shutdown","params":null}),
        )
        .await;
        assert_eq!(read_message(&mut client_read).await["id"], 2);
        write_message(
            &mut client_write,
            &json!({"jsonrpc":"2.0","method":"exit","params":null}),
        )
        .await;
    };

    let (server_result, ()) = tokio::join!(server, client);
    server_result.expect("clean exit");
}

#[tokio::test(flavor = "current_thread")]
async fn protocol_async_lsp_lifecycle_rejects_requests_in_invalid_states() {
    use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

    let (server_stream, client_stream) = tokio::io::duplex(64 * 1024);
    let (server_read, server_write) = tokio::io::split(server_stream);
    let server = run(server_read.compat(), server_write.compat_write());
    let (client_read, mut client_write) = tokio::io::split(client_stream);
    let mut client_read = BufReader::new(client_read);

    let client = async move {
        write_message(
            &mut client_write,
            &json!({
                "jsonrpc":"2.0",
                "method":"textDocument/didOpen",
                "params":{"textDocument":{
                    "uri":"file:///lifecycle.adoc",
                    "languageId":"asciidoc",
                    "version":1,
                    "text":"= Must be dropped\n"
                }}
            }),
        )
        .await;
        write_message(
            &mut client_write,
            &json!({
                "jsonrpc":"2.0",
                "id":1,
                "method":"textDocument/documentSymbol",
                "params":{"textDocument":{"uri":"file:///lifecycle.adoc"}}
            }),
        )
        .await;
        assert_eq!(
            read_message(&mut client_read).await["error"]["code"],
            -32002
        );

        let initialize = json!({
            "jsonrpc":"2.0",
            "id":2,
            "method":"initialize",
            "params":{"processId":null,"rootUri":null,"capabilities":{}}
        });
        write_message(&mut client_write, &initialize).await;
        let initialize_response = read_message(&mut client_read).await;
        assert_eq!(initialize_response["id"], 2);
        let capabilities = &initialize_response["result"]["capabilities"];
        assert!(capabilities.get("codeActionProvider").is_none());
        assert!(capabilities.get("semanticTokensProvider").is_none());
        assert!(capabilities.get("workspace").is_none());

        let mut duplicate = initialize;
        duplicate["id"] = json!(3);
        write_message(
            &mut client_write,
            &json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
        )
        .await;
        write_message(
            &mut client_write,
            &json!({
                "jsonrpc":"2.0",
                "id":6,
                "method":"textDocument/documentSymbol",
                "params":{"textDocument":{"uri":"file:///lifecycle.adoc"}}
            }),
        )
        .await;
        assert_eq!(read_message(&mut client_read).await["result"], json!([]));
        write_message(&mut client_write, &duplicate).await;
        assert_eq!(
            read_message(&mut client_read).await["error"]["code"],
            -32600
        );

        write_message(
            &mut client_write,
            &json!({"jsonrpc":"2.0","id":4,"method":"shutdown","params":null}),
        )
        .await;
        assert_eq!(read_message(&mut client_read).await["id"], 4);

        write_message(
            &mut client_write,
            &json!({
                "jsonrpc":"2.0",
                "method":"textDocument/didOpen",
                "params":{"textDocument":{
                    "uri":"file:///after-shutdown.adoc",
                    "languageId":"asciidoc",
                    "version":1,
                    "text":"= Must also be dropped\n"
                }}
            }),
        )
        .await;
        write_message(
            &mut client_write,
            &json!({
                "jsonrpc":"2.0",
                "id":5,
                "method":"textDocument/documentSymbol",
                "params":{"textDocument":{"uri":"file:///lifecycle.adoc"}}
            }),
        )
        .await;
        assert_eq!(
            read_message(&mut client_read).await["error"]["code"],
            -32600
        );
        write_message(
            &mut client_write,
            &json!({"jsonrpc":"2.0","method":"exit","params":null}),
        )
        .await;
    };

    let (server_result, ()) = tokio::join!(server, client);
    server_result.expect("clean exit");
}

#[tokio::test(flavor = "current_thread")]
async fn protocol_registers_all_extensions_for_include_recovery_and_survives_rejection() {
    use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

    let (server_stream, client_stream) = tokio::io::duplex(64 * 1024);
    let (server_read, server_write) = tokio::io::split(server_stream);
    let server = run(server_read.compat(), server_write.compat_write());
    let (client_read, mut client_write) = tokio::io::split(client_stream);
    let mut client_read = BufReader::new(client_read);

    let client = async move {
        write_message(
            &mut client_write,
            &json!({
                "jsonrpc":"2.0",
                "id":1,
                "method":"initialize",
                "params":{
                    "processId":null,
                    "rootUri":null,
                    "workspaceFolders":[],
                    "capabilities":{
                        "workspace":{
                            "workspaceFolders":true,
                            "didChangeWatchedFiles":{"dynamicRegistration":true}
                        }
                    }
                }
            }),
        )
        .await;
        let initialized = read_message(&mut client_read).await;
        assert_eq!(initialized["id"], 1);
        assert_eq!(
            initialized["result"]["capabilities"]["workspace"]["workspaceFolders"]["supported"],
            true
        );
        write_message(
            &mut client_write,
            &json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
        )
        .await;

        let registration = read_message(&mut client_read).await;
        assert_eq!(registration["method"], "client/registerCapability");
        assert_eq!(
            registration["params"]["registrations"][0]["method"],
            "workspace/didChangeWatchedFiles"
        );
        assert_eq!(
            registration["params"]["registrations"][0]["registerOptions"]["watchers"][0]["globPattern"],
            "**/*"
        );
        assert_eq!(
            registration["params"]["registrations"][0]["registerOptions"]["watchers"]
                .as_array()
                .expect("watchers")
                .len(),
            1,
            "one all-extension watcher must cover missing non-adoc include creation"
        );
        assert_eq!(
            registration["params"]["registrations"][0]["registerOptions"]["watchers"][0]["kind"],
            7
        );
        write_message(
            &mut client_write,
            &json!({
                "jsonrpc":"2.0",
                "id":registration["id"].clone(),
                "error":{"code":-32603,"message":"registration rejected for test"}
            }),
        )
        .await;

        write_message(
            &mut client_write,
            &json!({"jsonrpc":"2.0","id":2,"method":"shutdown","params":null}),
        )
        .await;
        assert_eq!(read_message(&mut client_read).await["id"], 2);
        write_message(
            &mut client_write,
            &json!({"jsonrpc":"2.0","method":"exit","params":null}),
        )
        .await;
    };

    let (server_result, ()) = tokio::join!(server, client);
    server_result.expect("clean exit");
}

#[tokio::test(flavor = "current_thread")]
async fn protocol_exit_without_shutdown_is_an_error() {
    use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

    let (server_stream, mut client_stream) = tokio::io::duplex(4096);
    let (server_read, server_write) = tokio::io::split(server_stream);
    let server = run(server_read.compat(), server_write.compat_write());
    let client = async move {
        write_message(
            &mut client_stream,
            &json!({"jsonrpc":"2.0","method":"exit","params":null}),
        )
        .await;
    };

    let (server_result, ()) = tokio::join!(server, client);
    assert!(server_result.is_err());
}

#[tokio::test(flavor = "current_thread")]
async fn protocol_preserves_json_rpc_ids_errors_and_notification_silence() {
    use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

    let (server_stream, client_stream) = tokio::io::duplex(16 * 1024);
    let (server_read, server_write) = tokio::io::split(server_stream);
    let server = run(server_read.compat(), server_write.compat_write());
    let (client_read, mut client_write) = tokio::io::split(client_stream);
    let mut client_read = BufReader::new(client_read);

    let client = async move {
        write_message(
            &mut client_write,
            &json!({
                "jsonrpc":"2.0",
                "id":"initialize-string-id",
                "method":"initialize",
                "params":{"processId":null,"rootUri":null,"capabilities":{}}
            }),
        )
        .await;
        assert_eq!(
            read_message(&mut client_read).await["id"],
            "initialize-string-id"
        );
        write_message(
            &mut client_write,
            &json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
        )
        .await;

        write_message(
            &mut client_write,
            &json!({
                "jsonrpc":"2.0",
                "id":"unknown-method",
                "method":"adocweave/unknown",
                "params":{}
            }),
        )
        .await;
        let unknown = read_message(&mut client_read).await;
        assert_eq!(unknown["id"], "unknown-method");
        assert_eq!(unknown["error"]["code"], -32601);
        assert!(unknown.get("result").is_none());

        write_message(
            &mut client_write,
            &json!({
                "jsonrpc":"2.0",
                "id":"invalid-params",
                "method":"textDocument/hover",
                "params":{"textDocument":{"uri":"file:///json-rpc.adoc"}}
            }),
        )
        .await;
        let invalid = read_message(&mut client_read).await;
        assert_eq!(invalid["id"], "invalid-params");
        assert_eq!(invalid["error"]["code"], -32602);
        assert!(invalid.get("result").is_none());

        write_message(
            &mut client_write,
            &json!({"jsonrpc":"2.0","method":"$/futureNotification","params":{}}),
        )
        .await;
        write_message(
            &mut client_write,
            &json!({
                "jsonrpc":"2.0",
                "id":"shutdown-after-notification",
                "method":"shutdown",
                "params":null
            }),
        )
        .await;
        let shutdown = read_message(&mut client_read).await;
        assert_eq!(shutdown["id"], "shutdown-after-notification");
        assert_eq!(shutdown["result"], Value::Null);
        assert!(shutdown.get("error").is_none());
        write_message(
            &mut client_write,
            &json!({"jsonrpc":"2.0","method":"exit","params":null}),
        )
        .await;
    };

    let (server_result, ()) = tokio::join!(server, client);
    server_result.expect("clean exit");
}

#[tokio::test(flavor = "current_thread")]
async fn protocol_reports_an_incomplete_scan_once_without_document_diagnostics() {
    use std::time::Duration;

    use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("adocweave-protocol-scan-notice-{unique}"));
    fs::create_dir_all(&root).expect("workspace");
    let config_path = root.join(adocweave_config::FILE_NAME);
    fs::write(
        &config_path,
        "schema-version = 2\n[resources]\nmax-files = 1\n",
    )
    .expect("configuration");
    for name in ["a.adoc", "b.adoc", "c.adoc"] {
        fs::write(root.join(name), "text\n").expect("document");
    }
    let root_uri = lsp::Url::from_directory_path(&root).expect("root URI");
    let document_uri = lsp::Url::from_file_path(root.join("a.adoc")).expect("document URI");
    let config_uri = lsp::Url::from_file_path(&config_path).expect("configuration URI");

    let (server_stream, client_stream) = tokio::io::duplex(64 * 1024);
    let (server_read, server_write) = tokio::io::split(server_stream);
    let server = run(server_read.compat(), server_write.compat_write());
    let (client_read, mut client_write) = tokio::io::split(client_stream);
    let mut client_read = BufReader::new(client_read);

    let client = async move {
        write_message(
            &mut client_write,
            &json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "processId": null,
                    "rootUri": root_uri,
                    "capabilities": {
                        "workspace": {
                            "didChangeWatchedFiles": {"dynamicRegistration": true}
                        }
                    }
                }
            }),
        )
        .await;
        assert_eq!(read_message(&mut client_read).await["id"], 1);
        write_message(
            &mut client_write,
            &json!({"jsonrpc": "2.0", "method": "initialized", "params": {}}),
        )
        .await;

        let mut registered = false;
        let mut first_notice = None;
        while !registered || first_notice.is_none() {
            let message = read_message(&mut client_read).await;
            match message["method"].as_str() {
                Some("client/registerCapability") => {
                    write_message(
                        &mut client_write,
                        &json!({
                            "jsonrpc": "2.0",
                            "id": message["id"].clone(),
                            "result": null
                        }),
                    )
                    .await;
                    registered = true;
                }
                Some("window/showMessage") => {
                    assert!(
                        first_notice.is_none(),
                        "scan notice was sent more than once"
                    );
                    first_notice = Some(typed::<lsp::ShowMessageParams>(message["params"].clone()));
                }
                method => panic!("unexpected server message before scan completion: {method:?}"),
            }
        }
        let first_notice = first_notice.expect("scan notice");
        assert_eq!(first_notice.typ, lsp::MessageType::WARNING);
        assert!(first_notice.message.contains("resources.max-files"));
        assert!(
            first_notice
                .message
                .contains(config_path.to_string_lossy().as_ref())
        );

        write_message(
            &mut client_write,
            &json!({
                "jsonrpc": "2.0",
                "method": "textDocument/didOpen",
                "params": {"textDocument": {
                    "uri": document_uri,
                    "languageId": "asciidoc",
                    "version": 1,
                    "text": "text\n"
                }}
            }),
        )
        .await;
        let initial_diagnostics = loop {
            let message = read_message(&mut client_read).await;
            assert_ne!(message["method"], "window/showMessage");
            if message["method"] == "textDocument/publishDiagnostics" {
                let params = typed::<lsp::PublishDiagnosticsParams>(message["params"].clone());
                if params.uri == document_uri {
                    break params;
                }
            }
        };
        assert!(initial_diagnostics.diagnostics.iter().all(|diagnostic| {
            diagnostic.code
                != Some(lsp::NumberOrString::String(
                    "workspace-scan-incomplete".to_owned(),
                ))
                && !diagnostic.message.contains("resources.max-files")
        }));

        write_message(
            &mut client_write,
            &json!({
                "jsonrpc": "2.0",
                "method": "workspace/didChangeWatchedFiles",
                "params": {"changes": [{"uri": config_uri, "type": 2}]}
            }),
        )
        .await;
        loop {
            let message = read_message(&mut client_read).await;
            assert_ne!(message["method"], "window/showMessage");
            if message["method"] == "textDocument/publishDiagnostics" {
                let params = typed::<lsp::PublishDiagnosticsParams>(message["params"].clone());
                if params.uri == document_uri {
                    break;
                }
            }
        }

        write_message(
            &mut client_write,
            &json!({"jsonrpc": "2.0", "id": 2, "method": "shutdown", "params": null}),
        )
        .await;
        assert_eq!(read_message(&mut client_read).await["id"], 2);
        write_message(
            &mut client_write,
            &json!({"jsonrpc": "2.0", "method": "exit", "params": null}),
        )
        .await;
    };

    let (server_result, ()) = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(server, client)
    })
    .await
    .expect("protocol timeout");
    server_result.expect("clean exit");
    fs::remove_dir_all(root).expect("cleanup");
}

#[cfg(unix)]
#[tokio::test(flavor = "current_thread")]
async fn protocol_stops_when_the_declared_client_process_does_not_exist() {
    use std::time::Duration;

    use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

    let (server_stream, mut client_stream) = tokio::io::duplex(4096);
    let (server_read, server_write) = tokio::io::split(server_stream);
    let server = run(server_read.compat(), server_write.compat_write());
    write_message(
        &mut client_stream,
        &json!({
            "jsonrpc":"2.0",
            "id":1,
            "method":"initialize",
            "params":{"processId":i32::MAX,"rootUri":null,"capabilities":{}}
        }),
    )
    .await;

    let result = tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .expect("client process monitor timeout");
    assert!(result.is_err());
}
