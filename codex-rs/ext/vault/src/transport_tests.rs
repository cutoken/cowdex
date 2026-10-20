use super::*;
use pretty_assertions::assert_eq;
use serde_json::json;
use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncWriteExt;

#[tokio::test]
async fn loopback_profile_exchanges_scoped_broker_messages() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("grant.json");
    let address = format!("tcp://{}", listener.local_addr().unwrap());
    tokio::fs::write(
        &path,
        serde_json::to_vec(&json!({"socket":address,"ticket":"ticket","scope":["cf"]})).unwrap(),
    )
    .await
    .unwrap();
    let profile = read_profile(path).await.unwrap();
    let request = json!({"op":"attach","ticket":profile.ticket,"scope":profile.scope});
    let expected = json!({"session":"session","aliases":["cf"],"expires_at":2000000000});
    let reply = expected.clone();
    let sent = request.clone();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut stream = tokio::io::BufReader::new(stream);
        let mut line = String::new();
        stream.read_line(&mut line).await.unwrap();
        assert_eq!(serde_json::from_str::<Value>(&line).unwrap(), sent);
        stream
            .get_mut()
            .write_all(format!("{reply}\n").as_bytes())
            .await
            .unwrap();
    });
    assert_eq!(exchange(&profile.socket, request).await.unwrap(), expected);
    server.await.unwrap();
}

#[tokio::test]
async fn tcp_profile_rejects_nonlocal_addresses() {
    for socket in [
        "tcp://192.0.2.1:2749",
        "tcp://localhost:2749",
        "tcp://127.0.0.1:0",
    ] {
        assert!(
            exchange(Path::new(socket), json!({"op":"status"}))
                .await
                .is_err()
        );
    }
}
