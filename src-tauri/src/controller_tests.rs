use super::*;

#[test]
fn rejects_foreign_urls_and_header_injection() {
    for path in [
        "https://example.com/configs",
        "//example.com/configs",
        "/\\example.com/configs",
        "/configs\r\nHost: example.com",
        "/configs/../restart",
        "/configs#fragment",
    ] {
        assert!(validate_path(path).is_err(), "{path}");
    }
    for path in [
        "/configs?force=true",
        "/proxies/a%2Fb%20%E4%B8%AD%E6%96%87/delay?timeout=1000",
        "/providers/rules/test",
        "/connections",
    ] {
        assert!(validate_path(path).is_ok(), "{path}");
    }
}

#[test]
fn clearing_subscriptions_cancels_idle_receivers() {
    let streams = Streams::default();
    let (tx, mut rx) = tokio::sync::oneshot::channel();
    streams.0.lock().unwrap().insert("old-session".into(), tx);
    streams.stop_all();
    assert!(matches!(
        rx.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Closed)
    ));
}

#[test]
fn tcp_fallback_authenticates_and_removes_subscription_socket_endpoints() {
    use std::io::{BufRead, BufReader, Write};
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let controller =
        Controller::from_endpoint(Endpoint::Tcp(port), "fallback-test-secret").unwrap();
    let mut config = serde_yaml::from_str("external-controller-unix: /tmp/unsafe.sock\nexternal-controller-tls: :9443\nexternal-controller-pipe: unsafe").unwrap();
    controller.configure(&mut config);
    assert_eq!(config.len(), 1);
    assert_eq!(
        config[serde_yaml::Value::String("external-controller".into())].as_str(),
        Some(format!("127.0.0.1:{port}").as_str())
    );
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut reader = BufReader::new(stream);
        let mut authenticated = false;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" {
                break;
            }
            assert!(!line.is_empty());
            authenticated |=
                line.eq_ignore_ascii_case("authorization: Bearer fallback-test-secret\r\n");
        }
        assert!(authenticated);
        write!(
            reader.get_mut(),
            "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
        )
        .unwrap();
    });
    controller
        .request("GET", "/version", None, Duration::from_secs(3))
        .unwrap();
    server.join().unwrap();
}

#[cfg(all(unix, desktop))]
mod unix {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::{fs::PermissionsExt, net::UnixListener};

    fn socket_path(controller: &Controller) -> std::path::PathBuf {
        match &controller.endpoint {
            Endpoint::Unix(dir) => dir.path().join("api.sock"),
            _ => panic!("expected Unix socket"),
        }
    }

    #[test]
    fn private_directory_protects_root_created_socket_and_lives_with_clients() {
        let controller = Controller::new(0, "test").unwrap();
        let path = socket_path(&controller);
        let parent = path.parent().unwrap().to_path_buf();
        assert_eq!(
            std::fs::metadata(&parent).unwrap().permissions().mode() & 0o777,
            0o700
        );
        let listener = UnixListener::bind(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).unwrap();
        assert_eq!(
            std::fs::metadata(&parent).unwrap().permissions().mode() & 0o777,
            0o700
        );
        let clone = controller.clone();
        drop(controller);
        assert!(path.exists());
        drop(listener);
        drop(clone);
        assert!(!parent.exists());
    }

    #[test]
    fn unix_config_removes_every_other_controller_listener() {
        let controller = Controller::new(0, "test").unwrap();
        let mut doc = serde_yaml::from_str("external-controller: 0.0.0.0:9090\nexternal-controller-tls: :9443\nexternal-controller-pipe: unsafe\nexternal-controller-unix: /tmp/unsafe.sock").unwrap();
        controller.configure(&mut doc);
        assert_eq!(doc.len(), 1);
        assert_eq!(
            doc[serde_yaml::Value::String("external-controller-unix".into())].as_str(),
            socket_path(&controller).to_str()
        );
    }

    fn http_fixture(
        controller: &Controller,
        replies: Vec<(&'static str, &'static str)>,
    ) -> std::thread::JoinHandle<Vec<String>> {
        let listener = UnixListener::bind(socket_path(controller)).unwrap();
        listener.set_nonblocking(true).unwrap();
        std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            let mut requests = Vec::new();
            for (status, body) in replies {
                let stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(e)
                            if e.kind() == std::io::ErrorKind::WouldBlock
                                && std::time::Instant::now() < deadline =>
                        {
                            std::thread::sleep(Duration::from_millis(5))
                        }
                        Err(e) => panic!("fixture accept: {e}"),
                    }
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut reader = BufReader::new(stream);
                let mut request = String::new();
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    assert!(!line.is_empty());
                    request.push_str(&line);
                }
                let length = request
                    .lines()
                    .find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("content-length: ")
                            .and_then(|n| n.parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                let mut data = vec![0; length];
                reader.read_exact(&mut data).unwrap();
                request.push_str(&String::from_utf8(data).unwrap());
                requests.push(request);
                write!(reader.get_mut(), "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}", body.len()).unwrap();
            }
            requests
        })
    }

    #[test]
    fn native_requests_preserve_methods_body_encoding_and_actionable_errors() {
        let c = Controller::new(0, "test-secret").unwrap();
        let server = http_fixture(
            &c,
            vec![
                ("200 OK", "{\"ok\":true}"),
                ("204 No Content", ""),
                ("503 Unavailable", "{\"message\":\"not ready\"}"),
                ("200 OK", "bad json"),
            ],
        );
        assert_eq!(
            c.request("GET", "/proxies/a%2Fb%20test", None, Duration::from_secs(2))
                .unwrap()["ok"],
            true
        );
        assert_eq!(
            c.request(
                "PATCH",
                "/configs",
                Some(serde_json::json!({"mode":"global"})),
                Duration::from_secs(2)
            )
            .unwrap(),
            Value::Null
        );
        assert!(c
            .request("GET", "/configs", None, Duration::from_secs(2))
            .unwrap_err()
            .contains("/configs: HTTP 503 — not ready"));
        assert!(c
            .request("GET", "/version", None, Duration::from_secs(2))
            .unwrap_err()
            .contains("invalid controller JSON"));
        let requests = server.join().unwrap();
        assert!(requests[0].starts_with("GET /proxies/a%2Fb%20test HTTP/1.1"));
        assert!(requests[1].starts_with("PATCH /configs HTTP/1.1"));
        assert!(requests[1].ends_with("{\"mode\":\"global\"}"));
    }

    #[test]
    fn readiness_requires_both_valid_http_endpoints() {
        let c = Controller::new(0, "test").unwrap();
        let server = http_fixture(
            &c,
            vec![
                ("200 OK", "{\"version\":\"test\"}"),
                ("503 Unavailable", "{}"),
                ("200 OK", "{\"version\":\"test\"}"),
                ("200 OK", "{\"mode\":\"rule\"}"),
            ],
        );
        assert!(c.ready().is_err());
        c.ready().unwrap();
        assert_eq!(server.join().unwrap().len(), 4);
    }

    #[test]
    fn websocket_frames_and_reconnect_use_the_unix_socket() {
        let controller = Controller::new(0, "test").unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let listener = tokio::net::UnixListener::bind(socket_path(&controller)).unwrap();
            let server = tokio::spawn(async move {
                for _ in 0..2 {
                    let (stream, _) = listener.accept().await.unwrap();
                    let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
                    ws.send(Message::Ping(vec![1, 2].into())).await.unwrap();
                    ws.send(Message::Text("{\"up\":7,\"down\":9}".into()))
                        .await
                        .unwrap();
                    while let Some(Ok(frame)) = ws.next().await {
                        if let Message::Pong(data) = frame {
                            assert_eq!(data.as_ref(), [1, 2]);
                            break;
                        }
                    }
                    ws.close(None).await.unwrap();
                }
            });
            let count = std::cell::Cell::new(0);
            let stream = controller.stream(StreamKind::Traffic, |event| {
                if let StreamEvent::Data { ref data } = event {
                    assert_eq!(data["up"], 7);
                    count.set(count.get() + 1);
                }
                if count.get() >= 2 && matches!(event, StreamEvent::Error { .. }) {
                    return Err("test finished".into());
                }
                Ok(())
            });
            tokio::time::timeout(Duration::from_secs(6), stream)
                .await
                .unwrap();
            server.await.unwrap();
            assert_eq!(count.get(), 2);
        });
    }

    /// Explicit opt-in: launches an isolated core, with no TUN/DNS/proxy listener.
    #[test]
    #[ignore = "set MFORK_TEST_CORE to a trusted local mihomo binary"]
    fn real_mihomo_unix_controller_smoke() {
        let binary = std::env::var_os("MFORK_TEST_CORE").expect("MFORK_TEST_CORE");
        let controller = Controller::new(0, "smoke-only").unwrap();
        let data = tempfile::tempdir().unwrap();
        let config = data.path().join("config.yaml");
        let mut doc = serde_yaml::from_str("mixed-port: 0\nmode: direct\nlog-level: debug\ngeo-auto-update: false\ndns: {enable: false}\ntun: {enable: false}\nproxies: []\nrules: ['MATCH,DIRECT']").unwrap();
        controller.configure(&mut doc);
        std::fs::write(&config, serde_yaml::to_string(&doc).unwrap()).unwrap();
        struct ChildGuard(std::process::Child);
        impl Drop for ChildGuard {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let log = std::fs::File::create(data.path().join("core.log")).unwrap();
        let mut child = ChildGuard(
            std::process::Command::new(binary)
                .arg("-d")
                .arg(data.path())
                .arg("-f")
                .arg(&config)
                .stdout(log.try_clone().unwrap())
                .stderr(log)
                .spawn()
                .unwrap(),
        );
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while let Err(error) = controller.ready() {
            assert!(
                child.0.try_wait().unwrap().is_none() && std::time::Instant::now() < deadline,
                "{error}\n{}",
                std::fs::read_to_string(data.path().join("core.log")).unwrap()
            );
            std::thread::sleep(Duration::from_millis(100));
        }
        let call =
            |method, path, body| controller.request(method, path, body, Duration::from_secs(3));
        let config_state = call("GET", "/configs", None).unwrap();
        assert_eq!(config_state["mixed-port"], 0);
        assert_eq!(config_state["tun"]["enable"], false);
        assert!(call("GET", "/proxies", None).unwrap()["proxies"].is_object());
        call(
            "PATCH",
            "/configs",
            Some(serde_json::json!({"mode":"global"})),
        )
        .unwrap();
        assert_eq!(call("GET", "/configs", None).unwrap()["mode"], "global");
        call(
            "PUT",
            "/configs?force=true",
            Some(serde_json::json!({"path":config})),
        )
        .unwrap();
        assert_eq!(call("GET", "/configs", None).unwrap()["mode"], "direct");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            for kind in [
                StreamKind::Traffic,
                StreamKind::Memory,
                StreamKind::Connections,
                StreamKind::Logs,
            ] {
                let received = std::cell::Cell::new(false);
                let receive = |event| {
                    if matches!(event, StreamEvent::Data { .. })
                        || matches!((kind, event), (StreamKind::Logs, StreamEvent::Connected))
                    {
                        received.set(true);
                        return Err("smoke received event".into());
                    }
                    Ok(())
                };
                let _ = tokio::time::timeout(
                    Duration::from_secs(4),
                    controller.stream_once(kind, &receive),
                )
                .await
                .unwrap();
                assert!(received.get(), "{}", kind.path());
            }
        });
    }
}
